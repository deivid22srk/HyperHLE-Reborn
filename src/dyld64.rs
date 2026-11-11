/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! 64-bit dynamic linking (F1, PROGRESS.md P2): applies Mach-O arm64
//! fixups and hands imports to the HLE layer.
//!
//! Two fixup encodings are supported, matching what the Subway Surfers
//! 3.70.1 binaries actually use (measured with `dev-scripts/analyze_ipa/`
//! against the real files — see PROGRESS.md "Dados do jogo"):
//!
//! * **`LC_DYLD_CHAINED_FIXUPS`** (the stub and UnityFramework, iOS 15+
//!   images): walks the per-page fixup chains of format
//!   `DYLD_CHAINED_PTR_64_OFFSET`, applying rebases as
//!   `load_addr + (high8 << 56 | target)` and binds via the resolver.
//! * **`LC_DYLD_INFO`/`LC_DYLD_INFO_ONLY`** (SubwaySurfers.dylib): the
//!   classic rebase/bind **opcode streams**, interpreted eagerly (lazy
//!   binds are also resolved up front; the HLE layer has no lazy binder).
//!
//! Behavior notes (clean-room, from the public format headers plus
//! empirical validation on the real binaries):
//! * `page_start[]` entries are byte offsets into the page (dyld's own
//!   semantics), and the page-start array begins at offset 22 of
//!   `dyld_chained_starts_in_segment` (right after `page_count`). Both
//!   facts were verified against the real binaries: the stub's __DATA_CONST
//!   chain starts at page byte 136 and decodes to a valid rebase; reading
//!   offset 24 (the imports table) yields garbage chains.
//! * `DYLD_CHAINED_PTR_64_OFFSET` rebase targets are offsets from the
//!   image load address; the optional `high8` tags the top byte (TBI), so
//!   the stored value is `load_addr + (high8 << 56 | target)`. Guest code
//!   that dereferences such a pointer relies on top-byte-ignore, which the
//!   64-bit memory model must implement (tracked in PROGRESS.md).
//! * Bind entries store the address returned by the resolver verbatim; the
//!   resolver decides how to treat the bind's embedded/import addends (a
//!   host trampoline ignores them, a data bind adds them).
//! * A handful of chain entries in repacked binaries walk into non-fixup
//!   data (file corruption from the repack tool). These are counted as
//!   anomalies and logged, not fatal — the same policy as dyld's
//!   `Diagnostics` but non-aborting.
//!
//! The module also provides the **A64 HLE call bridge**: imported symbols
//! resolve to trampoline slots that execute `svc #0x81`; when the CPU
//! reports that SVC, the host looks the slot up in
//! [`TrampolinePage::symbol_at`], runs the host implementation with the
//! guest's `x0..x7` arguments, writes the result to `x0` and resumes at
//! `LR` (`x30`). This is the A64 mirror of the A32 `Dyld` mechanism in
//! `src/dyld.rs`.

use crate::cpu::a64::{Mem64, VAddr};
use crate::mach_o64::{self, DyldInfoOps, LoadError, MachO64};

/// SVC immediate that means "host HLE function call" on A64.
/// `#0x80` stays reserved for BSD syscalls (F2), like on the A32 side.
pub const SVC_HLE: u32 = 0x81;

/// Base guest address of the A64 HLE trampoline page. Chosen far above any
/// realistic image mapping (and above the 4 GiB `__PAGEZERO` of PIE apps).
pub const TRAMPOLINE_BASE: VAddr = 0x0000_7000_0000_0000;

/// `svc #imm16` encoding (AArch64): `0xD4000000 | (imm16 << 5) | 1`.
pub const fn encode_svc(imm: u32) -> u32 {
    0xD400_0000 | (imm << 5) | 1
}

/// An imported symbol, from the chained-fixups import table.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    /// Dylib ordinal. `0` = this image, `253`/`254` are the classic special
    /// ordinals (main executable / self); `1..=240` indexes the
    /// `LC_LOAD_DYLIB`-style commands (1-based).
    pub lib_ordinal: u32,
    pub weak: bool,
    pub name: String,
}

/// Who turns an imported symbol into a guest address.
///
/// The concrete F2 implementation will map libc/ObjC/Swift symbols to host
/// functions and known guest images; tests install their own.
pub trait SymbolResolver {
    /// Resolve `symbol` from dylib `lib_ordinal`. The returned address is
    /// stored verbatim; `addend` is provided for context and the resolver
    /// decides whether it applies (host trampolines ignore it, data binds
    /// add it). `None` means unresolved: weak binds store 0 silently,
    /// strong binds store 0 and record an anomaly.
    fn resolve(
        &mut self,
        mem: &mut Mem64,
        lib_ordinal: u32,
        symbol: &str,
        weak: bool,
        addend: i64,
    ) -> Option<VAddr>;
}

/// Allocates `svc`-based trampolines for host-called imports and remembers
/// the symbol behind every slot, so [`SVC_HLE`] traps can be dispatched.
#[derive(Debug, Default)]
pub struct TrampolinePage {
    next_slot: u32,
    symbols: Vec<String>,
}

impl TrampolinePage {
    /// Address of the trampoline for `symbol`, creating it on first use.
    /// The guest jumps here; the slot executes [`encode_svc`]\([`SVC_HLE`]).
    pub fn trampoline_for(&mut self, mem: &mut Mem64, symbol: &str) -> VAddr {
        if let Some(i) = self.symbols.iter().position(|s| s == symbol) {
            return self.slot_addr(i as u32);
        }
        let index = self.next_slot;
        self.next_slot += 1;
        self.symbols.push(symbol.to_string());
        let addr = self.slot_addr(index);
        mem.map_range(addr, 8);
        mem.write_u32(addr, encode_svc(SVC_HLE));
        addr
    }

    pub fn slot_addr(&self, index: u32) -> VAddr {
        TRAMPOLINE_BASE + index as u64 * 8
    }

    /// Symbol of the trampoline the guest is executing at `pc`.
    ///
    /// Dynarmic reports the PC already advanced past the `svc` when the CPU
    /// halts on it, so dispatch tries `pc` first and then `pc - 4`.
    pub fn symbol_for_dispatch(&self, pc: VAddr) -> Option<&str> {
        self.symbol_at(pc).or_else(|| self.symbol_at(pc.wrapping_sub(4)))
    }

    /// Symbol whose trampoline lives exactly at `pc`.
    pub fn symbol_at(&self, pc: VAddr) -> Option<&str> {
        if pc < TRAMPOLINE_BASE || (pc - TRAMPOLINE_BASE) % 8 != 0 {
            return None;
        }
        let index = ((pc - TRAMPOLINE_BASE) / 8) as usize;
        self.symbols.get(index).map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }
}

/// A resolver that binds every import to a logging trap trampoline.
///
/// M1 policy: the game must not crash on unimplemented imports; each trap
/// returns 0 to the guest and the symbol is recorded so the F2 work can be
/// prioritized by what actually gets called (the A64 analogue of the A32
/// `unimplemented_function_stub`, but recorded per trampoline slot).
#[derive(Debug, Default)]
pub struct TrapResolver {
    pub trampolines: TrampolinePage,
    pub bound: Vec<String>,
}

impl SymbolResolver for TrapResolver {
    fn resolve(
        &mut self,
        mem: &mut Mem64,
        lib_ordinal: u32,
        symbol: &str,
        _weak: bool,
        _addend: i64,
    ) -> Option<VAddr> {
        let addr = self.trampolines.trampoline_for(mem, symbol);
        self.bound.push(format!("[lib {lib_ordinal}] {symbol}"));
        Some(addr)
    }
}

/// Counters from a fixup pass, for logs and tests.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct FixupStats {
    pub rebase: usize,
    pub bind: usize,
    /// Fixups that could not be applied (walked into non-fixup data,
    /// unresolved non-weak binds, unknown opcode…). Non-fatal by policy.
    pub anomalies: usize,
    /// Human-readable reasons for `anomalies` (bounded).
    pub anomaly_log: Vec<String>,
}

impl FixupStats {
    fn anomaly(&mut self, msg: String) {
        self.anomalies += 1;
        if self.anomaly_log.len() < 16 {
            self.anomaly_log.push(msg);
        }
    }
}

/// One initializer entry found in an image's sections.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InitSlot {
    /// `__mod_init_func`: the slot holds a 64-bit absolute pointer.
    Pointer(VAddr),
    /// `__init_offsets` (iOS 15+ linker): the slot holds a `u32` offset
    /// from the image load address.
    Offset(VAddr),
}

/// Everything the loader produced for one image.
#[derive(Debug)]
pub struct LoadedImage {
    pub load_addr: VAddr,
    /// The image's preferred load address (`__TEXT` vmaddr).
    pub image_base: VAddr,
    pub entry: Option<VAddr>,
    /// Initializer slots, in link order (see [`init_slots`]).
    pub init_slots: Vec<InitSlot>,
    pub imports: Vec<Import>,
    pub stats: FixupStats,
}

/// Preferred load address of an image: its `__TEXT` vmaddr (0 for dylibs).
pub fn image_base(image: &MachO64) -> VAddr {
    image
        .segments
        .iter()
        .find(|s| s.name == "__TEXT")
        .map(|s| s.vmaddr)
        .unwrap_or(0)
}

/// Initializer slots of an image, in link order.
pub fn init_slots(image: &MachO64) -> Vec<InitSlot> {
    let mut out = Vec::new();
    for section in &image.sections {
        match (section.segment.as_str(), section.name.as_str()) {
            (_, "__mod_init_func") if section.size > 0 => {
                let count = section.size / 8;
                for i in 0..count {
                    out.push(InitSlot::Pointer(section.addr + i * 8));
                }
            }
            ("__TEXT", "__init_offsets") if section.size > 0 => {
                let count = section.size / 4;
                for i in 0..count {
                    out.push(InitSlot::Offset(section.addr + i * 4));
                }
            }
            _ => {}
        }
    }
    out
}

/// Resolve initializer slots to function addresses (applying the slide).
pub fn read_init_functions(
    mem: &mut Mem64,
    slots: &[InitSlot],
    load_addr: VAddr,
    image_base: VAddr,
) -> Vec<VAddr> {
    let slide = load_addr as i64 - image_base as i64;
    slots
        .iter()
        .filter_map(|slot| match slot {
            InitSlot::Pointer(addr) => {
                let raw = mem.read_u64(*addr) as i64;
                (raw != 0).then_some((raw + slide) as VAddr)
            }
            InitSlot::Offset(addr) => {
                let raw = mem.read_u32(*addr) as i64;
                (raw != 0).then_some((load_addr as i64 + raw) as VAddr)
            }
        })
        .collect()
}

/// Parse, map and link one image: the M1 loader pipeline.
///
/// `load_addr` must be where the caller wants the mach header mapped
/// (executables: their `__TEXT` vmaddr, i.e. slide 0; dylibs: caller-chosen).
pub fn load_and_link(
    mem: &mut Mem64,
    file_bytes: &[u8],
    load_addr: VAddr,
    resolver: &mut dyn SymbolResolver,
) -> Result<LoadedImage, LoadError> {
    let image = mach_o64::parse(file_bytes)?;
    // map_with_entry applies the slide to every segment and to the entry
    // point; dylibs have no entry point and map with a dummy entry of 0.
    let (entry, _) = mach_o64::map_with_entry(mem, &image, file_bytes, load_addr)?;
    let entry = (image.filetype != mach_o64::MH_DYLIB).then_some(entry);

    let mut stats = FixupStats::default();
    let mut imports = Vec::new();
    if let Some((dataoff, datasize)) = image.chained_fixups {
        apply_chained_fixups(
            mem,
            &image,
            file_bytes,
            load_addr,
            dataoff,
            datasize,
            resolver,
            &mut stats,
        )?;
        imports = chained_imports(file_bytes, dataoff, datasize)?;
    } else if let Some(ops) = image.dyld_info {
        apply_dyld_info(mem, &image, file_bytes, load_addr, ops, resolver, &mut stats)?;
    }

    Ok(LoadedImage {
        load_addr,
        image_base: image_base(&image),
        entry,
        init_slots: init_slots(&image),
        imports,
        stats,
    })
}

// ---------------------------------------------------------------------------
// Chained fixups
// ---------------------------------------------------------------------------

const DYLD_CHAINED_PTR_START_NONE: u16 = 0xFFFF;
const DYLD_CHAINED_PTR_START_MULTI: u16 = 0x8000;
const DYLD_CHAINED_PTR_64_OFFSET: u16 = 6;

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(b[off..off + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

/// The import table of a chained-fixups payload.
pub fn chained_imports(
    bytes: &[u8],
    dataoff: u32,
    datasize: u32,
) -> Result<Vec<Import>, LoadError> {
    Ok(parse_chained_fixups(bytes, dataoff, datasize)?.imports)
}

struct ChainedFixups<'a> {
    starts_offset: usize,
    imports: Vec<Import>,
    payload: &'a [u8],
}

fn parse_chained_fixups<'a>(
    bytes: &'a [u8],
    dataoff: u32,
    datasize: u32,
) -> Result<ChainedFixups<'a>, LoadError> {
    let base = dataoff as usize;
    let end = base + datasize as usize;
    if end > bytes.len() || base + 28 > bytes.len() {
        return Err(LoadError::Malformed("chained fixups payload truncated"));
    }
    let payload = &bytes[base..end];
    let fixups_version = u32_at(payload, 0);
    if fixups_version != 0 {
        return Err(LoadError::Malformed("unknown chained fixups version"));
    }
    let starts_offset = u32_at(payload, 4) as usize;
    let imports_offset = u32_at(payload, 8) as usize;
    let symbols_offset = u32_at(payload, 12) as usize;
    let imports_count = u32_at(payload, 16) as usize;
    let imports_format = u32_at(payload, 20);
    let symbols_format = u32_at(payload, 24);
    if symbols_format != 0 {
        return Err(LoadError::Malformed("compressed chained fixup symbols"));
    }
    let entry_size = match imports_format {
        1 => 4,  // DYLD_CHAINED_IMPORT
        2 => 8,  // DYLD_CHAINED_IMPORT_ADDEND
        3 => 16, // DYLD_CHAINED_IMPORT_ADDEND64
        _ => {
            return Err(LoadError::Malformed("unknown chained imports format"));
        }
    };
    if imports_offset + imports_count * entry_size > payload.len()
        || symbols_offset > payload.len()
    {
        return Err(LoadError::Malformed("chained imports table truncated"));
    }
    let mut imports = Vec::with_capacity(imports_count);
    for i in 0..imports_count {
        let p = imports_offset + i * entry_size;
        let (lib_ordinal, weak, name_offset, _addend) = match imports_format {
            1 => {
                let w = u32_at(payload, p);
                (w & 0xFF, (w >> 8) & 1 != 0, w >> 9, 0i64)
            }
            2 => {
                let w = u32_at(payload, p);
                let addend = u32_at(payload, p + 4) as i32;
                (w & 0xFF, (w >> 8) & 1 != 0, w >> 9, addend as i64)
            }
            _ => {
                let lo = u64_at(payload, p);
                let addend = u64_at(payload, p + 8) as i64;
                (
                    (lo & 0xFFFF) as u32,
                    (lo >> 16) & 1 != 0,
                    ((lo >> 32) & 0xFFFF_FFFF) as u32,
                    addend,
                )
            }
        };
        let sym_start = symbols_offset + name_offset as usize;
        if sym_start >= payload.len() {
            return Err(LoadError::Malformed("import name outside symbols blob"));
        }
        let sym_end = payload[sym_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| sym_start + p)
            .unwrap_or(payload.len());
        let name = String::from_utf8_lossy(&payload[sym_start..sym_end]).into_owned();
        imports.push(Import {
            lib_ordinal,
            weak,
            name,
        });
    }
    Ok(ChainedFixups {
        starts_offset,
        imports,
        payload,
    })
}

/// Apply all chained fixups of an image already mapped at `load_addr`.
#[allow(clippy::too_many_lines)]
pub fn apply_chained_fixups(
    mem: &mut Mem64,
    image: &MachO64,
    file_bytes: &[u8],
    load_addr: VAddr,
    dataoff: u32,
    datasize: u32,
    resolver: &mut dyn SymbolResolver,
    stats: &mut FixupStats,
) -> Result<(), LoadError> {
    let slide64 = load_addr as i64 - image_base(image) as i64;
    let cf = parse_chained_fixups(file_bytes, dataoff, datasize)?;
    let starts_img = cf.starts_offset;
    if starts_img + 4 > cf.payload.len() {
        return Err(LoadError::Malformed("chained fixups starts truncated"));
    }
    let seg_count = u32_at(cf.payload, starts_img) as usize;
    if starts_img + 4 + seg_count * 4 > cf.payload.len() {
        return Err(LoadError::Malformed("chained fixups seg table truncated"));
    }

    for (seg_index, seg) in image.segments.iter().enumerate() {
        if seg_index >= seg_count {
            break;
        }
        let so = u32_at(cf.payload, starts_img + 4 + seg_index * 4) as usize;
        if so == 0 {
            continue; // segment has no fixups
        }
        let p = starts_img + so;
        if p + 24 > cf.payload.len() {
            return Err(LoadError::Malformed("chained fixups segment info truncated"));
        }
        let info_size = u32_at(cf.payload, p) as usize;
        let page_size = u16::from_le_bytes([cf.payload[p + 4], cf.payload[p + 5]]) as u64;
        let pointer_format = u16::from_le_bytes([cf.payload[p + 6], cf.payload[p + 7]]);
        let page_count = u16::from_le_bytes([cf.payload[p + 20], cf.payload[p + 21]]) as u64;
        if page_size == 0 || info_size == 0 || p + info_size > cf.payload.len() {
            return Err(LoadError::Malformed("bad chained fixups segment info"));
        }
        if pointer_format != DYLD_CHAINED_PTR_64_OFFSET {
            stats.anomaly(format!(
                "segment {seg_index} ({}): unsupported pointer format {pointer_format}",
                seg.name
            ));
            continue;
        }
        // dyld_chained_starts_in_segment: size(4) page_size(2)
        // pointer_format(2) segment_offset(8) max_valid_pointer(4)
        // page_count(2) page_start[] — so page starts begin at +22.
        let starts_base = p + 22;
        if starts_base + page_count as usize * 2 > cf.payload.len() {
            return Err(LoadError::Malformed("page starts table truncated"));
        }

        for page in 0..page_count {
            let ps = u16::from_le_bytes([
                cf.payload[starts_base + page as usize * 2],
                cf.payload[starts_base + page as usize * 2 + 1],
            ]);
            if ps == DYLD_CHAINED_PTR_START_NONE {
                continue;
            }
            if ps & DYLD_CHAINED_PTR_START_MULTI != 0 {
                stats.anomaly(format!(
                    "segment {seg_index} page {page}: MULTI chain starts (32-bit format) unsupported"
                ));
                continue;
            }
            // dyld uses the page-start value directly as a BYTE offset
            // within the page (verified on the real binaries: e.g. the stub
            // starts its __DATA_CONST chain at 136 = 0x88, an entry that
            // decodes to a valid rebase; units of 4 or 8 hit dead slots).
            let seg_base = (seg.vmaddr as i64 + slide64) as VAddr;
            let seg_end = seg_base + seg.vmsize;
            let page_start_vm = seg_base + page * page_size;
            let mut cur = page_start_vm + ps as u64;
            let mut visited = 0usize;
            loop {
                // Guard against chains that walk into non-fixup data (repack
                // corruption): bound the walk, require alignment, detect loops.
                visited += 1;
                if visited > 1_000_000 || cur % 8 != 0 || cur + 8 > seg_end {
                    stats.anomaly(format!(
                        "segment {seg_index} page {page}: chain left the fixup region at {cur:#x}"
                    ));
                    break;
                }
                if !mem.is_mapped(cur) {
                    stats.anomaly(format!(
                        "segment {seg_index} page {page}: chain hit unmapped memory at {cur:#x}"
                    ));
                    break;
                }
                let v = mem.read_u64(cur);
                if v == 0 {
                    stats.anomaly(format!(
                        "segment {seg_index} page {page}: chain hit a zero slot at {cur:#x}"
                    ));
                    break;
                }
                let next = (v >> 51) & 0xFFF;
                if v >> 63 != 0 {
                    // bind: ordinal:24, addend:8
                    let ordinal = (v & 0x00FF_FFFF) as usize;
                    let embedded = ((v >> 24) & 0xFF) as i64;
                    match cf.imports.get(ordinal) {
                        Some(imp) => match resolver.resolve(
                            mem,
                            imp.lib_ordinal,
                            &imp.name,
                            imp.weak,
                            embedded,
                        ) {
                            Some(target) => mem.write_u64(cur, target),
                            None => {
                                if imp.weak {
                                    mem.write_u64(cur, 0);
                                } else {
                                    stats.anomaly(format!("unresolved bind: {}", imp.name));
                                    mem.write_u64(cur, 0);
                                }
                            }
                        },
                        None => {
                            stats.anomaly(format!("bind ordinal {ordinal} out of range"));
                            break;
                        }
                    }
                    stats.bind += 1;
                } else {
                    // rebase: target:36, high8:8
                    let target = v & ((1u64 << 36) - 1);
                    let high8 = (v >> 36) & 0xFF;
                    let final_addr = load_addr + ((high8 << 56) | target);
                    mem.write_u64(cur, final_addr);
                    stats.rebase += 1;
                }
                if next == 0 {
                    break;
                }
                cur += next * 4;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Classic LC_DYLD_INFO opcode streams
// ---------------------------------------------------------------------------

const REBASE_OPCODE_DONE: u8 = 0x00;
const REBASE_OPCODE_SET_TYPE_IMM: u8 = 0x10;
const REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x20;
const REBASE_OPCODE_ADD_ADDRESS_ULEB: u8 = 0x30;
const REBASE_OPCODE_DO_REBASE: u8 = 0x40;
const REBASE_OPCODE_DO_REBASE_IMM_TIMES: u8 = 0x50;
const REBASE_OPCODE_DO_REBASE_ULEB_TIMES: u8 = 0x60;
const REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB: u8 = 0x70;

const BIND_OPCODE_MASK: u8 = 0xF0;
const BIND_OPCODE_DONE: u8 = 0x00;
const BIND_OPCODE_SET_DYLIB_ORDINAL_IMM: u8 = 0x10;
const BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB: u8 = 0x20;
const BIND_OPCODE_SET_DYLIB_SPECIAL_IMM: u8 = 0x30;
const BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM: u8 = 0x40;
const BIND_OPCODE_SET_TYPE_IMM: u8 = 0x50;
const BIND_OPCODE_SET_ADDEND_SLEB: u8 = 0x60;
const BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB: u8 = 0x70;
const BIND_OPCODE_ADD_ADDR_ULEB: u8 = 0x80;
const BIND_OPCODE_DO_BIND: u8 = 0x90;
const BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB: u8 = 0xA0;
const BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED: u8 = 0xB0;
const BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB: u8 = 0xC0;
const BIND_OPCODE_THREADED: u8 = 0xD0;

const BIND_SUBOPCODE_THREADED_SET_BIND_ORDINAL_TABLE_SIZE_ULEB: u8 = 0x00;
const BIND_SUBOPCODE_THREADED_APPLY: u8 = 0x01;

/// Interpret the `LC_DYLD_INFO` opcode streams of a mapped image.
///
/// The rebase stream follows the documented rebase opcode semantics; all
/// bind-family streams (bind/weak/lazy) share [`run_bind_stream`]. All
/// streams are applied eagerly.
pub fn apply_dyld_info(
    mem: &mut Mem64,
    image: &MachO64,
    file_bytes: &[u8],
    load_addr: VAddr,
    ops: DyldInfoOps,
    resolver: &mut dyn SymbolResolver,
    stats: &mut FixupStats,
) -> Result<(), LoadError> {
    let slide = load_addr as i64 - image_base(image) as i64;
    let stream_at = |off: u32, size: u32| -> Option<&[u8]> {
        let start = off as usize;
        file_bytes.get(start..start + size as usize)
    };

    if let Some(rebase) = stream_at(ops.rebase_off, ops.rebase_size) {
        run_rebase_stream(mem, image, rebase, slide, stats);
    }
    if let Some(bind) = stream_at(ops.bind_off, ops.bind_size) {
        run_bind_stream(mem, image, bind, slide, resolver, stats);
    }
    if let Some(weak) = stream_at(ops.weak_bind_off, ops.weak_bind_size) {
        run_bind_stream(mem, image, weak, slide, resolver, stats);
    }
    if let Some(lazy) = stream_at(ops.lazy_bind_off, ops.lazy_bind_size) {
        run_bind_stream(mem, image, lazy, slide, resolver, stats);
    }
    Ok(())
}

fn segment_vmaddr(image: &MachO64, index: usize) -> Option<VAddr> {
    image.segments.get(index).map(|s| s.vmaddr)
}

/// Interpret the classic rebase opcode stream (dyld semantics: the cursor
/// is `(segment, offset)`; every `DO_REBASE*` adds the slide to the slot).
fn run_rebase_stream(
    mem: &mut Mem64,
    image: &MachO64,
    stream: &[u8],
    slide: i64,
    stats: &mut FixupStats,
) {
    let mut pos = 0usize;
    let mut seg_index = 0usize;
    let mut address: VAddr = 0;
    let mut rebase_type: u8 = 1; // REBASE_TYPE_POINTER
    let mut done = stream.is_empty();

    while !done && pos < stream.len() {
        let op = stream[pos] & !0x0F;
        let imm = stream[pos] & 0x0F;
        pos += 1;
        match op {
            REBASE_OPCODE_DONE => done = true,
            REBASE_OPCODE_SET_TYPE_IMM => {
                rebase_type = imm;
                if rebase_type != 1 {
                    // Types 2/3 are __TEXT forms that still reduce to +slide
                    // for us; anything else is logged when first seen.
                    stats.anomaly(format!("rebase: type {rebase_type} treated as +slide"));
                }
            }
            REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                seg_index = imm as usize;
                let (off, p) = read_uleb(stream, pos);
                pos = p;
                match segment_vmaddr(image, seg_index) {
                    Some(vm) => address = (vm as i64 + off as i64 + slide) as VAddr,
                    None => {
                        stats.anomaly(format!("rebase: bad segment {seg_index}"));
                        done = true;
                    }
                }
            }
            REBASE_OPCODE_ADD_ADDRESS_ULEB => {
                let (off, p) = read_uleb(stream, pos);
                pos = p;
                address = address.wrapping_add(off);
            }
            REBASE_OPCODE_DO_REBASE => {
                if mem.is_mapped(address) {
                    let old = mem.read_u64(address) as i64;
                    mem.write_u64(address, (old + slide) as u64);
                    stats.rebase += 1;
                } else {
                    stats.anomaly(format!("rebase: {address:#x} unmapped"));
                    done = true;
                    continue;
                }
                address = address.wrapping_add(8);
            }
            REBASE_OPCODE_DO_REBASE_IMM_TIMES => {
                for _ in 0..imm {
                    if !mem.is_mapped(address) {
                        stats.anomaly(format!("rebase: {address:#x} unmapped"));
                        done = true;
                        break;
                    }
                    let old = mem.read_u64(address) as i64;
                    mem.write_u64(address, (old + slide) as u64);
                    stats.rebase += 1;
                    address = address.wrapping_add(8);
                }
            }
            REBASE_OPCODE_DO_REBASE_ULEB_TIMES => {
                let (count, p) = read_uleb(stream, pos);
                pos = p;
                for _ in 0..count {
                    if !mem.is_mapped(address) {
                        stats.anomaly(format!("rebase: {address:#x} unmapped"));
                        done = true;
                        break;
                    }
                    let old = mem.read_u64(address) as i64;
                    mem.write_u64(address, (old + slide) as u64);
                    stats.rebase += 1;
                    address = address.wrapping_add(8);
                }
            }
            REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB => {
                let (count, p) = read_uleb(stream, pos);
                let (skip, p) = read_uleb(stream, p);
                pos = p;
                for _ in 0..count {
                    if !mem.is_mapped(address) {
                        stats.anomaly(format!("rebase: {address:#x} unmapped"));
                        done = true;
                        break;
                    }
                    let old = mem.read_u64(address) as i64;
                    mem.write_u64(address, (old + slide) as u64);
                    stats.rebase += 1;
                    address = address.wrapping_add(8 * (1 + skip));
                }
            }
            _ => {
                stats.anomaly(format!("rebase: unknown opcode {op:#04x}"));
                done = true;
            }
        }
    }
}


/// Interpret the classic bind/lazy-bind/weak-bind opcode stream.
///
/// The state machine mirrors dyld's documented bind semantics exactly
/// (dyld2 `ImageLoaderMachOCompressed::eachBind` and the current dyld's
/// `MachOAnalyzer::forEachBind`, which agree):
///
/// * the cursor is `(segment_index, segment_offset)`, not an absolute
///   address — `ADD_ADDR` and the `DO_BIND*` family advance the offset;
/// * `0xA0` binds once then advances `uleb + 8`, `0xB0` advances
///   `imm*8 + 8`, `0xC0` binds `count` times advancing `skip + 8` each;
/// * `0xD0` selects the **threaded** bind scheme: sub-opcode 0 fills the
///   ordinal table from the following plain `DO_BIND` opcodes (which stop
///   writing), and sub-opcode 1 applies the fixups by walking the chained
///   pointer slots themselves (arm64e-style entries: bit 62 = bind,
///   ordinal in bits 0..16, chain delta in bits 51..62).
///
/// All streams are applied eagerly (including lazy binds).
fn run_bind_stream(
    mem: &mut Mem64,
    image: &MachO64,
    stream: &[u8],
    slide: i64,
    resolver: &mut dyn SymbolResolver,
    stats: &mut FixupStats,
) {
    let mut pos = 0usize;
    let mut seg_index: usize = 0;
    let mut seg_offset: u64 = 0;
    let mut have_segment = false;
    let mut symbol = String::new();
    let mut addend: i64 = 0;
    let mut lib_ordinal: u32 = 0;
    let mut done = stream.is_empty();
    // Threaded-bind state.
    let mut threaded = false;
    let mut ordinal_table: Vec<(String, i64, u32)> = Vec::new();

    // Lazy bind streams start at __la_symbol_ptr if they never set a segment.
    if !stream
        .iter()
        .any(|&b| b & !0x0F == BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB)
    {
        if let Some(sec) = image
            .sections
            .iter()
            .find(|s| s.name == "__la_symbol_ptr")
        {
            if let Some(seg_i) = image
                .segments
                .iter()
                .position(|s| s.vmaddr <= sec.addr && sec.addr < s.vmaddr + s.vmsize)
            {
                seg_index = seg_i;
                seg_offset = sec.addr - image.segments[seg_i].vmaddr;
                have_segment = true;
            }
        }
    }

    let slot_addr = |image: &MachO64, seg_index: usize, seg_offset: u64, slide: i64| -> Option<VAddr> {
        let vm = segment_vmaddr(image, seg_index)?;
        Some((vm as i64 + seg_offset as i64 + slide) as VAddr)
    };

    let mut bind_one = |mem: &mut Mem64,
                        image: &MachO64,
                        seg_index: usize,
                        seg_offset: u64,
                        slide: i64,
                        symbol: &str,
                        addend: i64,
                        lib_ordinal: u32,
                        stats: &mut FixupStats| {
        let Some(addr) = slot_addr(image, seg_index, seg_offset, slide) else {
            stats.anomaly(format!("bind: bad segment {seg_index} for {symbol}"));
            return;
        };
        if !mem.is_mapped(addr) {
            stats.anomaly(format!("bind: {symbol} target {addr:#x} is unmapped"));
            return;
        }
        match resolver.resolve(mem, lib_ordinal, symbol, false, addend) {
            Some(target) => mem.write_u64(addr, target),
            None => {
                stats.anomaly(format!("bind: unresolved {symbol}"));
                mem.write_u64(addr, 0);
            }
        }
        stats.bind += 1;
    };

    while !done && pos < stream.len() {
        let op = stream[pos] & BIND_OPCODE_MASK;
        let imm = stream[pos] & 0x0F;
        pos += 1;
        match op {
            BIND_OPCODE_DONE => done = true,
            BIND_OPCODE_SET_DYLIB_ORDINAL_IMM => lib_ordinal = imm as u32,
            BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB => {
                let (o, p) = read_uleb(stream, pos);
                pos = p;
                lib_ordinal = o as u32;
            }
            BIND_OPCODE_SET_DYLIB_SPECIAL_IMM => {
                // Sign-extended 4-bit special ordinal (0 means ordinal 0).
                lib_ordinal = if imm == 0 {
                    0
                } else {
                    (BIND_OPCODE_MASK | imm) as u32
                };
            }
            BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM => {
                let start = pos;
                while pos < stream.len() && stream[pos] != 0 {
                    pos += 1;
                }
                symbol = String::from_utf8_lossy(&stream[start..pos]).into_owned();
                pos += 1; // NUL
            }
            BIND_OPCODE_SET_TYPE_IMM => {
                // BIND_TYPE_POINTER (1) is the only type that makes sense
                // for eager HLE binding; others are ignored.
                let _ = imm;
            }
            BIND_OPCODE_SET_ADDEND_SLEB => {
                let (a, p) = read_sleb(stream, pos);
                pos = p;
                addend = a;
            }
            BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                let (off, p) = read_uleb(stream, pos);
                pos = p;
                seg_index = imm as usize;
                seg_offset = off;
                have_segment = true;
            }
            BIND_OPCODE_ADD_ADDR_ULEB => {
                // ld64 emits negative deltas as 10-byte sleb-style encodings;
                // dyld reads them as u64 and adds with wraparound (identical
                // result), so we do the same.
                let (off, p) = read_uleb(stream, pos);
                pos = p;
                seg_offset = seg_offset.wrapping_add(off);
            }
            BIND_OPCODE_DO_BIND => {
                if threaded {
                    // Threaded mode: plain DO_BINDs fill the ordinal table.
                    ordinal_table.push((symbol.clone(), addend, lib_ordinal));
                } else if have_segment && !symbol.is_empty() {
                    bind_one(mem, image, seg_index, seg_offset, slide, &symbol, addend, lib_ordinal, stats);
                    seg_offset = seg_offset.wrapping_add(8);
                } else {
                    stats.anomaly("bind: DO_BIND without segment/symbol".to_string());
                    done = true;
                }
            }
            BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB => {
                if have_segment && !symbol.is_empty() {
                    bind_one(mem, image, seg_index, seg_offset, slide, &symbol, addend, lib_ordinal, stats);
                    let (delta, p) = read_uleb(stream, pos);
                    pos = p;
                    seg_offset = seg_offset.wrapping_add(delta).wrapping_add(8);
                } else {
                    stats.anomaly("bind: DO_BIND_ADD_ADDR without segment/symbol".to_string());
                    done = true;
                }
            }
            BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED => {
                if have_segment && !symbol.is_empty() {
                    bind_one(mem, image, seg_index, seg_offset, slide, &symbol, addend, lib_ordinal, stats);
                    seg_offset = seg_offset
                        .wrapping_add(imm as u64 * 8)
                        .wrapping_add(8);
                } else {
                    stats.anomaly("bind: DO_BIND_ADD_ADDR_IMM_SCALED without segment/symbol".to_string());
                    done = true;
                }
            }
            BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB => {
                let (count, p) = read_uleb(stream, pos);
                let (skip, p) = read_uleb(stream, p);
                pos = p;
                if !have_segment || symbol.is_empty() {
                    stats.anomaly("bind: DO_BIND_ULEB_TIMES without segment/symbol".to_string());
                    done = true;
                    continue;
                }
                for _ in 0..count {
                    bind_one(mem, image, seg_index, seg_offset, slide, &symbol, addend, lib_ordinal, stats);
                    seg_offset = seg_offset.wrapping_add(skip).wrapping_add(8);
                }
            }
            BIND_OPCODE_THREADED => match imm {
                BIND_SUBOPCODE_THREADED_SET_BIND_ORDINAL_TABLE_SIZE_ULEB => {
                    let (_count, p) = read_uleb(stream, pos);
                    pos = p;
                    ordinal_table.clear();
                    threaded = true;
                }
                BIND_SUBOPCODE_THREADED_APPLY => {
                    if !have_segment {
                        stats.anomaly("bind: THREADED_APPLY without segment".to_string());
                        done = true;
                        continue;
                    }
                    // The chained pointers start at the current cursor and
                    // use the arm64e pointer layout (see fixup-chains.h):
                    // bit63 auth, bit62 bind, ordinal bits 0..16 (binds),
                    // target bits 0..43 + high8 43..51 (rebases), and the
                    // chain delta in bits 51..62, in units of 8 bytes.
                    let mut cursor = seg_offset;
                    loop {
                        let Some(addr) = slot_addr(image, seg_index, cursor, slide) else {
                            stats.anomaly(format!("bind: threaded chain bad slot in seg {seg_index}"));
                            break;
                        };
                        if !mem.is_mapped(addr) {
                            stats.anomaly(format!(
                                "bind: threaded chain hit unmapped memory at {addr:#x}"
                            ));
                            break;
                        }
                        let value = mem.read_u64(addr);
                        let delta = ((value >> 51) & 0x7FF) as u64;
                        let is_bind = (value >> 62) & 1 != 0;
                        if is_bind {
                            let ordinal = (value & 0xFFFF) as usize;
                            match ordinal_table.get(ordinal) {
                                Some((sym, a, lib)) => {
                                    bind_one(mem, image, seg_index, cursor, slide, sym, *a, *lib, stats)
                                }
                                None => {
                                    stats.anomaly(format!(
                                        "bind: threaded ordinal {ordinal} out of range"
                                    ));
                                    break;
                                }
                            }
                        } else {
                            // Threaded rebase: target is a vmaddr.
                            let target = value & ((1u64 << 43) - 1);
                            let high8 = (value >> 43) & 0xFF;
                            let final_addr = image_base(image) as i64 + slide
                                + (((high8 << 56) | target) as i64);
                            mem.write_u64(addr, final_addr as u64);
                            stats.rebase += 1;
                        }
                        if delta == 0 {
                            break;
                        }
                        cursor = cursor.wrapping_add(delta * 8);
                    }
                }
                other => {
                    stats.anomaly(format!("bind: bad threaded sub-opcode {other:#04x}"));
                    done = true;
                }
            },
            _ => {
                stats.anomaly(format!("bind: unknown opcode {op:#04x}"));
                done = true;
            }
        }
    }
}

fn read_uleb(bytes: &[u8], mut pos: usize) -> (u64, usize) {
    let mut result: u64 = 0;
    let mut bit = 0;
    loop {
        let b = *bytes.get(pos).unwrap_or(&0);
        pos += 1;
        if bit < 64 {
            result |= ((b & 0x7F) as u64) << bit;
        }
        // Overlong encodings (corrupt streams) saturate instead of
        // overflowing the shift.
        if b & 0x80 == 0 || bit >= 64 {
            return (result, pos);
        }
        bit += 7;
    }
}

fn read_sleb(bytes: &[u8], mut pos: usize) -> (i64, usize) {
    let mut result: u64 = 0;
    let mut bit = 0;
    loop {
        let b = *bytes.get(pos).unwrap_or(&0);
        pos += 1;
        if bit < 64 {
            result |= ((b & 0x7F) as u64) << bit;
        }
        bit += 7;
        if b & 0x80 == 0 || bit >= 70 {
            break;
        }
    }
    // Sign extend.
    if bit < 64 && (result & (1 << (bit - 1))) != 0 {
        result |= u64::MAX << bit;
    }
    (result as i64, pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::a64::{Cpu, CpuState};
    use crate::mach_o64::{
        CPU_TYPE_ARM64, LC_DYLD_CHAINED_FIXUPS, LC_DYLD_INFO_ONLY, LC_MAIN,
        LC_SEGMENT_64, MH_DYLIB, MH_MAGIC_64, MH_EXECUTE,
    };

    const TEXT_VMADDR: VAddr = 0x1_0000_0000;
    const DATA_CONST_VMADDR: VAddr = 0x1_0000_1000;
    const DATA_VMADDR: VAddr = 0x1_0000_2000;

    /// Encodes `movz xRd, #imm16, lsl #shift*16`.
    fn movz(rd: u32, imm: u64, shift: u32) -> u32 {
        0xD280_0000 | (shift << 21) | (((imm & 0xFFFF) as u32) << 5) | rd
    }
    /// Encodes `movk xRd, #imm16, lsl #shift*16`.
    fn movk(rd: u32, imm: u64, shift: u32) -> u32 {
        0xF280_0000 | (shift << 21) | (((imm & 0xFFFF) as u32) << 5) | rd
    }
    fn ldr_x(rd: u32, rn: u32) -> u32 {
        0xF940_0000 | (rn << 5) | rd
    }
    fn str_x(rd: u32, rn: u32) -> u32 {
        0xF900_0000 | (rn << 5) | rd
    }
    fn blr(rn: u32) -> u32 {
        0xD63F_0000 | (rn << 5)
    }
    fn uleb(v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut v = v;
        loop {
            let b = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
        out
    }

    fn seg_cmd(
        name: &str,
        vmaddr: u64,
        vmsize: u64,
        fileoff: u64,
        filesize: u64,
        sections: &[(&str, u64, u64, u32)], // name, addr, size, flags
    ) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
        let cmdsize = (72 + sections.len() * 80) as u32;
        b.extend_from_slice(&cmdsize.to_le_bytes());
        let mut nm = [0u8; 16];
        nm[..name.len()].copy_from_slice(name.as_bytes());
        b.extend_from_slice(&nm);
        b.extend_from_slice(&vmaddr.to_le_bytes());
        b.extend_from_slice(&vmsize.to_le_bytes());
        b.extend_from_slice(&fileoff.to_le_bytes());
        b.extend_from_slice(&filesize.to_le_bytes());
        b.extend_from_slice(&7u32.to_le_bytes()); // maxprot
        b.extend_from_slice(&5u32.to_le_bytes()); // initprot
        b.extend_from_slice(&(sections.len() as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // flags
        for (sname, addr, size, flags) in sections {
            let mut sn = [0u8; 16];
            sn[..sname.len()].copy_from_slice(sname.as_bytes());
            b.extend_from_slice(&sn);
            b.extend_from_slice(&nm); // segment name again
            b.extend_from_slice(&addr.to_le_bytes());
            b.extend_from_slice(&size.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes()); // offset
            b.extend_from_slice(&0u32.to_le_bytes()); // align
            b.extend_from_slice(&0u32.to_le_bytes()); // reloff
            b.extend_from_slice(&0u32.to_le_bytes()); // nreloc
            b.extend_from_slice(&flags.to_le_bytes());
            b.extend_from_slice(&[0u8; 12]); // reserved1 + reserved2 + reserved3
        }
        b
    }

    fn header(filetype: u32, ncmds: u32) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        b.extend_from_slice(&CPU_TYPE_ARM64.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&filetype.to_le_bytes());
        b.extend_from_slice(&ncmds.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds, patched later
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b
    }

    fn patch_sizeofcmds(b: &mut [u8]) {
        let n = (b.len() - 32) as u32;
        b[20..24].copy_from_slice(&n.to_le_bytes());
    }

    fn bind_word(ordinal: u64, addend: u64, next: u64) -> u64 {
        (1 << 63) | ((next & 0xFFF) << 51) | ((addend & 0xFF) << 24) | (ordinal & 0xFFFFFF)
    }
    fn rebase_word(target: u64, high8: u64, next: u64) -> u64 {
        ((next & 0xFFF) << 51) | ((high8 & 0xFF) << 36) | (target & ((1 << 36) - 1))
    }

    /// Build a chained-fixups executable like the SubwaySurf stub:
    /// `__PAGEZERO` + `__TEXT` (with `code`) + `__DATA_CONST` (with chain
    /// entries at the given in-segment offsets) + fixups payload.
    /// `imports` is (name, weak, ordinal).
    fn chained_executable(
        code: &[u32],
        chain_entries: &[(u64, u64)],
        imports: &[(&str, bool, u32)],
    ) -> Vec<u8> {
        let last_off = chain_entries
            .iter()
            .map(|(off, _)| off + 8)
            .max()
            .unwrap_or(0x200);
        let dc_size = (last_off + 0xFFF) & !0xFFF;

        // Fixups payload (see PROGRESS.md for the measured stub layout).
        let mut payload = Vec::new();
        let starts_offset = 28u32;
        // starts_in_image = 4 + 5 seg offsets; one 24-byte starts_in_segment.
        let imports_offset = starts_offset + (4 + 4 * 5) + 24;
        let symbols_offset = imports_offset + imports.len() as u32 * 4;
        payload.extend_from_slice(&0u32.to_le_bytes()); // fixups_version
        payload.extend_from_slice(&starts_offset.to_le_bytes());
        payload.extend_from_slice(&imports_offset.to_le_bytes());
        payload.extend_from_slice(&symbols_offset.to_le_bytes());
        payload.extend_from_slice(&(imports.len() as u32).to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes()); // DYLD_CHAINED_IMPORT
        payload.extend_from_slice(&0u32.to_le_bytes()); // symbols format
        payload.extend_from_slice(&5u32.to_le_bytes()); // seg_count
        for off in [0u32, 0, 24, 0, 0] {
            payload.extend_from_slice(&off.to_le_bytes()); // only __DATA_CONST
        }
        // dyld_chained_starts_in_segment for __DATA_CONST
        payload.extend_from_slice(&24u32.to_le_bytes()); // size
        payload.extend_from_slice(&0x1000u16.to_le_bytes()); // page size
        payload.extend_from_slice(&DYLD_CHAINED_PTR_64_OFFSET.to_le_bytes());
        payload.extend_from_slice(&(DATA_CONST_VMADDR - TEXT_VMADDR).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes()); // max valid pointer
        payload.extend_from_slice(&1u16.to_le_bytes()); // page count
        payload.extend_from_slice(&0u16.to_le_bytes()); // page 0 starts at 0
        let mut names = Vec::new();
        for (name, weak, ordinal) in imports {
            let name_off = names.len() as u32;
            names.extend_from_slice(name.as_bytes());
            names.push(0);
            let w = (name_off << 9) | ((*weak as u32) << 8) | ordinal;
            payload.extend_from_slice(&w.to_le_bytes());
        }
        payload.extend_from_slice(&names);

        // File layout: header+cmds | code | __DATA_CONST blob | payload.
        let cmds = [
            seg_cmd("__PAGEZERO", 0, 0x1_0000_0000, 0, 0, &[]),
            seg_cmd("__TEXT", TEXT_VMADDR, 0x4000, 0, 0, &[]), // patched
            seg_cmd("__DATA_CONST", DATA_CONST_VMADDR, dc_size, 0, dc_size, &[]),
            seg_cmd("__DATA", DATA_VMADDR, 0x1000, 0, 0, &[]),
            {
                let mut c = Vec::new();
                c.extend_from_slice(&LC_DYLD_CHAINED_FIXUPS.to_le_bytes());
                c.extend_from_slice(&16u32.to_le_bytes());
                c.extend_from_slice(&0u32.to_le_bytes()); // dataoff patched
                c.extend_from_slice(&(payload.len() as u32).to_le_bytes());
                c
            },
            {
                let mut c = Vec::new();
                c.extend_from_slice(&LC_MAIN.to_le_bytes());
                c.extend_from_slice(&24u32.to_le_bytes());
                c.extend_from_slice(&0u64.to_le_bytes()); // entryoff patched
                c.extend_from_slice(&0u64.to_le_bytes());
                c
            },
        ];
        let cmds_len: usize = cmds.iter().map(|c| c.len()).sum();
        let code_off = 32 + cmds_len;
        let dc_off = code_off + code.len() * 4;
        let payload_off = dc_off + dc_size as usize;

        let mut out = header(MH_EXECUTE, cmds.len() as u32);
        for (i, c) in cmds.iter().enumerate() {
            let mut cmd = c.clone();
            match i {
                // __TEXT filesize = header+cmds+code (ends at the blob)
                1 => cmd[48..56].copy_from_slice(&(dc_off as u64).to_le_bytes()),
                // __DATA_CONST fileoff
                2 => cmd[40..48].copy_from_slice(&(dc_off as u64).to_le_bytes()),
                // chained fixups dataoff
                4 => cmd[8..12].copy_from_slice(&(payload_off as u32).to_le_bytes()),
                // LC_MAIN entryoff
                5 => cmd[8..16].copy_from_slice(&(code_off as u64).to_le_bytes()),
                _ => {}
            }
            out.extend_from_slice(&cmd);
        }
        patch_sizeofcmds(&mut out);
        for w in code {
            out.extend_from_slice(&w.to_le_bytes());
        }
        let mut dc = vec![0u8; dc_size as usize];
        for (off, word) in chain_entries {
            dc[*off as usize..*off as usize + 8].copy_from_slice(&word.to_le_bytes());
        }
        out.extend_from_slice(&dc);
        out.extend_from_slice(&payload);
        out
    }

    struct TestResolver {
        trampolines: TrampolinePage,
        data_symbols: std::collections::HashMap<String, VAddr>,
        calls: Vec<String>,
    }

    impl TestResolver {
        fn new(data: &[(&str, VAddr)]) -> TestResolver {
            TestResolver {
                trampolines: TrampolinePage::default(),
                data_symbols: data.iter().map(|(s, a)| (s.to_string(), *a)).collect(),
                calls: Vec::new(),
            }
        }
    }

    impl SymbolResolver for TestResolver {
        fn resolve(
            &mut self,
            mem: &mut Mem64,
            _lib_ordinal: u32,
            symbol: &str,
            _weak: bool,
            addend: i64,
        ) -> Option<VAddr> {
            self.calls.push(symbol.to_string());
            if let Some(addr) = self.data_symbols.get(symbol) {
                // Data binds honor the addend (resolver-side, per contract).
                return Some(addr.wrapping_add(addend as u64));
            }
            Some(self.trampolines.trampoline_for(mem, symbol))
        }
    }

    #[test]
    fn chained_fixups_binds_rebase_and_executes_through_hle() {
        // main: x8 = __DATA_CONST base; ldr x9,[x8]; blr x9; str x0,[sp];
        // svc #0x80.
        let code = [
            movz(8, DATA_CONST_VMADDR, 0),
            movk(8, DATA_CONST_VMADDR >> 16, 1),
            movk(8, DATA_CONST_VMADDR >> 32, 2),
            ldr_x(9, 8),
            blr(9),
            str_x(0, 31), // sp
            0xD400_1001,  // svc #0x80
        ];
        // Two chained binds + one rebase, all on page 0 (page_start = 0).
        let chain = vec![
            (0x00, bind_word(0, 0, 2)),
            // next=14 links on to the rebase slot at 0x40 ((0x40-0x08)/4)
            (0x08, bind_word(1, 0, 14)),
            (0x40, rebase_word(0x1234, 0, 0)),
        ];
        let bytes = chained_executable(
            &code,
            &chain,
            &[("hooked_fn", false, 1), ("weak_data", true, 2)],
        );

        let mut mem = Mem64::new();
        let mut resolver = TestResolver::new(&[("weak_data", 0x12_3456_7800)]);
        let loaded = load_and_link(&mut mem, &bytes, TEXT_VMADDR, &mut resolver).unwrap();

        // Entry points at the code (first instruction is the movz).
        let entry = loaded.entry.unwrap();
        assert_eq!(mem.read_u32(entry), movz(8, DATA_CONST_VMADDR, 0));

        // Binds landed.
        let hook_addr = mem.read_u64(DATA_CONST_VMADDR);
        assert_eq!(resolver.trampolines.symbol_at(hook_addr), Some("hooked_fn"));
        assert_eq!(mem.read_u64(DATA_CONST_VMADDR + 8), 0x12_3456_7800);
        // Rebase stored load_addr + target.
        assert_eq!(mem.read_u64(DATA_CONST_VMADDR + 0x40), TEXT_VMADDR + 0x1234);
        assert_eq!(loaded.stats.rebase, 1);
        assert_eq!(loaded.stats.bind, 2);
        assert_eq!(
            loaded
                .imports
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>(),
            vec!["hooked_fn", "weak_data"]
        );

        // Run to main's final SVC through the HLE call bridge.
        mem.map_range(0x1_0f00_0000, 0x1000); // stack page for `str x0,[sp]`
        let mut cpu = Cpu::new();
        cpu.set_pc(entry);
        cpu.set_sp(0x1_0f00_0000);
        let mut ticks = 10_000u64;
        loop {
            match cpu.run_or_step(&mut mem, Some(&mut ticks)) {
                CpuState::Svc(0x81) => {
                    let sym = resolver.trampolines.symbol_for_dispatch(cpu.pc()).unwrap();
                    assert_eq!(sym, "hooked_fn");
                    cpu.set_reg(0, 42);
                    cpu.set_pc(cpu.get_reg(30)); // LR
                }
                CpuState::Svc(0x80) => break,
                other => panic!("unexpected CPU state {other:?}"),
            }
        }
        assert_eq!(mem.read_u64(0x1_0f00_0000), 42);
        assert_eq!(resolver.calls, vec!["hooked_fn", "weak_data"]);
    }

    #[test]
    fn classic_bind_and_rebase_streams_apply_with_slide() {
        // A dylib (preferred load address 0) mapped at 0x4000_0000.
        // Layout: header | __TEXT cmd | __DATA_CONST cmd | LC_DYLD_INFO_ONLY
        //         | blob (0x1000) | rebase stream | bind stream.
        let blob_off = (32 + 72 + 72 + 48) as u64; // 224

        let mut bind_stream: Vec<u8> = vec![
            0x11, // SET_DYLIB_ORDINAL_IMM 1
            0x40, b'f', b'o', b'o', 0, // SET_SYMBOL_TRAILING "foo"
            0x51, // SET_TYPE_IMM 1
            0x71, // SET_SEGMENT_AND_OFFSET_ULEB seg 1
        ];
        bind_stream.extend_from_slice(&uleb(0x100));
        bind_stream.push(0x60);
        bind_stream.push(0x08); // SET_ADDEND_SLEB 8
        bind_stream.push(0x90); // DO_BIND (address += 8)
        bind_stream.push(0x60);
        bind_stream.push(0x10); // SET_ADDEND_SLEB 0x10
        bind_stream.push(0x90); // DO_BIND
        bind_stream.push(0x00); // DONE

        let mut rebase_stream: Vec<u8> = vec![0x21]; // SET_SEGMENT seg 1
        rebase_stream.extend_from_slice(&uleb(0x200));
        rebase_stream.push(0x60); // DO_REBASE_ULEB_TIMES
        rebase_stream.extend_from_slice(&uleb(2));
        rebase_stream.push(0x00); // DONE

        let mut b = header(MH_DYLIB, 3);
        b.extend_from_slice(&seg_cmd("__TEXT", 0, 0x4000, 0, 0, &[]));
        b.extend_from_slice(&seg_cmd(
            "__DATA_CONST",
            0x4000,
            0x1000,
            blob_off,
            0x1000,
            &[],
        ));
        let streams_off = blob_off as u64 + 0x1000; // streams follow the blob
        b.extend_from_slice(&LC_DYLD_INFO_ONLY.to_le_bytes());
        b.extend_from_slice(&48u32.to_le_bytes());
        b.extend_from_slice(&(streams_off as u32).to_le_bytes()); // rebase_off
        b.extend_from_slice(&(rebase_stream.len() as u32).to_le_bytes());
        b.extend_from_slice(&(streams_off as u32 + rebase_stream.len() as u32).to_le_bytes());
        b.extend_from_slice(&(bind_stream.len() as u32).to_le_bytes());
        for _ in 0..6 {
            b.extend_from_slice(&0u32.to_le_bytes()); // weak/lazy/export
        }
        patch_sizeofcmds(&mut b);
        assert_eq!(b.len() as u64, blob_off);
        // __DATA_CONST blob: pre-rebase values and bind-slot sentinels.
        let mut blob = vec![0u8; 0x1000];
        blob[0x200..0x208].copy_from_slice(&0x2000u64.to_le_bytes());
        blob[0x208..0x210].copy_from_slice(&0x2008u64.to_le_bytes());
        blob[0x100..0x108].copy_from_slice(&0xDEADu64.to_le_bytes());
        blob[0x108..0x110].copy_from_slice(&0xBEEFu64.to_le_bytes());
        b.extend_from_slice(&blob);
        b.extend_from_slice(&rebase_stream);
        b.extend_from_slice(&bind_stream);

        let load_addr = 0x4000_0000;
        let mut mem = Mem64::new();
        let mut resolver = TestResolver::new(&[("foo", 0xAAAA_0000)]);
        let loaded = load_and_link(&mut mem, &b, load_addr, &mut resolver).unwrap();
        assert_eq!(loaded.entry, None); // dylib

        // Rebases applied with the slide.
        assert_eq!(mem.read_u64(0x4000_4000 + 0x200), load_addr + 0x2000);
        assert_eq!(mem.read_u64(0x4000_4000 + 0x208), load_addr + 0x2008);
        // Binds: resolver adds the addend itself (documented contract).
        assert_eq!(mem.read_u64(0x4000_4000 + 0x100), 0xAAAA_0008);
        assert_eq!(mem.read_u64(0x4000_4000 + 0x108), 0xAAAA_0010);
        assert_eq!(loaded.stats.rebase, 2);
        assert_eq!(loaded.stats.bind, 2);
    }

    #[test]
    fn init_slots_are_found_and_resolved() {
        // __DATA/__mod_init_func (2 pointers) + __TEXT/__init_offsets
        // (u32 offsets, one zero to be skipped).
        // Layout: header | __TEXT cmd | __DATA cmd | LC_MAIN | code | data.
        let cmds_len = 152 + 152 + 24;
        let code_off = (32 + cmds_len) as u64; // 360
        let data_off = code_off + 0x1000;

        let mut b = header(MH_EXECUTE, 3);
        b.extend_from_slice(&seg_cmd(
            "__TEXT",
            TEXT_VMADDR,
            0x1000,
            0,
            0x1000,
            // The u32 offsets live at file offset code_off+0x300, which maps
            // to TEXT + code_off + 0x300.
            &[(
                "__init_offsets",
                TEXT_VMADDR + code_off + 0x300,
                8,
                0,
            )],
        ));
        b.extend_from_slice(&seg_cmd(
            "__DATA",
            DATA_VMADDR,
            0x1000,
            data_off,
            0x1000,
            &[("__mod_init_func", DATA_VMADDR, 16, 0x9)],
        ));
        b.extend_from_slice(&LC_MAIN.to_le_bytes());
        b.extend_from_slice(&24u32.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes());
        assert_eq!(b.len() as u64, code_off);
        patch_sizeofcmds(&mut b);
        let mut code = vec![0u8; 0x1000];
        code[0x300..0x304].copy_from_slice(&0x40f0u32.to_le_bytes());
        code[0x304..0x308].copy_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&code);
        let mut data = vec![0u8; 0x1000];
        data[0..8].copy_from_slice(&(TEXT_VMADDR + 0x1000).to_le_bytes());
        data[8..16].copy_from_slice(&(TEXT_VMADDR + 0x1800).to_le_bytes());
        b.extend_from_slice(&data);

        let mut mem = Mem64::new();
        let mut resolver = TestResolver::new(&[]);
        let loaded = load_and_link(&mut mem, &b, TEXT_VMADDR, &mut resolver).unwrap();
        let fns = read_init_functions(
            &mut mem,
            &loaded.init_slots,
            TEXT_VMADDR,
            loaded.image_base,
        );
        // Link order: __TEXT/__init_offsets first, then __DATA pointers.
        assert_eq!(
            fns,
            vec![
                TEXT_VMADDR + 0x40f0,
                TEXT_VMADDR + 0x1000,
                TEXT_VMADDR + 0x1800,
            ]
        );
    }

    #[test]
    fn trampoline_page_round_trip_and_svc_encoding() {
        let mut mem = Mem64::new();
        let mut page = TrampolinePage::default();
        let a = page.trampoline_for(&mut mem, "objc_msgSend");
        let b = page.trampoline_for(&mut mem, "dlopen");
        let a2 = page.trampoline_for(&mut mem, "objc_msgSend");
        assert_eq!(a, a2);
        assert_eq!(b, a + 8);
        assert_eq!(mem.read_u32(a), encode_svc(SVC_HLE));
        assert_eq!(encode_svc(0x80), 0xD400_1001);
        assert_eq!(encode_svc(SVC_HLE), 0xD400_1021);
        assert_eq!(page.symbol_at(a), Some("objc_msgSend"));
        assert_eq!(page.symbol_at(b), Some("dlopen"));
        assert_eq!(page.symbol_at(a + 4), None);
        assert_eq!(page.symbol_at(0x1234), None);
    }

    #[test]
    fn unresolved_weak_bind_stores_zero_without_anomaly() {
        struct NoneResolver;
        impl SymbolResolver for NoneResolver {
            fn resolve(
                &mut self,
                _mem: &mut Mem64,
                _lib: u32,
                _symbol: &str,
                _weak: bool,
                _addend: i64,
            ) -> Option<VAddr> {
                None
            }
        }
        let code = [0xD400_1001];
        let bytes = chained_executable(
            &code,
            &[(0x00, bind_word(0, 0, 0))],
            &[("missing_weak", true, 1)],
        );
        let mut mem = Mem64::new();
        let mut resolver = NoneResolver;
        let loaded = load_and_link(&mut mem, &bytes, TEXT_VMADDR, &mut resolver).unwrap();
        assert_eq!(mem.read_u64(DATA_CONST_VMADDR), 0);
        assert_eq!(loaded.stats.bind, 1);
        assert_eq!(loaded.stats.anomalies, 0);
    }

    /// Real-binary smoke test (ignored by default: needs the IPA on disk).
    /// Point `TOUCHHLE_A64_IPA_APP` at the extracted `SubwaySurf.app` and
    /// run `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn real_subway_surfers_binaries_link_through_the_64_bit_loader() {
        let app = std::env::var("TOUCHHLE_A64_IPA_APP").expect("set TOUCHHLE_A64_IPA_APP");
        let app = std::path::PathBuf::from(app);

        // Stub: chained fixups, 56 binds + 55 rebases (measured), 50 imports.
        let stub = std::fs::read(app.join("SubwaySurf")).unwrap();
        let mut mem = Mem64::new();
        let mut resolver = TrapResolver::default();
        let loaded = load_and_link(&mut mem, &stub, TEXT_VMADDR, &mut resolver).unwrap();
        eprintln!("stub: {:?}", loaded.stats);
        assert_eq!(loaded.imports.len(), 50);
        assert_eq!(loaded.stats.rebase, 55);
        assert_eq!(loaded.stats.bind, 56);
        assert!(loaded.stats.anomalies == 0, "{:?}", loaded.stats.anomaly_log);
        assert!(loaded.entry.is_some());

        // Player: chained fixups, ~916k fixups, 6613 imports (measured).
        let player = std::fs::read(
            app.join("Frameworks/UnityFramework.framework/UnityFramework"),
        )
        .unwrap();
        let mut resolver2 = TrapResolver::default();
        let loaded2 = load_and_link(&mut mem, &player, 0x5000_0000_0000, &mut resolver2).unwrap();
        eprintln!("player: {:?}", loaded2.stats);
        assert_eq!(loaded2.imports.len(), 6613);
        assert!(loaded2.stats.bind + loaded2.stats.rebase > 900_000);
        assert!(loaded2.stats.anomalies <= 64, "{:?}", loaded2.stats.anomaly_log);
        assert_eq!(loaded2.entry, None);

        // Game code: classic LC_DYLD_INFO binds (SubwaySurfers.dylib).
        let game = std::fs::read(app.join("Frameworks/SubwaySurfers.dylib")).unwrap();
        let mut resolver3 = TrapResolver::default();
        let loaded3 = load_and_link(&mut mem, &game, 0x6000_0000_0000, &mut resolver3).unwrap();
        eprintln!("game: {:?}", loaded3.stats);
        // Measured with the canonical interpreter: 5367 binds + 1565 rebases.
        assert_eq!(loaded3.stats.bind, 5367, "{:?}", loaded3.stats);
        assert_eq!(loaded3.stats.rebase, 1565, "{:?}", loaded3.stats);
        assert!(loaded3.stats.anomalies == 0, "{:?}", loaded3.stats.anomaly_log);
    }
}
