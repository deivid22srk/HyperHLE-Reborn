/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! 64-bit process setup and execution (F1, PROGRESS.md P3): everything
//! between "the image is linked" and "guest code is running".
//!
//! Three pieces, mirroring what XNU + dyld do for an LC_MAIN arm64
//! executable (clean-room, from public dyld/XNU behaviour and validated
//! against the real SubwaySurf stub — see PROGRESS.md):
//!
//! * [`prep_stack_for_main`] lays out the kernel-provided process stack:
//!   the `argc`/`argv`/`envp`/`apple` arrays plus a minimal auxv, and
//!   returns the initial SP. Like dyld's `__dyld_start`, the entry point
//!   gets the arguments **in registers** (`x0`=argc, `x1`=argv, `x2`=envp,
//!   `x3`=apple) — confirmed by disassembling the real stub, whose entry
//!   immediately forwards `x0`/`x1` into its first calls. The stack copy is
//!   still laid out because libSystem code reads it (and `__progname`
//!   derives from `argv[0]`).
//! * [`run_a64_image`] executes initializers then the entry point,
//!   dispatching `svc #0x81` traps to host HLE functions through the
//!   [`dyld64::TrampolinePage`], and `svc #0x80` to a minimal BSD syscall
//!   layer ([`DarwinSyscalls`]) using the Apple arm64 ABI: the syscall
//!   number is in `x16`, results go to `x0`, and **errors set the carry
//!   flag** in NZCV with `x0` = errno (XNU arm64 convention; libc wrappers
//!   branch on `b.cs`).
//! * [`dump_missing_hle`] — the `--dump-missing-hle64` CLI command: links
//!   the real stub with a [`dyld64::TrapResolver`], runs it, and reports
//!   every HLE symbol that was actually called but has no host
//!   implementation yet. This is the F2 work list, measured rather than
//!   guessed.
//!
//! Nothing here touches the 32-bit `Environment` machinery; F2 grows real
//! host functions behind [`A64HleHost`] and eventually feeds the same
//! `frameworks`/`libc` trees as the A32 side.

use std::path::Path;

use crate::cpu::a64::{Cpu, CpuState, Mem64, VAddr};
use crate::dyld64::{self, encode_svc, LoadedImage, SVC_HLE};

/// SVC immediate that means "BSD syscall" on A64 (mirrors the A32 side).
/// The syscall number itself travels in `x16` (Apple arm64 ABI).
pub const SVC_SYSCALL: u32 = 0x80;

/// Internal (host-only) syscall number meaning "the guest returned to a
/// constructor sentinel". Never issued by guest code: the sentinel page
/// materialises it right after a constructor returns.
pub const SVC_INTERNAL_RETURNED: u32 = 0xFFFE;

/// Top of the A64 main-thread stack. The layout grows downwards from here.
/// Nothing above it is mapped, so an upwards stack walk faults (the same
/// guard policy as the A32 `MAIN_THREAD_STACK_HIGH_END`).
pub const MAIN_STACK_TOP: VAddr = 0x0000_7ffe_0000_0000;
/// Mapped stack size (1 MiB is ample for the stub's startup; F2 will raise
/// it for the player). Overflow past the low end faults like a real guard
/// page.
pub const MAIN_STACK_SIZE: u64 = 0x0010_0000;

/// Base of the constructor sentinel page: one `movz x16, #0xFFFE` +
/// `svc #0x80` pair per initializer, so the runner can tell "constructor
/// returned" apart from guest syscalls without relying on register state
/// surviving the return.
pub const CONSTRUCTOR_SENTINEL_BASE: VAddr = 0x0000_7100_0000_0000;

/// A small zeroed scratch page exposed via TPIDRRO_EL0, for guests that
/// read thread state before the F2 TLS work exists.
const TLS_SCRATCH_BASE: VAddr = 0x0000_7200_0000_0000;

/// NZCV carry bit: set on syscall error (XNU arm64 convention).
const NZCV_CARRY: u32 = 1 << 29;

// Darwin auxv keys (System V numbering as pushed by XNU's mach_loader).
const AT_NULL: u64 = 0;
const AT_PAGESZ: u64 = 6;
const AT_ENTRY: u64 = 9;
const AT_UID: u64 = 11;
const AT_EUID: u64 = 12;
const AT_GID: u64 = 13;
const AT_EGID: u64 = 14;

/// Darwin errno values used by the minimal syscall layer.
const EBADF: u64 = 9;
const EFAULT: u64 = 14;
const ENOSYS: u64 = 78;

/// The initial process arguments, as the kernel would provide them.
pub struct ProcessArgs<'a> {
    pub argv: &'a [&'a str],
    pub envp: &'a [&'a str],
    /// The `apple` key/values dyld passes to `main` (may be empty).
    pub apple: &'a [&'a str],
}

/// Lay out the process stack for the main thread and return the initial SP.
///
/// The final layout, growing upwards from the returned SP (16-byte aligned),
/// is the XNU mach_loader arrangement:
///
/// ```text
/// sp+0                          argc (u64)
/// sp+8                          argv[0..argc], NULL
///                               envp[0..], NULL
///                               apple[0..], NULL
///                               auxv key/value pairs ..., AT_NULL, 0
/// string_base .. stack_top      argv/envp/apple strings, NUL-terminated
/// ```
pub fn prep_stack_for_main(
    mem: &mut Mem64,
    cpu: &mut Cpu,
    top: VAddr,
    args: &ProcessArgs,
    entry: VAddr,
) -> VAddr {
    // String pool: every argv/envp/apple entry plus its NUL.
    let mut strings: Vec<(&str, VAddr)> = Vec::new();
    let string_bytes: u64 = args
        .argv
        .iter()
        .chain(args.envp)
        .chain(args.apple)
        .map(|s| s.len() as u64 + 1)
        .sum();

    let argc = args.argv.len() as u64;
    let ptr_slots = (argc + 1) + (args.envp.len() as u64 + 1) + (args.apple.len() as u64 + 1);
    let auxv: Vec<(u64, u64)> = vec![
        (AT_PAGESZ, 4096),
        (AT_ENTRY, entry),
        (AT_UID, 501),  // the `mobile` user on iOS
        (AT_EUID, 501),
        (AT_GID, 501),
        (AT_EGID, 501),
        (AT_NULL, 0),
    ];
    let auxv_slots = (auxv.len() * 2) as u64;

    let total = 8 /* argc */ + ptr_slots * 8 + auxv_slots * 8 + string_bytes;
    assert!(
        total <= MAIN_STACK_SIZE,
        "process arguments exceed the mapped stack"
    );
    let sp = (top - total) & !0xF;

    // Addresses (going up from sp):
    let argc_addr = sp;
    let argv_addr = argc_addr + 8;
    let envp_addr = argv_addr + (argc + 1) * 8;
    let apple_addr = envp_addr + (args.envp.len() as u64 + 1) * 8;
    let auxv_addr = apple_addr + (args.apple.len() as u64 + 1) * 8;
    let string_base = auxv_addr + auxv_slots * 8;

    // Pointer arrays + argc + auxv. The caller normally has the stack
    // mapped already; this keeps the function self-contained.
    mem.map_range(argc_addr, string_base + string_bytes - argc_addr);
    mem.write_u64(argc_addr, argc);
    let mut cursor = string_base;
    for (i, s) in args.argv.iter().enumerate() {
        mem.write_u64(argv_addr + i as u64 * 8, cursor);
        strings.push((s, cursor));
        cursor += s.len() as u64 + 1;
    }
    mem.write_u64(argv_addr + argc * 8, 0);
    for (i, s) in args.envp.iter().enumerate() {
        mem.write_u64(envp_addr + i as u64 * 8, cursor);
        strings.push((s, cursor));
        cursor += s.len() as u64 + 1;
    }
    mem.write_u64(envp_addr + args.envp.len() as u64 * 8, 0);
    for (i, s) in args.apple.iter().enumerate() {
        mem.write_u64(apple_addr + i as u64 * 8, cursor);
        strings.push((s, cursor));
        cursor += s.len() as u64 + 1;
    }
    mem.write_u64(apple_addr + args.apple.len() as u64 * 8, 0);
    for (i, (k, v)) in auxv.iter().enumerate() {
        mem.write_u64(auxv_addr + i as u64 * 16, *k);
        mem.write_u64(auxv_addr + i as u64 * 16 + 8, *v);
    }

    // String pool.
    for (s, addr) in &strings {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        mem.write_bytes(*addr, &bytes);
    }

    // The LC_MAIN handoff (dyld's __dyld_start): arguments in registers.
    cpu.set_sp(sp);
    cpu.set_reg(0, argc);
    cpu.set_reg(1, argv_addr);
    cpu.set_reg(2, envp_addr);
    cpu.set_reg(3, apple_addr);
    sp
}

/// A host implementation for `svc #0x81` HLE calls.
///
/// `Some(value)` means a real host function ran and the guest gets `value`
/// in `x0`. `None` means no implementation exists: the runner treats the
/// call as **missing** (recorded by the host itself) and the guest still
/// gets 0, so the run can continue and surface more of the work list.
pub trait A64HleHost {
    fn call(&mut self, symbol: &str, cpu: &mut Cpu, mem: &mut Mem64) -> Option<u64>;

    /// Symbols called without an implementation, first-call order, with
    /// call counts. The default is "none"; recording hosts override this.
    fn missing_report(&self) -> Vec<(String, u32)> {
        Vec::new()
    }
}

/// The P3 host: records every called symbol and returns 0.
///
/// This is the measurement instrument for F2, not an implementation: each
/// recorded symbol needs a real host function before the player can boot.
#[derive(Debug, Default)]
pub struct RecordingHleHost {
    /// (symbol, call count), in first-call order.
    pub missing: Vec<(String, u32)>,
}

impl RecordingHleHost {
    fn record(&mut self, symbol: &str) {
        if let Some(entry) = self.missing.iter_mut().find(|(s, _)| s == symbol) {
            entry.1 += 1;
        } else {
            self.missing.push((symbol.to_string(), 1));
        }
    }
}

impl A64HleHost for RecordingHleHost {
    fn call(&mut self, symbol: &str, _cpu: &mut Cpu, _mem: &mut Mem64) -> Option<u64> {
        self.record(symbol);
        Some(0)
    }

    fn missing_report(&self) -> Vec<(String, u32)> {
        self.missing.clone()
    }
}

/// Minimal BSD syscall layer for `svc #0x80` (Apple arm64 ABI).
///
/// Real implementations where P3 needs them (exit, write, getpid);
/// everything else fails with `ENOSYS` — which is the honest outcome, not a
/// silent success. Errors follow the XNU arm64 convention: `x0` = errno and
/// the NZCV carry flag set, so guest libc wrappers take their `b.cs` error
/// path.
#[derive(Debug, Default)]
pub struct DarwinSyscalls {
    /// (syscall number, call count), first-call order.
    pub calls: Vec<(u32, u32)>,
}

impl DarwinSyscalls {
    fn record(&mut self, nr: u32) {
        if let Some(entry) = self.calls.iter_mut().find(|(n, _)| *n == nr) {
            entry.1 += 1;
        } else {
            self.calls.push((nr, 1));
        }
    }

    fn fail(cpu: &mut Cpu, errno: u64) {
        cpu.set_reg(0, errno);
        cpu.set_pstate(NZCV_CARRY);
    }

    fn succeed(cpu: &mut Cpu, value: u64) {
        cpu.set_reg(0, value);
        // Success clears the carry flag; the other NZCV bits are undefined
        // after a syscall, per the ABI.
        cpu.set_pstate(0);
    }

    /// Dispatch the syscall in `x16`. Returns `Some(exit_code)` when the
    /// guest asked to terminate.
    pub fn dispatch(&mut self, cpu: &mut Cpu, mem: &mut Mem64) -> Option<i32> {
        let nr = cpu.get_reg(16) as u32;
        self.record(nr);
        match nr {
            1 => {
                // exit
                Some(cpu.get_reg(0) as i32)
            }
            4 => {
                // write(fd, buf, len)
                let fd = cpu.get_reg(0);
                let buf = cpu.get_reg(1);
                let len = cpu.get_reg(2);
                if len == 0 {
                    Self::succeed(cpu, 0);
                    return None;
                }
                if len > 0x1000_0000 {
                    Self::fail(cpu, EFAULT);
                    return None;
                }
                let bytes = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    mem.read_bytes(buf, len as usize)
                }));
                match bytes {
                    Err(_) => Self::fail(cpu, EFAULT),
                    Ok(bytes) => {
                        if fd == 1 || fd == 2 {
                            log_no_panic!("A64 guest write(fd={}): {:?}", fd, bytes);
                            Self::succeed(cpu, len);
                        } else {
                            Self::fail(cpu, EBADF);
                        }
                    }
                }
                None
            }
            20 => {
                // getpid
                Self::succeed(cpu, 4242);
                None
            }
            _ => {
                log_no_panic!("A64 guest called unimplemented syscall {}", nr);
                Self::fail(cpu, ENOSYS);
                None
            }
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum A64Outcome {
    /// The guest called `exit(code)`.
    Exit(i32),
    /// Guest memory access to unmapped memory.
    MemoryFault { pc: VAddr, lr: VAddr },
    /// Guest executed an undefined instruction.
    UndefinedInstruction { pc: VAddr },
    /// Guest hit a breakpoint (BRK).
    Breakpoint { pc: VAddr },
    /// The instruction budget ran out without terminating.
    TickBudget,
}

/// Everything one image run produced.
#[derive(Debug, Default)]
pub struct A64RunReport {
    /// Initializers that ran to completion before the entry point.
    pub constructors_ran: usize,
    /// Whether the entry point was reached (false if an initializer
    /// terminated the run first).
    pub entry_reached: bool,
    pub outcome: Option<A64Outcome>,
    /// Syscalls made (`nr`, count), first-call order.
    pub syscalls: Vec<(u32, u32)>,
    /// HLE symbols called without an implementation (`symbol`, count).
    pub missing: Vec<(String, u32)>,
}

/// What the shared run loop should stop for, inside one guest phase.
enum PhaseResult {
    /// The phase finished by returning to its sentinel.
    Returned,
    Terminal(A64Outcome),
}

/// Was the CPU halted just after executing the sentinel pair at
/// `sentinel`? (The wrapper's reported PC is already past the SVC, but
/// `symbol_for_dispatch` also tolerates the non-advanced case.)
fn is_at_sentinel(pc: VAddr, sentinel: VAddr) -> bool {
    pc == sentinel + 8 || pc == sentinel + 4
}

/// Run one guest phase (a constructor or the entry point) until it returns
/// to `sentinel`, terminates, or exhausts `budget`.
///
/// Every `svc #0x81` is dispatched through `trampolines`/`host`; every
/// `svc #0x80` goes through `syscalls`, except the internal
/// [`SVC_INTERNAL_RETURNED`] sentinel value, which ends the phase.
fn run_phase(
    cpu: &mut Cpu,
    mem: &mut Mem64,
    trampolines: &dyld64::TrampolinePage,
    host: &mut dyn A64HleHost,
    syscalls: &mut DarwinSyscalls,
    sentinel: VAddr,
    budget: u64,
) -> PhaseResult {
    let mut used: u64 = 0;
    loop {
        let slice = budget.saturating_sub(used).min(1_000_000);
        if slice == 0 {
            return PhaseResult::Terminal(A64Outcome::TickBudget);
        }
        let mut ticks = slice;
        let before = ticks;
        match cpu.run_or_step(mem, Some(&mut ticks)) {
            CpuState::Normal => {
                used += before - ticks;
                if used >= budget {
                    return PhaseResult::Terminal(A64Outcome::TickBudget);
                }
            }
            CpuState::Svc(svc) => {
                used += before - ticks;
                if svc == SVC_SYSCALL {
                    let nr = cpu.get_reg(16) as u32;
                    if nr == SVC_INTERNAL_RETURNED
                        && sentinel != VAddr::MAX
                        && is_at_sentinel(cpu.pc(), sentinel)
                    {
                        return PhaseResult::Returned;
                    }
                    if let Some(code) = syscalls.dispatch(cpu, mem) {
                        return PhaseResult::Terminal(A64Outcome::Exit(code));
                    }
                } else if svc == SVC_HLE {
                    let pc = cpu.pc();
                    let symbol = match trampolines.symbol_for_dispatch(pc) {
                        Some(symbol) => symbol,
                        None => panic!("SVC_HLE at {pc:#x} is not a known trampoline"),
                    };
                    let symbol = symbol.to_string();
                    let rv = host.call(&symbol, cpu, mem).unwrap_or(0);
                    cpu.set_reg(0, rv);
                    // Return to the guest caller (the trampoline was entered
                    // with BL/BLR, so LR holds the resume address).
                    cpu.set_pc(cpu.get_reg(30));
                } else {
                    panic!("unexpected SVC #{svc:#x}");
                }
            }
            CpuState::Error(crate::cpu::a64::CpuError::MemoryError) => {
                return PhaseResult::Terminal(A64Outcome::MemoryFault {
                    pc: cpu.pc(),
                    lr: cpu.get_reg(30),
                });
            }
            CpuState::Error(crate::cpu::a64::CpuError::UndefinedInstruction) => {
                return PhaseResult::Terminal(A64Outcome::UndefinedInstruction {
                    pc: cpu.last_exception_pc(),
                });
            }
            CpuState::Error(crate::cpu::a64::CpuError::Breakpoint) => {
                return PhaseResult::Terminal(A64Outcome::Breakpoint {
                    pc: cpu.last_exception_pc(),
                });
            }
        }
    }
}

/// Run a linked A64 image: initializers first, then the entry point, with
/// full HLE/syscall dispatch. `cpu` should be fresh; the stack, registers
/// and TLS scratch are set up here.
///
/// `budget` is the total instruction budget across all phases (the P3
/// dump uses 50M — the stub's startup is a few hundred thousand).
pub fn run_a64_image(
    mem: &mut Mem64,
    cpu: &mut Cpu,
    loaded: &LoadedImage,
    trampolines: &dyld64::TrampolinePage,
    host: &mut dyn A64HleHost,
    args: &ProcessArgs,
    budget: u64,
) -> A64RunReport {
    let mut report = A64RunReport::default();
    let entry = loaded.entry.expect("run_a64_image needs an executable");
    let syscalls = &mut DarwinSyscalls::default();

    // Stack + guard: map below the top, leave everything above it unmapped.
    mem.map_range(MAIN_STACK_TOP - MAIN_STACK_SIZE, MAIN_STACK_SIZE);
    let sp = prep_stack_for_main(mem, cpu, MAIN_STACK_TOP, args, entry);

    // TLS scratch via TPIDRRO_EL0 (must be re-set on every thread switch in
    // F2; here there is only the main thread).
    mem.map_range(TLS_SCRATCH_BASE, 0x1000);
    cpu.set_tpidrro_el0(TLS_SCRATCH_BASE);

    // Sentinel page: one movz/svc pair per initializer slot.
    let inits = dyld64::read_init_functions(
        mem,
        &loaded.init_slots,
        loaded.load_addr,
        loaded.image_base,
    );
    if !inits.is_empty() {
        mem.map_range(CONSTRUCTOR_SENTINEL_BASE, 0x1000);
        assert!(inits.len() * 8 <= 0x1000, "constructor sentinel overflow");
        for i in 0..inits.len() {
            // movz x16, #0xFFFE
            let movz = 0xD280_0000 | (SVC_INTERNAL_RETURNED << 5) | 16;
            mem.write_u32(CONSTRUCTOR_SENTINEL_BASE + i as u64 * 8, movz);
            mem.write_u32(
                CONSTRUCTOR_SENTINEL_BASE + i as u64 * 8 + 4,
                encode_svc(SVC_SYSCALL),
            );
        }
    }

    for (i, init_fn) in inits.iter().enumerate() {
        cpu.set_sp(sp);
        cpu.set_pc(*init_fn);
        cpu.set_reg(30, CONSTRUCTOR_SENTINEL_BASE + i as u64 * 8);
        for r in 0..8 {
            cpu.set_reg(r, 0);
        }
        match run_phase(
            cpu,
            mem,
            trampolines,
            host,
            syscalls,
            CONSTRUCTOR_SENTINEL_BASE + i as u64 * 8,
            budget,
        ) {
            PhaseResult::Returned => report.constructors_ran += 1,
            PhaseResult::Terminal(outcome) => {
                report.outcome = Some(outcome);
                report.syscalls = syscalls.calls.clone();
                report.missing = host.missing_report();
                return report;
            }
        }
    }

    // Entry point (prep_stack_for_main already put argc/argv/envp/apple in
    // x0..x3 — the LC_MAIN handoff).
    cpu.set_sp(sp);
    cpu.set_pc(entry);
    report.entry_reached = true;
    match run_phase(
        cpu,
        mem,
        trampolines,
        host,
        syscalls,
        VAddr::MAX, // the entry has no sentinel to return to
        budget,
    ) {
        PhaseResult::Returned => unreachable!("entry point cannot return to a sentinel"),
        PhaseResult::Terminal(outcome) => report.outcome = Some(outcome),
    }
    report.syscalls = syscalls.calls.clone();
    report.missing = host.missing_report();
    report
}

/// `--dump-missing-hle64`: link the real stub from an extracted `.app`
/// directory and run it, printing a JSON report of every HLE symbol that
/// was called but has no host implementation.
///
/// This is the F2 work list, measured on the real binary instead of
/// guessed from import tables.
pub fn dump_missing_hle(app_dir: &Path) -> Result<String, String> {
    let stub_path = app_dir.join("SubwaySurf");
    let bytes = std::fs::read(&stub_path)
        .map_err(|e| format!("could not read {}: {e}", stub_path.display()))?;
    let image = crate::mach_o64::parse(&bytes)
        .map_err(|e| format!("stub is not a loadable Mach-O: {e:?}"))?;
    if image.cryptid != 0 {
        return Err(format!(
            "refusing to run: FairPlay cryptid = {} (encrypted binaries are out of scope)",
            image.cryptid
        ));
    }
    let base = dyld64::image_base(&image);

    let mut mem = Mem64::new();
    let mut resolver = dyld64::TrapResolver::default();
    let loaded = dyld64::load_and_link(&mut mem, &bytes, base, &mut resolver)
        .map_err(|e| format!("linking failed: {e:?}"))?;

    let mut cpu = Cpu::new();
    let mut host = RecordingHleHost::default();
    let args = ProcessArgs {
        argv: &["/var/containers/Bundle/Application/00000000-0000-0000-0000-000000000000/SubwaySurf.app/SubwaySurf"],
        envp: &["HOME=/var/mobile", "TMPDIR=/private/var/tmp"],
        apple: &[],
    };
    let report = run_a64_image(
        &mut mem,
        &mut cpu,
        &loaded,
        &resolver.trampolines,
        &mut host,
        &args,
        50_000_000,
    );

    let entry = loaded.entry.map(|e| format!("{e:#x}")).unwrap_or_default();
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"app\": {},\n", json_string(&app_dir.display().to_string())));
    out.push_str(&format!("  \"image_base\": \"{base:#x}\",\n"));
    out.push_str(&format!("  \"entry\": \"{entry}\",\n"));
    out.push_str(&format!("  \"imports_bound\": {},\n", resolver.bound.len()));
    out.push_str(&format!("  \"constructors_ran\": {},\n", report.constructors_ran));
    out.push_str(&format!("  \"entry_reached\": {},\n", report.entry_reached));
    match &report.outcome {
        Some(A64Outcome::Exit(code)) => out.push_str(&format!(
            "  \"outcome\": {{ \"kind\": \"exit\", \"code\": {code} }},\n"
        )),
        Some(A64Outcome::MemoryFault { pc, lr }) => out.push_str(&format!(
            "  \"outcome\": {{ \"kind\": \"memory_fault\", \"pc\": \"{pc:#x}\", \"lr\": \"{lr:#x}\" }},\n"
        )),
        Some(A64Outcome::UndefinedInstruction { pc }) => out.push_str(&format!(
            "  \"outcome\": {{ \"kind\": \"undefined_instruction\", \"pc\": \"{pc:#x}\" }},\n"
        )),
        Some(A64Outcome::Breakpoint { pc }) => out.push_str(&format!(
            "  \"outcome\": {{ \"kind\": \"breakpoint\", \"pc\": \"{pc:#x}\" }},\n"
        )),
        Some(A64Outcome::TickBudget) => {
            out.push_str("  \"outcome\": { \"kind\": \"tick_budget\" },\n")
        }
        None => out.push_str("  \"outcome\": null,\n"),
    }
    out.push_str("  \"missing_hle\": [\n");
    for (i, (symbol, count)) in report.missing.iter().enumerate() {
        let comma = if i + 1 == report.missing.len() { "" } else { "," };
        out.push_str(&format!(
            "    {{ \"symbol\": {}, \"calls\": {} }}{}\n",
            json_string(symbol),
            count,
            comma
        ));
    }
    out.push_str("  ],\n");
    out.push_str("  \"syscalls\": [\n");
    for (i, (nr, count)) in report.syscalls.iter().enumerate() {
        let comma = if i + 1 == report.syscalls.len() { "" } else { "," };
        out.push_str(&format!("    {{ \"nr\": {nr}, \"calls\": {count} }}{comma}\n"));
    }
    out.push_str("  ]\n}\n");
    Ok(out)
}

/// A JSON string literal with the minimal escaping set.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
mod tests {
    use super::*;
    use crate::dyld64::TrampolinePage;

    // Minimal A64 encoders for hand-built test images (mirrors dyld64 tests).
    fn movz64(rd: u32, imm: u32, hw: u32) -> u32 {
        0xD280_0000 | ((imm & 0xFFFF) << 5) | (hw << 21) | rd
    }
    fn movk64(rd: u32, imm: u32, hw: u32) -> u32 {
        0xF280_0000 | ((imm & 0xFFFF) << 5) | (hw << 21) | rd
    }
    fn load_const(rd: u32, value: u64) -> [u32; 4] {
        [
            movz64(rd, (value & 0xFFFF) as u32, 0),
            movk64(rd, ((value >> 16) & 0xFFFF) as u32, 1),
            movk64(rd, ((value >> 32) & 0xFFFF) as u32, 2),
            movk64(rd, ((value >> 48) & 0xFFFF) as u32, 3),
        ]
    }

    const CODE: VAddr = 0x1_0000_0000;
    const DATA: VAddr = 0x1_0001_0000;
    const ENTRY_OFF: u64 = 200; // ehdr + cmds

    /// Hand-built executable: no fixups, code at CODE (entry at ENTRY_OFF),
    /// a data page at DATA backed by the file.
    fn raw_executable(code_words: &[u32]) -> Vec<u8> {
        // ehdr(32) + seg_TEXT(72) + seg_DATA(72) + LC_MAIN(24) = 200
        let mut b = Vec::new();
        b.extend_from_slice(&0xFEEDFACFu32.to_le_bytes()); // magic
        b.extend_from_slice(&0x0100_000Cu32.to_le_bytes()); // cputype arm64
        b.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
        b.extend_from_slice(&2u32.to_le_bytes()); // MH_EXECUTE
        b.extend_from_slice(&3u32.to_le_bytes()); // ncmds
        b.extend_from_slice(&168u32.to_le_bytes()); // sizeofcmds
        b.extend_from_slice(&0u32.to_le_bytes()); // flags
        b.extend_from_slice(&0u32.to_le_bytes()); // reserved
        let segments: [(&str, VAddr, u64); 2] =
            [("__TEXT", CODE, 0), ("__DATA", DATA, 0x1000)];
        for (name, vmaddr, fileoff) in segments {
            let mut cmd = [0u8; 72];
            cmd[0..4].copy_from_slice(&0x19u32.to_le_bytes()); // LC_SEGMENT_64
            cmd[4..8].copy_from_slice(&72u32.to_le_bytes());
            let nm = name.as_bytes();
            cmd[8..8 + nm.len()].copy_from_slice(nm);
            // segment_command_64: cmd(4) cmdsize(4) segname(16) vmaddr(8)
            // vmsize(8) fileoff(8) filesize(8) maxprot(4) initprot(4)
            // nsects(4) flags(4).
            cmd[24..32].copy_from_slice(&vmaddr.to_le_bytes()); // vmaddr
            cmd[32..40].copy_from_slice(&0x1000u64.to_le_bytes()); // vmsize
            cmd[40..48].copy_from_slice(&fileoff.to_le_bytes()); // fileoff
            cmd[48..56].copy_from_slice(&0x1000u64.to_le_bytes()); // filesize
            cmd[56..60].copy_from_slice(&7u32.to_le_bytes()); // maxprot
            cmd[60..64].copy_from_slice(&5u32.to_le_bytes()); // initprot
            // nsects (64..68) and flags (68..72) stay zero.
            b.extend_from_slice(&cmd);
        }
        // LC_MAIN
        b.extend_from_slice(&0x8000_0028u32.to_le_bytes());
        b.extend_from_slice(&24u32.to_le_bytes());
        b.extend_from_slice(&ENTRY_OFF.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        assert_eq!(b.len(), 200);
        for w in code_words {
            b.extend_from_slice(&w.to_le_bytes());
        }
        b.resize(0x2000, 0);
        b
    }

    /// Link a raw test image (no fixups) and place trampoline addresses for
    /// `symbols` into consecutive slots at DATA.
    fn link_raw(mem: &mut Mem64, bytes: &[u8], symbols: &[&str]) -> (TrampolinePage, LoadedImage) {
        crate::mach_o64::parse(bytes).unwrap();
        let mut tramp = TrampolinePage::default();
        let mut resolver = dyld64::TrapResolver::default();
        let loaded = dyld64::load_and_link(mem, bytes, CODE, &mut resolver).unwrap();
        for (i, s) in symbols.iter().enumerate() {
            let addr = tramp.trampoline_for(mem, s);
            mem.write_u64(DATA + i as u64 * 8, addr);
        }
        (tramp, loaded)
    }

    #[test]
    fn stack_layout_round_trips_like_the_kernel() {
        let mut mem = Mem64::new();
        let mut cpu = Cpu::new();
        mem.map_range(MAIN_STACK_TOP - MAIN_STACK_SIZE, MAIN_STACK_SIZE);
        let args = ProcessArgs {
            argv: &["/var/mobile/SubwaySurf.app/SubwaySurf", "-psn_0_1"],
            envp: &["HOME=/var/mobile"],
            apple: &["initializer_source=x"],
        };
        let sp = prep_stack_for_main(&mut mem, &mut cpu, MAIN_STACK_TOP, &args, 0x1000);

        assert_eq!(sp % 16, 0, "SP must be 16-byte aligned");
        assert_eq!(mem.read_u64(sp), 2, "argc");
        let argv0 = mem.read_u64(sp + 8);
        let argv1 = mem.read_u64(sp + 16);
        assert_eq!(mem.read_u64(sp + 24), 0, "argv NULL terminator");
        let buf = mem.read_bytes(argv0, 38);
        assert_eq!(
            String::from_utf8_lossy(&buf),
            "/var/mobile/SubwaySurf.app/SubwaySurf\0"
        );
        let buf = mem.read_bytes(argv1, 9);
        assert_eq!(String::from_utf8_lossy(&buf), "-psn_0_1\0");
        let envp0 = mem.read_u64(sp + 32);
        let buf = mem.read_bytes(envp0, 17);
        assert_eq!(String::from_utf8_lossy(&buf), "HOME=/var/mobile\0");
        assert_eq!(mem.read_u64(sp + 40), 0, "envp NULL terminator");
        let apple0 = mem.read_u64(sp + 48);
        let buf = mem.read_bytes(apple0, 21);
        assert_eq!(String::from_utf8_lossy(&buf), "initializer_source=x\0");
        assert_eq!(mem.read_u64(sp + 56), 0, "apple NULL terminator");
        // auxv: first pair is AT_PAGESZ; the last pair (just below the
        // string pool) is AT_NULL.
        assert_eq!(mem.read_u64(sp + 64), AT_PAGESZ);
        assert_eq!(mem.read_u64(sp + 72), 4096);
        let auxv_end = sp + 64 + 6 * 16;
        assert_eq!(mem.read_u64(auxv_end), AT_NULL);
        assert_eq!(mem.read_u64(auxv_end + 8), 0);
        let s = mem.read_bytes(auxv_end + 16, 4);
        assert_eq!(&s, b"/var");

        // Registers: the LC_MAIN handoff.
        assert_eq!(cpu.get_reg(0), 2);
        assert_eq!(cpu.get_reg(1), sp + 8);
        assert_eq!(cpu.get_reg(2), sp + 32);
        assert_eq!(cpu.get_reg(3), sp + 48);
        assert_eq!(cpu.sp(), sp);
    }

    #[test]
    fn run_dispatches_hle_calls_and_bsd_syscalls() {
        // Entry: load trampoline addr from DATA, call it, then exit(7).
        // SYS_exit is 1 on Darwin; the code travels in x0.
        let mut code: Vec<u32> = Vec::new();
        code.extend_from_slice(&load_const(8, DATA));
        code.push(0xF940_0109); // ldr x9, [x8]
        code.push(0xD63F_0120); // blr x9
        code.extend_from_slice(&load_const(16, 1)); // mov x16, #SYS_exit
        code.extend_from_slice(&load_const(0, 7)); // mov x0, #7 (exit code)
        code.push(0xD400_1001); // svc #0x80

        let bytes = raw_executable(&code);
        let mut mem = Mem64::new();
        let (tramp, loaded) = link_raw(&mut mem, &bytes, &["test_hook"]);

        let mut cpu = Cpu::new();
        let mut host = RecordingHleHost::default();
        let args = ProcessArgs { argv: &["app"], envp: &[], apple: &[] };
        let report =
            run_a64_image(&mut mem, &mut cpu, &loaded, &tramp, &mut host, &args, 1_000_000);

        assert!(report.entry_reached);
        assert_eq!(report.constructors_ran, 0);
        assert_eq!(report.outcome, Some(A64Outcome::Exit(7)));
        assert_eq!(report.missing, vec![("test_hook".to_string(), 1)]);
        assert_eq!(report.syscalls, vec![(1, 1)]);
    }

    #[test]
    fn run_reports_memory_fault_instead_of_panicking() {
        // Entry: ldr x0, [x20] with x20 = 0 — the __PAGEZERO is unmapped,
        // so a null load faults. (x1 cannot be used: prep_stack_for_main
        // writes argv into it, per the LC_MAIN handoff.)
        let code = [0xF940_02A0, 0xD400_1001];
        let bytes = raw_executable(&code);
        let mut mem = Mem64::new();
        let (tramp, loaded) = link_raw(&mut mem, &bytes, &[]);

        let mut cpu = Cpu::new();
        let mut host = RecordingHleHost::default();
        let args = ProcessArgs { argv: &["app"], envp: &[], apple: &[] };
        let report =
            run_a64_image(&mut mem, &mut cpu, &loaded, &tramp, &mut host, &args, 1_000_000);
        match report.outcome {
            Some(A64Outcome::MemoryFault { .. }) => {}
            other => panic!("expected MemoryFault, got {other:?}"),
        }
    }

    #[test]
    fn undecodable_instructions_halt_as_undefined_not_abort() {
        // A64 0x00000000 is a permanent undefined encoding; dynarmic
        // routes it to InterpreterFallback, which must halt cleanly
        // instead of aborting the host process.
        let code = [0x0000_0000, 0xD400_1001];
        let bytes = raw_executable(&code);
        let mut mem = Mem64::new();
        let (tramp, loaded) = link_raw(&mut mem, &bytes, &[]);

        let mut cpu = Cpu::new();
        let mut host = RecordingHleHost::default();
        let args = ProcessArgs { argv: &["app"], envp: &[], apple: &[] };
        let report =
            run_a64_image(&mut mem, &mut cpu, &loaded, &tramp, &mut host, &args, 1_000_000);
        match report.outcome {
            Some(A64Outcome::UndefinedInstruction { .. }) => {}
            other => panic!("expected UndefinedInstruction, got {other:?}"),
        }
    }

    #[test]
    fn syscall_errors_do_not_stop_the_run() {
        // getpid (implemented), an unimplemented syscall (must fail with
        // ENOSYS but keep running), then exit(3).
        let mut code: Vec<u32> = Vec::new();
        code.extend_from_slice(&load_const(16, 20)); // x16 = SYS_getpid
        code.push(0xD400_1001);
        code.extend_from_slice(&load_const(16, 99)); // unimplemented
        code.push(0xD400_1001);
        code.extend_from_slice(&load_const(0, 3)); // x0 = 3
        code.extend_from_slice(&load_const(16, 1)); // x16 = SYS_exit
        code.push(0xD400_1001);

        let bytes = raw_executable(&code);
        let mut mem = Mem64::new();
        let (tramp, loaded) = link_raw(&mut mem, &bytes, &[]);
        let mut cpu = Cpu::new();
        let mut host = RecordingHleHost::default();
        let args = ProcessArgs { argv: &["app"], envp: &[], apple: &[] };
        let report =
            run_a64_image(&mut mem, &mut cpu, &loaded, &tramp, &mut host, &args, 1_000_000);
        assert_eq!(report.outcome, Some(A64Outcome::Exit(3)));
        assert_eq!(report.syscalls, vec![(20, 1), (99, 1), (1, 1)]);
        // The carry-flag contract (C set on error, clear on success) is
        // what guest libc wrappers observe via b.cs/b.cc; it cannot be read
        // back through the wrapper API, so it is exercised indirectly: the
        // ENOSYS call did not terminate the run and the exit did.
    }

    #[test]
    fn constructors_run_before_the_entry_point() {
        // Constructor at CODE+0x100: increments a counter at DATA+0x100,
        // then returns. Entry: exits with the counter value.
        let ctor = CODE + 0x100;
        let mut image: Vec<u32> = Vec::new();
        // Entry words (indices 0..11).
        image.extend_from_slice(&load_const(9, DATA + 0x100));
        image.push(0xB940_012A); // ldr w10, [x9]
        image.extend_from_slice(&load_const(16, 1)); // x16 = SYS_exit
        image.push(0xAA0A_03E0); // mov x0, x10
        image.push(0xD400_1001); // svc #0x80
        // Constructor (word index 14 => byte offset 56 => CODE+0x100).
        assert_eq!(image.len(), 11);
        while image.len() < 14 {
            image.push(0xD503_201F); // nop
        }
        image.extend_from_slice(&load_const(9, DATA + 0x100));
        image.push(0xB940_012A); // ldr w10, [x9]
        image.push(0x1100_054A); // add w10, w10, #1
        image.push(0xB900_012A); // str w10, [x9]
        image.push(0xD65F_03C0); // ret

        let bytes = raw_executable(&image);
        let mut mem = Mem64::new();
        let (tramp, mut loaded) = link_raw(&mut mem, &bytes, &[]);
        // Emulate what a __DATA,__mod_init_func slot would look like after
        // fixups: a pointer to the constructor.
        loaded.init_slots = vec![dyld64::InitSlot::Pointer(DATA + 0x200)];
        mem.write_u64(DATA + 0x200, ctor);

        let mut cpu = Cpu::new();
        let mut host = RecordingHleHost::default();
        let args = ProcessArgs { argv: &["app"], envp: &[], apple: &[] };
        let report =
            run_a64_image(&mut mem, &mut cpu, &loaded, &tramp, &mut host, &args, 1_000_000);

        assert_eq!(report.constructors_ran, 1);
        assert!(report.entry_reached);
        assert_eq!(report.outcome, Some(A64Outcome::Exit(1)));
    }

    #[test]
    fn json_string_escapes_control_characters() {
        assert_eq!(json_string("simple"), "\"simple\"");
        assert_eq!(json_string("a\"b\\c\nd\te"), "\"a\\\"b\\\\c\\nd\\te\"");
        assert_eq!(json_string("\u{1}"), "\"\\u0001\"");
    }

    /// Real-binary P3 milestone (ignored by default: needs the IPA on
    /// disk). Point `TOUCHHLE_A64_IPA_APP` at the extracted `SubwaySurf.app`
    /// and run `cargo test -- --ignored`.
    ///
    /// Measured behaviour (2026-11-11): the stub links (56 imports),
    /// reaches its entry at `0x1000040f0`, runs real guest code through the
    /// ObjC ARC entry glue and faults when a `blr` follows a NULL
    /// `_objc_msgSend` return. The **first missing-HLE family is ObjC/ARC**
    /// (`_objc_autoreleasePoolPush`, `_objc_msgSend`,
    /// `_objc_retainAutoreleasedReturnValue`, `_objc_release`,
    /// `_objc_autoreleaseReturnValue`, `_objc_autoreleasePoolPop`) —
    /// `dlopen`/`dlsym` sit behind it and appear once F2's ObjC runtime
    /// lets the stub get that far.
    #[test]
    #[ignore]
    fn real_subway_surfers_stub_runs_to_the_first_missing_hle() {
        let app = std::env::var("TOUCHHLE_A64_IPA_APP").expect("set TOUCHHLE_A64_IPA_APP");
        let json = dump_missing_hle(Path::new(&app)).expect("dump failed");
        eprintln!("{json}");
        assert!(json.contains("\"entry_reached\": true"));
        assert!(json.contains("_objc_msgSend"));
        assert!(json.contains("_objc_autoreleasePoolPush"));
    }
}
