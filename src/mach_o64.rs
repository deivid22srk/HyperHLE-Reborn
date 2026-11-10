/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Mach-O loader for 64-bit (arm64) executables.
//!
//! This is the F1 loader groundwork (PROGRESS.md P2). It parses the Mach-O
//! header and load commands of a 64-bit little-endian image (thin or fat),
//! refuses anything that must not run (wrong architecture, FairPlay
//! encryption, big-endian garbage), maps the segments into a
//! [`crate::cpu::a64::Mem64`], and returns the entry point from `LC_MAIN`
//! (falling back to `LC_UNIXTHREAD`).
//!
//! It deliberately does NOT touch the 32-bit loader (`src/mach_o.rs`) and
//! does not resolve imports yet: bind processing (chained fixups for iOS 15+
//! images, classic `LC_DYLD_INFO` binds for older ones) is the next slice.
//! The formats were reverse-engineered with the clean-room scripts in
//! `dev-scripts/analyze_ipa/` against real App Store binaries.
//!
//! References (behavior only, per the project's clean-room policy):
//! * `<mach-o/loader.h>` structures (mach_header_64, segment_command_64)
//! * <https://developer.apple.com/documentation/bundleresources/…>

use crate::cpu::a64::{Mem64, VAddr};

/// Mach-O magic numbers, see `<mach-o/loader.h>`.
const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_CIGAM_64: u32 = 0xcffa_edfe;
const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;

const CPU_TYPE_ARM64: i32 = 0x0100_000c;

const MH_EXECUTE: u32 = 2;
const MH_DYLIB: u32 = 6;

const LC_SEGMENT_64: u32 = 0x19;
const LC_UNIXTHREAD: u32 = 0x5;
const LC_MAIN: u32 = 0x28 | 0x8000_0000;
const LC_BUILD_VERSION: u32 = 0x32;
const LC_VERSION_MIN_IPHONEOS: u32 = 0x25;
const LC_ENCRYPTION_INFO_64: u32 = 0x2c;

/// arm64 thread state flavor for `LC_UNIXTHREAD` (`ARM_THREAD_STATE64`).
const ARM_THREAD_STATE64: u32 = 6;

/// Why an image cannot be loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadError {
    /// Not a Mach-O file at all.
    NotMachO,
    /// Big-endian image (unsupported byte order).
    ByteSwapped,
    /// A thin image for a CPU type other than arm64, or a fat image with no
    /// arm64 slice.
    NotArm64,
    /// Wrong file type (only MH_EXECUTE and MH_DYLIB are handled).
    UnsupportedFileType(u32),
    /// FairPlay encryption: the emulator cannot execute encrypted code.
    FairPlayEncrypted,
    /// A load command is malformed (truncated, bad size, out of range).
    Malformed(&'static str),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotMachO => write!(f, "not a Mach-O image"),
            LoadError::ByteSwapped => write!(f, "big-endian Mach-O images are not supported"),
            LoadError::NotArm64 => write!(f, "image is not arm64"),
            LoadError::UnsupportedFileType(t) => {
                write!(f, "unsupported Mach-O file type {t}")
            }
            LoadError::FairPlayEncrypted => write!(
                f,
                "executable is FairPlay encrypted (cryptid != 0); \
                 obtain a decrypted copy"
            ),
            LoadError::Malformed(what) => write!(f, "malformed Mach-O: {what}"),
        }
    }
}

/// A parsed 64-bit segment, from `LC_SEGMENT_64`.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment64 {
    pub name: String,
    pub vmaddr: VAddr,
    pub vmsize: u64,
    pub fileoff: u64,
    pub filesize: u64,
}

/// A parsed arm64 Mach-O image.
#[derive(Debug, Clone, PartialEq)]
pub struct MachO64 {
    pub filetype: u32,
    /// Minimum iOS version from `LC_BUILD_VERSION`/`LC_VERSION_MIN_IPHONEOS`,
    /// encoded like Apple does: major << 16 | minor << 8 | patch.
    pub minos: Option<u32>,
    /// FairPlay state from `LC_ENCRYPTION_INFO_64`.
    pub cryptid: u32,
    /// Linked dylibs (install names from `LC_LOAD_DYLIB` and friends).
    pub dylibs: Vec<String>,
    pub segments: Vec<Segment64>,
    /// Entry point file offset, from `LC_MAIN` (executables).
    pub entry_off: Option<u64>,
    /// Entry PC from `LC_UNIXTHREAD` state, if present (dylibs have none).
    pub thread_entry: Option<VAddr>,
}

fn u16_at(bytes: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([bytes[off], bytes[off + 1]])
}
fn u32_at(bytes: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(bytes[off..off + 8].try_into().unwrap())
}

/// Parse the header + load commands of a thin 64-bit little-endian image.
///
/// `base` is the offset of the thin image inside the containing file (0 for
/// thin files), so all file offsets in load commands must be rebased.
fn parse_thin(bytes: &[u8], base: usize) -> Result<MachO64, LoadError> {
    if bytes.len() < 32 {
        return Err(LoadError::NotMachO);
    }
    let magic = u32_at(bytes, base);
    match magic {
        MH_MAGIC_64 => {}
        MH_CIGAM_64 => return Err(LoadError::ByteSwapped),
        _ => return Err(LoadError::NotMachO),
    }
    let cputype = u32_at(bytes, base + 4) as i32;
    let filetype = u32_at(bytes, base + 12);
    let ncmds = u32_at(bytes, base + 16);
    let sizeofcmds = u32_at(bytes, base + 20);
    if cputype != CPU_TYPE_ARM64 {
        return Err(LoadError::NotArm64);
    }
    if !matches!(filetype, MH_EXECUTE | MH_DYLIB) {
        return Err(LoadError::UnsupportedFileType(filetype));
    }
    let cmds_end = base + 32 + sizeofcmds as usize;
    if cmds_end > bytes.len() {
        return Err(LoadError::Malformed("load commands exceed file size"));
    }

    let mut image = MachO64 {
        filetype,
        minos: None,
        cryptid: 0,
        dylibs: Vec::new(),
        segments: Vec::new(),
        entry_off: None,
        thread_entry: None,
    };

    let mut off = base + 32;
    for _ in 0..ncmds {
        if off + 8 > base + 32 + sizeofcmds as usize {
            return Err(LoadError::Malformed("load command overruns its space"));
        }
        let cmd = u32_at(bytes, off);
        let cmdsize = u32_at(bytes, off + 4) as usize;
        if cmdsize < 8 || off + cmdsize > base + 32 + sizeofcmds as usize {
            return Err(LoadError::Malformed("bad load command size"));
        }
        match cmd {
            LC_SEGMENT_64 => {
                if cmdsize < 72 {
                    return Err(LoadError::Malformed("LC_SEGMENT_64 too small"));
                }
                let name = String::from_utf8_lossy(&bytes[off + 8..off + 24])
                    .trim_end_matches('\0')
                    .to_string();
                let vmaddr = u64_at(bytes, off + 24);
                let vmsize = u64_at(bytes, off + 32);
                let fileoff = u64_at(bytes, off + 40) as usize + base;
                let filesize = u64_at(bytes, off + 48);
                if fileoff + filesize as usize > bytes.len() {
                    return Err(LoadError::Malformed("segment data exceeds file size"));
                }
                image.segments.push(Segment64 {
                    name,
                    vmaddr,
                    vmsize,
                    fileoff: fileoff as u64,
                    filesize,
                });
            }
            LC_MAIN => {
                if cmdsize < 24 {
                    return Err(LoadError::Malformed("LC_MAIN too small"));
                }
                image.entry_off = Some(u64_at(bytes, off + 8));
            }
            LC_UNIXTHREAD => {
                // flavor(4) count(4) then `count` u32 state words. For
                // ARM_THREAD_STATE64 the PC is state word 32 (x0..x30 are
                // 0..30, pc is 32 after sp=31 — see mach/thread_status.h).
                let flavor = u32_at(bytes, off + 8);
                let count = u32_at(bytes, off + 12) as usize;
                if off + 16 + count * 4 > base + 32 + sizeofcmds as usize {
                    return Err(LoadError::Malformed("thread state overruns command"));
                }
                if flavor == ARM_THREAD_STATE64 && count >= 33 {
                    let pc_off = off + 16 + 32 * 4;
                    image.thread_entry = Some(u64_at(bytes, pc_off));
                }
            }
            LC_BUILD_VERSION => {
                if cmdsize >= 24 {
                    image.minos = Some(u32_at(bytes, off + 12));
                }
            }
            LC_VERSION_MIN_IPHONEOS => {
                if image.minos.is_none() && cmdsize >= 16 {
                    image.minos = Some(u32_at(bytes, off + 8));
                }
            }
            LC_ENCRYPTION_INFO_64 => {
                if cmdsize >= 20 {
                    image.cryptid = u32_at(bytes, off + 16);
                }
            }
            // LC_LOAD_DYLIB (0xc), LC_LOAD_WEAK_DYLIB (0x18|REQ),
            // LC_REEXPORT_DYLIB (0x1f|REQ): install name follows the header.
            0xc | 0x8000_0018 | 0x8000_001f => {
                if cmdsize > 24 {
                    let name = &bytes[off + 24..off + cmdsize];
                    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
                    image.dylibs.push(String::from_utf8_lossy(&name[..end]).to_string());
                }
            }
            _ => {}
        }
        off += cmdsize;
    }
    Ok(image)
}

/// Parse a file that may be thin or a fat (universal) binary.
///
/// For fat binaries the arm64 slice is selected; the returned offsets are
/// already rebased so mapping needs no special casing.
pub fn parse(bytes: &[u8]) -> Result<MachO64, LoadError> {
    let magic = if bytes.len() >= 4 {
        u32_at(bytes, 0)
    } else {
        return Err(LoadError::NotMachO);
    };
    match magic {
        MH_MAGIC_64 => parse_thin(bytes, 0),
        MH_CIGAM_64 => Err(LoadError::ByteSwapped),
        // Fat headers are always big-endian, so reading the little-endian
        // first word of a fat file yields the byte-swapped FAT_MAGIC.
        m if m == FAT_MAGIC.swap_bytes() || m == FAT_MAGIC_64.swap_bytes() => {
            if bytes.len() < 8 {
                return Err(LoadError::NotMachO);
            }
            let nfat = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
            let entry_size = if magic == FAT_MAGIC.swap_bytes() { 20 } else { 32 };
            let table_end = 8 + nfat * entry_size;
            if table_end > bytes.len() {
                return Err(LoadError::Malformed("fat archive table exceeds file size"));
            }
            for i in 0..nfat {
                let e = 8 + i * entry_size;
                // Fat arch entries are big-endian too.
                let cputype =
                    u32::from_be_bytes(bytes[e..e + 4].try_into().unwrap()) as i32;
                let offset =
                    u32::from_be_bytes(bytes[e + 8..e + 12].try_into().unwrap()) as usize;
                if cputype == CPU_TYPE_ARM64 {
                    return parse_thin(bytes, offset);
                }
            }
            Err(LoadError::NotArm64)
        }
        _ => Err(LoadError::NotMachO),
    }
}

/// Map the parsed image into guest memory.
///
/// Every page overlapping each segment's `vmaddr..vmaddr + vmsize` is mapped
/// (zero-filled) and the file contents are copied over the first `filesize`
/// bytes, which is exactly how dyld treats `vmsize > filesize` segments (the
/// rest is zero fill for `__bss`). Returns the entry point: `LC_MAIN`'s file
/// offset added to `__TEXT`'s vmaddr for executables, else the
/// `LC_UNIXTHREAD` PC.
pub fn map(mem: &mut Mem64, image: &MachO64, file_bytes: &[u8]) -> Result<VAddr, LoadError> {
    if image.cryptid != 0 {
        return Err(LoadError::FairPlayEncrypted);
    }
    let text_vmaddr = image
        .segments
        .iter()
        .find(|s| s.name == "__TEXT")
        .map(|s| s.vmaddr);
    for seg in &image.segments {
        mem.map_range(seg.vmaddr, seg.vmsize.max(1));
        if seg.filesize > 0 {
            let src = seg.fileoff as usize;
            if src + seg.filesize as usize > file_bytes.len() {
                return Err(LoadError::Malformed("segment data exceeds file size"));
            }
            mem.write_bytes(seg.vmaddr, &file_bytes[src..src + seg.filesize as usize]);
        }
    }
    match (image.entry_off, text_vmaddr) {
        (Some(entry_off), Some(text_vmaddr)) => Ok(text_vmaddr + entry_off),
        _ => image.thread_entry.ok_or(LoadError::Malformed(
            "no LC_MAIN/LC_UNIXTHREAD entry point found",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::a64::{Cpu, CpuState};

    /// Build a minimal but realistic arm64 executable: 4 GiB __PAGEZERO,
    /// __TEXT at 0x100000000 with a tiny program, LC_MAIN, LC_BUILD_VERSION,
    /// LC_ENCRYPTION_INFO_64 (cryptid 0). This mirrors the layout of real
    /// modern App Store executables (see PROGRESS.md, the SubwaySurf stub).
    fn synthetic_executable(cryptid: u32) -> Vec<u8> {
        let mut b = Vec::new();
        let prog: [u32; 3] = [
            0xD280_00E0, // movz x0, #7
            0x9100_8C01, // add x1, x0, #35
            0xD400_1001, // svc #0x80
        ];
        let ncmds = 5u32;
        // mach_header_64
        b.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        b.extend_from_slice(&CPU_TYPE_ARM64.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // cpusubtype
        b.extend_from_slice(&MH_EXECUTE.to_le_bytes());
        b.extend_from_slice(&ncmds.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds, patched later
        b.extend_from_slice(&0u32.to_le_bytes()); // flags
        b.extend_from_slice(&0u32.to_le_bytes()); // reserved

        let mut seg = |name: &str, vmaddr: u64, vmsize: u64, fileoff: u64, filesize: u64, b: &mut Vec<u8>| {
            b.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
            b.extend_from_slice(&72u32.to_le_bytes());
            let mut nm = [0u8; 16];
            nm[..name.len()].copy_from_slice(name.as_bytes());
            b.extend_from_slice(&nm);
            b.extend_from_slice(&vmaddr.to_le_bytes());
            b.extend_from_slice(&vmsize.to_le_bytes());
            b.extend_from_slice(&fileoff.to_le_bytes());
            b.extend_from_slice(&filesize.to_le_bytes());
            b.extend_from_slice(&7u32.to_le_bytes()); // maxprot
            b.extend_from_slice(&5u32.to_le_bytes()); // initprot
            b.extend_from_slice(&0u32.to_le_bytes()); // nsects
            b.extend_from_slice(&0u32.to_le_bytes()); // flags
        };

        // header + 2 segment commands (72 B) + LC_MAIN (24) +
        // LC_BUILD_VERSION (24) + LC_ENCRYPTION_INFO_64 (20).
        let header_and_cmds = 32 + 2 * 72 + 24 + 24 + 20;
        let code_off = header_and_cmds as u64;
        let total_len = header_and_cmds as u64 + prog.len() as u64 * 4;
        // __TEXT covers the whole file from offset 0 (like every real Mach-O:
        // the header itself lives in __TEXT), with vmsize > filesize so the
        // zero-fill path is exercised.
        seg(
            "__TEXT",
            0x1_0000_0000,
            0x3000,
            0,
            total_len,
            &mut b,
        );
        // __DATA: pure bss (filesize 0).
        seg("__DATA", 0x1_0000_c000, 0x1000, total_len, 0, &mut b);
        // LC_MAIN: entry at the code, right after the load commands.
        b.extend_from_slice(&LC_MAIN.to_le_bytes());
        b.extend_from_slice(&24u32.to_le_bytes());
        b.extend_from_slice(&code_off.to_le_bytes());
        b.extend_from_slice(&0u64.to_le_bytes()); // stacksize
        // LC_BUILD_VERSION: iOS 15.0, SDK 15.0.
        b.extend_from_slice(&LC_BUILD_VERSION.to_le_bytes());
        b.extend_from_slice(&24u32.to_le_bytes());
        b.extend_from_slice(&2u32.to_le_bytes()); // platform = iOS
        b.extend_from_slice(&(15u32 << 16).to_le_bytes());
        b.extend_from_slice(&(15u32 << 16).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // ntools
        // LC_ENCRYPTION_INFO_64.
        b.extend_from_slice(&LC_ENCRYPTION_INFO_64.to_le_bytes());
        b.extend_from_slice(&20u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes()); // cryptoff
        b.extend_from_slice(&0u32.to_le_bytes()); // cryptsize
        b.extend_from_slice(&cryptid.to_le_bytes());

        assert_eq!(b.len(), header_and_cmds);
        for w in prog {
            b.extend_from_slice(&w.to_le_bytes());
        }
        // Patch sizeofcmds.
        let sizeofcmds = (b.len() - 32) as u32;
        b[20..24].copy_from_slice(&sizeofcmds.to_le_bytes());
        b
    }

    #[test]
    fn parses_and_reports_minos_and_dylibs() {
        let img = parse(&synthetic_executable(0)).unwrap();
        assert_eq!(img.filetype, MH_EXECUTE);
        assert_eq!(img.minos, Some(15 << 16));
        assert_eq!(img.cryptid, 0);
        assert_eq!(img.segments.len(), 2);
        assert_eq!(img.segments[0].name, "__TEXT");
        assert_eq!(img.segments[0].vmaddr, 0x1_0000_0000);
        assert!(img.dylibs.is_empty());
    }

    #[test]
    fn refuses_fairplay_encrypted_images() {
        let bytes = synthetic_executable(1);
        let img = parse(&bytes).unwrap();
        assert_eq!(img.cryptid, 1);
        let err = map(&mut Mem64::new(), &img, &bytes).unwrap_err();
        assert_eq!(err, LoadError::FairPlayEncrypted);
    }

    #[test]
    fn refuses_non_macho_and_non_arm64() {
        assert_eq!(parse(&[0u8; 64]), Err(LoadError::NotMachO));
        // x86_64 thin image header (cputype 7|0x01000000).
        let mut b = Vec::new();
        b.extend_from_slice(&MH_MAGIC_64.to_le_bytes());
        b.extend_from_slice(&(0x0100_0007u32).to_le_bytes());
        b.extend_from_slice(&[0u8; 24]);
        assert_eq!(parse(&b), Err(LoadError::NotArm64));
    }

    #[test]
    fn selects_the_arm64_slice_of_a_fat_binary() {
        // Build a fat file: x86_64 slice + arm64 slice.
        let arm = synthetic_executable(0);
        let mut x86 = vec![0u8; 32];
        x86[0..4].copy_from_slice(&MH_MAGIC_64.to_le_bytes());
        x86[4..8].copy_from_slice(&(0x0100_0007u32).to_le_bytes());

        let mut fat = Vec::new();
        fat.extend_from_slice(&FAT_MAGIC.to_be_bytes()); // fat headers are BE
        fat.extend_from_slice(&2u32.to_be_bytes());
        // arch entry: cputype, cpusubtype, offset, size, align
        fat.extend_from_slice(&0x0100_0007u32.to_be_bytes());
        fat.extend_from_slice(&0u32.to_be_bytes());
        fat.extend_from_slice(&((8 + 2 * 20) as u32).to_be_bytes());
        fat.extend_from_slice(&(x86.len() as u32).to_be_bytes());
        fat.extend_from_slice(&12u32.to_be_bytes()); // 2^12 align
        fat.extend_from_slice(&CPU_TYPE_ARM64.to_be_bytes());
        fat.extend_from_slice(&0u32.to_be_bytes());
        fat.extend_from_slice(&((8 + 2 * 20 + x86.len()) as u32).to_be_bytes());
        fat.extend_from_slice(&(arm.len() as u32).to_be_bytes());
        fat.extend_from_slice(&12u32.to_be_bytes());
        fat.extend_from_slice(&x86);
        fat.extend_from_slice(&arm);

        let img = parse(&fat).unwrap();
        assert_eq!(img.filetype, MH_EXECUTE);
        assert_eq!(img.segments[0].name, "__TEXT");
        // The arm64 slice's file offsets were rebased onto the fat file.
        let mut mem = Mem64::new();
        let entry = map(&mut mem, &img, &fat).unwrap();
        // LC_MAIN entryoff + __TEXT vmaddr lands on the first instruction.
        assert_eq!(mem.read_u32(entry), 0xD280_00E0);
    }

    #[test]
    fn maps_segments_and_runs_the_entry_point() {
        let bytes = synthetic_executable(0);
        let img = parse(&bytes).unwrap();
        let mut mem = Mem64::new();
        let entry = map(&mut mem, &img, &bytes).unwrap();

        // Entry from LC_MAIN = __TEXT vmaddr + entryoff, pointing at the code.
        assert_eq!(entry, 0x1_0000_0000 + 244);
        // The program bytes landed in guest memory.
        assert_eq!(mem.read_u32(entry), 0xD280_00E0);
        // __DATA is mapped and zero-filled (vmsize > filesize case).
        assert!(mem.is_mapped(0x1_0000_c000));
        assert_eq!(mem.read_u64(0x1_0000_c000), 0);

        // The loader + CPU integration: run the mapped image to its SVC.
        let mut cpu = Cpu::new();
        cpu.set_pc(entry);
        cpu.set_sp(0x1_0000_c000 + 0x800);
        let mut ticks = 1_000;
        match cpu.run_or_step(&mut mem, Some(&mut ticks)) {
            CpuState::Svc(svc) => assert_eq!(svc, 0x80),
            other => panic!("expected Svc(0x80), got {other:?}"),
        }
        assert_eq!(cpu.get_reg(0), 7);
        assert_eq!(cpu.get_reg(1), 42);
    }
}
