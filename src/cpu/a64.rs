/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! CPU emulation for 64-bit (AArch64) guest code.
//!
//! This is the F1 groundwork for running modern arm64 apps (see
//! PROGRESS.md decision D1). It mirrors `src/cpu.rs` (the A32 core) so the
//! two stay easy to compare, and is deliberately self-contained: it does not
//! touch the 32-bit [`crate::mem::Mem`] model at all. It ships its own
//! page-based [`Mem64`], which is the seed of the 64-bit guest memory model
//! (P1) and is enough to execute real A64 code end-to-end.
//!
//! What is intentionally NOT here yet (tracked in PROGRESS.md):
//! * fastmem/page-table integration with dynarmic (needs the real Mem64);
//! * the Mach-O arm64 loader that maps binaries into a [`Mem64`];
//! * wiring A64 apps into the HLE syscall/hook layer.

use crate::mem::PAGE_SIZE;

// Import functions and types from C++ (see lib_a64.cpp and the wrapper
// crate's lib.rs).
use touchHLE_dynarmic_wrapper::*;

/// Guest address type for A64 apps.
pub type VAddr = u64;

/// Register numbers that mean something across the A64 ABI.
pub mod reg {
    /// Frame pointer register (x29).
    pub const FP: usize = 29;
    /// Link register (x30).
    pub const LR: usize = 30;
}

/// Snapshot of the guest CPU state, for guest thread switching.
///
/// This is the wrapper crate's ABI type, kept in sync by construction with
/// `touchHLE_DynarmicA64Context` in `src/cpu/dynarmic_wrapper/lib_a64.cpp`.
pub use touchHLE_dynarmic_wrapper::touchHLE_DynarmicA64Context as CpuContext;

/// Why CPU execution ended.
#[derive(Debug)]
pub enum CpuState {
    /// Execution halted due to using up all remaining ticks (normal
    /// execution) or after the single instruction was executed (stepping).
    Normal,
    /// SVC instruction encountered (the guest's syscall entry).
    Svc(u32),
    /// An error was encountered.
    Error(CpuError),
}

/// A reason that can cause CPU execution to be interrupted.
#[derive(Debug, Clone, PartialEq)]
pub enum CpuError {
    MemoryError,
    UndefinedInstruction,
    Breakpoint,
}

// Export functions for use by C++
//
// These are the A64 equivalents of the touchHLE_cpu_read_*/write_* callbacks
// in src/cpu.rs. The same catch_unwind discipline applies: a Rust panic (an
// unmapped or misaligned guest access, for now) must not unwind through the
// C++ JIT frames, so it is caught and turned into a memory abort.

fn cpu_a64_read_impl<T: Copy + Default>(
    mem: *mut Mem64,
    addr: VAddr,
    error: *mut bool,
    read: fn(&mut Mem64, VAddr) -> T,
) -> T {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mem = unsafe { &mut *mem };
        read(mem, addr)
    }));
    unsafe {
        error.write(res.is_err());
    }
    res.unwrap_or_default()
}

fn cpu_a64_write_impl<T: Copy>(
    mem: *mut Mem64,
    addr: VAddr,
    value: T,
    write: fn(&mut Mem64, VAddr, T),
) -> bool {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mem = unsafe { &mut *mem };
        write(mem, addr, value)
    }));
    res.is_err()
}

#[no_mangle]
extern "C" fn touchHLE_cpu_a64_read_u8(
    mem: *mut Mem64,
    addr: VAddr,
    error: *mut bool,
) -> u8 {
    cpu_a64_read_impl(mem, addr, error, Mem64::read_u8)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_a64_read_u16(
    mem: *mut Mem64,
    addr: VAddr,
    error: *mut bool,
) -> u16 {
    cpu_a64_read_impl(mem, addr, error, Mem64::read_u16)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_a64_read_u32(
    mem: *mut Mem64,
    addr: VAddr,
    error: *mut bool,
) -> u32 {
    cpu_a64_read_impl(mem, addr, error, Mem64::read_u32)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_a64_read_u64(
    mem: *mut Mem64,
    addr: VAddr,
    error: *mut bool,
) -> u64 {
    cpu_a64_read_impl(mem, addr, error, Mem64::read_u64)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_a64_write_u8(mem: *mut Mem64, addr: VAddr, value: u8) -> bool {
    cpu_a64_write_impl(mem, addr, value, Mem64::write_u8)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_a64_write_u16(mem: *mut Mem64, addr: VAddr, value: u16) -> bool {
    cpu_a64_write_impl(mem, addr, value, Mem64::write_u16)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_a64_write_u32(mem: *mut Mem64, addr: VAddr, value: u32) -> bool {
    cpu_a64_write_impl(mem, addr, value, Mem64::write_u32)
}
#[no_mangle]
extern "C" fn touchHLE_cpu_a64_write_u64(mem: *mut Mem64, addr: VAddr, value: u64) -> bool {
    cpu_a64_write_impl(mem, addr, value, Mem64::write_u64)
}

/// Page-based 64-bit guest memory.
///
/// The seed of the 64-bit memory model (PROGRESS.md P1). Pages are allocated
/// on demand via [Mem64::map] (the future Mach-O loader will call this for
/// each mapped segment) and any access to an unmapped address panics, which
/// the CPU callbacks above turn into a [CpuError::MemoryError] — the same
/// user-visible behavior as a segfault on a real device.
///
/// This is intentionally the simplest correct thing: per-page heap
/// allocations with a hash map. The performance-critical follow-ups (a
/// contiguous host reservation like `Mem` uses for fastmem, read-only
/// segments, file-backed pages) land together with the arm64 loader.
pub struct Mem64 {
    pages: std::collections::HashMap<u64, Box<[u8; PAGE_SIZE as usize]>>,
}

impl Mem64 {
    pub fn new() -> Mem64 {
        Mem64 {
            pages: std::collections::HashMap::new(),
        }
    }

    fn page_base(addr: VAddr) -> u64 {
        // PAGE_SIZE is 0x1000; dynarmic's A64 default page size matches.
        addr & !(PAGE_SIZE as u64 - 1)
    }

    fn offset(addr: VAddr) -> usize {
        (addr & (PAGE_SIZE as u64 - 1)) as usize
    }

    /// Map a fresh zeroed page containing `addr`.
    pub fn map_page(&mut self, addr: VAddr) {
        self.pages
            .entry(Self::page_base(addr))
            .or_insert_with(|| Box::new([0; PAGE_SIZE as usize]));
    }

    /// Map every page overlapping `addr..addr + size` as zeroed memory.
    pub fn map_range(&mut self, addr: VAddr, size: u64) {
        let mut page = Self::page_base(addr);
        let end = addr.checked_add(size).expect("map range overflow");
        while page < Self::page_base(end - 1) + PAGE_SIZE as u64 {
            self.map_page(page);
            page += PAGE_SIZE as u64;
        }
    }

    pub fn is_mapped(&self, addr: VAddr) -> bool {
        self.pages.contains_key(&Self::page_base(addr))
    }

    fn ensure(&self, addr: VAddr) {
        if !self.is_mapped(addr) {
            panic!("Mem64: access to unmapped address {addr:#x}");
        }
    }

    /// Read bytes spanning page boundaries, for the loader's segment copies.
    pub fn read_bytes(&self, addr: VAddr, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut cur = addr;
        for _ in 0..len {
            self.ensure(cur);
            let page = &self.pages[&Self::page_base(cur)];
            out.push(page[Self::offset(cur)]);
            cur += 1;
        }
        out
    }

    /// Write bytes spanning page boundaries, for the loader's segment copies.
    pub fn write_bytes(&mut self, addr: VAddr, bytes: &[u8]) {
        let mut cur = addr;
        for &b in bytes {
            self.ensure(cur);
            let page = self.pages.get_mut(&Self::page_base(cur)).unwrap();
            page[Self::offset(cur)] = b;
            cur += 1;
        }
    }

    pub fn read_u8(&mut self, addr: VAddr) -> u8 {
        self.ensure(addr);
        self.pages[&Self::page_base(addr)][Self::offset(addr)]
    }
    pub fn read_u16(&mut self, addr: VAddr) -> u16 {
        u16::from_le_bytes(self.read_bytes(addr, 2).try_into().unwrap())
    }
    pub fn read_u32(&mut self, addr: VAddr) -> u32 {
        u32::from_le_bytes(self.read_bytes(addr, 4).try_into().unwrap())
    }
    pub fn read_u64(&mut self, addr: VAddr) -> u64 {
        u64::from_le_bytes(self.read_bytes(addr, 8).try_into().unwrap())
    }

    pub fn write_u8(&mut self, addr: VAddr, value: u8) {
        self.ensure(addr);
        self.pages.get_mut(&Self::page_base(addr)).unwrap()[Self::offset(addr)] = value;
    }
    pub fn write_u16(&mut self, addr: VAddr, value: u16) {
        self.write_bytes(addr, &value.to_le_bytes());
    }
    pub fn write_u32(&mut self, addr: VAddr, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }
    pub fn write_u64(&mut self, addr: VAddr, value: u64) {
        self.write_bytes(addr, &value.to_le_bytes());
    }
}

/// The A64 CPU, wrapping the dynarmic A64 JIT.
pub struct Cpu {
    wrapper: *mut touchHLE_DynarmicA64Wrapper,
}

impl Drop for Cpu {
    fn drop(&mut self) {
        unsafe { touchHLE_DynarmicA64Wrapper_delete(self.wrapper) }
    }
}

impl Cpu {
    /// Construct a new A64 CPU. No memory is bound at construction time;
    /// the [Mem64] is passed to [Cpu::run_or_step] like on the A32 side.
    pub fn new() -> Cpu {
        Cpu {
            wrapper: unsafe { touchHLE_DynarmicA64Wrapper_new() },
        }
    }

    pub fn get_reg(&self, index: usize) -> u64 {
        assert!(index < 31, "A64 register index out of range: {index}");
        unsafe { touchHLE_DynarmicA64Wrapper_get_reg(self.wrapper, index) }
    }
    pub fn set_reg(&mut self, index: usize, value: u64) {
        assert!(index < 31, "A64 register index out of range: {index}");
        unsafe { touchHLE_DynarmicA64Wrapper_set_reg(self.wrapper, index, value) }
    }

    pub fn sp(&self) -> u64 {
        unsafe { touchHLE_DynarmicA64Wrapper_get_sp(self.wrapper) }
    }
    pub fn set_sp(&mut self, value: u64) {
        unsafe { touchHLE_DynarmicA64Wrapper_set_sp(self.wrapper, value) }
    }
    pub fn pc(&self) -> u64 {
        unsafe { touchHLE_DynarmicA64Wrapper_get_pc(self.wrapper) }
    }
    pub fn set_pc(&mut self, value: u64) {
        unsafe { touchHLE_DynarmicA64Wrapper_set_pc(self.wrapper, value) }
    }

    pub fn set_fpcr(&mut self, value: u32) {
        unsafe { touchHLE_DynarmicA64Wrapper_set_fpcr(self.wrapper, value) }
    }
    pub fn set_fpsr(&mut self, value: u32) {
        unsafe { touchHLE_DynarmicA64Wrapper_set_fpsr(self.wrapper, value) }
    }
    pub fn set_pstate(&mut self, value: u32) {
        unsafe { touchHLE_DynarmicA64Wrapper_set_pstate(self.wrapper, value) }
    }

    /// Set the value guest code reads from TPIDRRO_EL0. This is per-thread
    /// state (the A64 successor of the A32 TPIDRURO), so it must be set
    /// again on every thread switch.
    pub fn set_tpidrro_el0(&mut self, value: u64) {
        unsafe { touchHLE_DynarmicA64Wrapper_set_tpidrro_el0(self.wrapper, value) }
    }

    /// Start address of the instruction that raised the most recent
    /// [CpuError::UndefinedInstruction] or [CpuError::Breakpoint]. Only
    /// meaningful immediately after [Cpu::run_or_step] returned that error.
    pub fn last_exception_pc(&self) -> VAddr {
        unsafe { touchHLE_DynarmicA64Wrapper_last_exception_pc(self.wrapper) }
    }

    /// Swap the current CPU state with the state stored in `context`.
    pub fn swap_context(&mut self, context: &mut CpuContext) {
        unsafe { touchHLE_DynarmicA64Wrapper_swap_context(self.wrapper, context) }
    }

    /// Clear dynarmic's instruction cache for some address range, for the
    /// dynamic linker when it rewrites code.
    pub fn invalidate_cache_range(&mut self, base: VAddr, size: u64) {
        unsafe { touchHLE_DynarmicA64Wrapper_invalidate_cache_range(self.wrapper, base, size) }
    }

    /// Start CPU execution. See the A32 [crate::cpu::Cpu::run_or_step] for
    /// the ticks/stepping semantics; the return mapping is identical.
    #[must_use]
    pub fn run_or_step(&mut self, mem: &mut Mem64, ticks: Option<&mut u64>) -> CpuState {
        let res = unsafe {
            touchHLE_DynarmicA64Wrapper_run_or_step(
                self.wrapper,
                mem as *mut Mem64 as *mut touchHLE_Mem64,
                ticks,
            )
        };
        match res {
            -1 => CpuState::Normal,
            -2 => CpuState::Error(CpuError::MemoryError),
            -3 => CpuState::Error(CpuError::UndefinedInstruction),
            -4 => CpuState::Error(CpuError::Breakpoint),
            _ if res < -4 => panic!("Unexpected A64 CPU execution result"),
            svc => CpuState::Svc(svc as u32),
        }
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "android")))]
mod tests {
    use super::{Cpu, CpuState, Mem64, VAddr};

    /// Base of the code page used by the test, matching the app-side layout
    /// of the SubwaySurf stub (__TEXT at 0x100000000, after a 4 GiB
    /// __PAGEZERO).
    const CODE: VAddr = 0x1_0000_0000;
    const STACK: VAddr = 0x1_0f00_0000;

    fn program(mem: &mut Mem64, words: &[u32]) {
        mem.map_range(CODE, 0x1000);
        mem.map_range(STACK, 0x1000);
        let mut addr = CODE;
        for &w in words {
            mem.write_u32(addr, w);
            addr += 4;
        }
    }

    #[test]
    fn a64_cpu_executes_code_and_reaches_svc() {
        let mut mem = Mem64::new();
        // movz x0, #7
        // add  x1, x0, #35          ; x1 = 42
        // str  x1, [sp]             ; store through the stack pointer
        // ldr  x2, [sp]             ; load it back
        // movz x3, #0x1234, lsl #16 ; build a 32-bit constant
        // movk x3, #0xbeef          ; complete it (low half)
        // svc  #0x80                ; guest syscall entry
        program(
            &mut mem,
            &[
                0xD280_00E0,
                0x9100_8C01,
                0xF900_03E1,
                0xF940_03E2,
                0xD2A2_4683,
                0xF297_DDE3,
                0xD400_1001,
            ],
        );

        let mut cpu = Cpu::new();
        cpu.set_pc(CODE);
        cpu.set_sp(STACK + 0x800);

        // ticks = Some: run until the SVC or the tick budget runs out.
        // (ticks = None steps a single instruction instead.)
        let mut ticks = 1_000;
        match cpu.run_or_step(&mut mem, Some(&mut ticks)) {
            CpuState::Svc(svc) => assert_eq!(svc, 0x80),
            other => panic!("expected Svc(0x80), got {other:?}"),
        }
        assert_eq!(cpu.get_reg(0), 7);
        assert_eq!(cpu.get_reg(1), 42);
        assert_eq!(cpu.get_reg(2), 42);
        assert_eq!(cpu.get_reg(3), 0x1234_beef);
        // SP-relative store actually landed in guest memory.
        assert_eq!(mem.read_u64(STACK + 0x800), 42);
    }

    #[test]
    fn a64_cpu_reports_unmapped_access_as_memory_error() {
        let mut mem = Mem64::new();
        // ldr x0, [x1] with x1 pointing at unmapped memory, then svc.
        program(&mut mem, &[0xF940_0020, 0xD400_1001]);

        let mut cpu = Cpu::new();
        cpu.set_pc(CODE);
        cpu.set_sp(STACK);
        cpu.set_reg(1, 0xdead_0000); // never mapped

        assert!(matches!(
            cpu.run_or_step(&mut mem, None),
            CpuState::Error(super::CpuError::MemoryError)
        ));
    }

    #[test]
    fn a64_cpu_steps_one_instruction_at_a_time() {
        let mut mem = Mem64::new();
        // add x1, x1, #1 twice, then svc.
        program(&mut mem, &[0x9100_0421, 0x9100_0421, 0xD400_1001]);

        let mut cpu = Cpu::new();
        cpu.set_pc(CODE);
        cpu.set_sp(STACK);
        cpu.set_reg(1, 9);

        assert!(matches!(cpu.run_or_step(&mut mem, None), CpuState::Normal));
        assert_eq!(cpu.get_reg(1), 10);
        assert_eq!(cpu.pc(), CODE + 4);
        assert!(matches!(cpu.run_or_step(&mut mem, None), CpuState::Normal));
        assert_eq!(cpu.get_reg(1), 11);
        assert!(matches!(
            cpu.run_or_step(&mut mem, None),
            CpuState::Svc(0x80)
        ));
    }

    #[test]
    fn a64_context_swap_round_trips() {
        let mut mem = Mem64::new();
        program(&mut mem, &[0xD400_1001]); // svc #0x80
        let mut cpu = Cpu::new();
        cpu.set_pc(CODE);
        cpu.set_sp(STACK);
        cpu.set_reg(5, 0x1234_5678_9abc_def0);

        let mut ctx = super::CpuContext {
            regs: [0; 31],
            vectors: [0; 64],
            sp: 0,
            fpcr: 0,
            fpsr: 0,
            pstate: 0,
        };
        cpu.swap_context(&mut ctx);
        assert_eq!(ctx.regs[5], 0x1234_5678_9abc_def0);
        assert_eq!(ctx.sp, STACK);
        // The CPU should now be at reset state; restore and check.
        cpu.set_reg(5, 0);
        cpu.swap_context(&mut ctx);
        assert_eq!(cpu.get_reg(5), 0x1234_5678_9abc_def0);
        assert_eq!(cpu.sp(), STACK);
    }
}
