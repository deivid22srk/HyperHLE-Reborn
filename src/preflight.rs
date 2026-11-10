/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/.
 */
//! Preflight diagnosis of an app's main executable.
//!
//! Before an app bundle is mapped into guest memory, we can read its main
//! Mach-O header and load commands from the host and explain *why* a bundle
//! cannot run instead of failing deep inside the loader with a message that
//! gives the user nothing to act on.
//!
//! This module deliberately does not touch guest memory and does not depend on
//! the loader, so it can run for `--info` and produce a report even for
//! executables that touchHLE will never be able to map (64-bit ones, for
//! example).
//!
//! Data sources:
//! * Mach-O header and load commands:
//!   <https://developer.apple.com/library/archive/documentation/DeveloperTools/Conceptual/MachOTopics/0-Introduction/introduction.html>
//! * `LC_BUILD_VERSION` / `LC_VERSION_MIN_IPHONEOS`:
//!   <https://developer.apple.com/documentation/bundleresources/information-property-list/minimumosversion>
//! * Emulator capabilities: `src/cpu.rs` (ARMv6/ARMv7-A, 32-bit guest
//!   addresses) and `src/mach_o.rs` (32-bit little-endian ARM only).

/// Mach-O magic numbers, see `<mach-o/loader.h>`.
const MH_MAGIC: u32 = 0xfeed_face;
const MH_CIGAM: u32 = 0xcefa_edfe;
const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_CIGAM_64: u32 = 0xcffa_edfe;
const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;

/// `CPU_TYPE_ARM` (32-bit) and `CPU_TYPE_ARM64`, see `<mach/machine.h>`.
const CPU_TYPE_ARM: u32 = 12;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const CPU_TYPE_ARM64_32: u32 = 0x0200_000c;

/// Load command numbers used below, see `<mach-o/loader.h>`.
///
/// `LC_LOAD_DYLIB` and friends share a layout; the ones with `LC_REQ_DYLD` set
/// exist because dyld treats their absence differently (a weak link may resolve
/// to null). Swift runtime dylibs are weak-linked, so missing this distinction
/// silently drops them from the report.
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_LOAD_WEAK_DYLIB: u32 = 0x18 | LC_REQ_DYLD;
const LC_REEXPORT_DYLIB: u32 = 0x1f | LC_REQ_DYLD;
const LC_LAZY_LOAD_DYLIB: u32 = 0x20 | LC_REQ_DYLD;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x23 | LC_REQ_DYLD;
const LC_REQ_DYLD: u32 = 0x8000_0000;
const LC_ENCRYPTION_INFO: u32 = 0x21;
const LC_VERSION_MIN_IPHONEOS: u32 = 0x25;
const LC_ENCRYPTION_INFO_64: u32 = 0x2c;
const LC_BUILD_VERSION: u32 = 0x32;

/// What the emulator can execute: a 32-bit little-endian ARM Mach-O, as
/// documented in `src/cpu.rs` ("iPhone OS apps used either ARMv6 or ARMv7-A,
/// which are both 32-bit ISAs").
pub const SUPPORTED_MIN_OS_MAJOR: u32 = 2;
pub const SUPPORTED_MAX_OS_MAJOR: u32 = 9;

/// A framework or dylib the executable links against, together with whether
/// the emulator implements it. Only entries that matter for human-readable
/// diagnosis are listed; anything unknown is reported as unverified.
/// A library that needs a note beyond "not implemented", because either it has
/// a partial implementation whose limits matter, or its absence has a
/// consequence the generic message does not convey.
struct KnownLibrary {
    /// Substring matched against the linked path.
    needle: &'static str,
    /// Human-readable reason, printed when the library is linked.
    note: &'static str,
}

/// Notes for libraries whose handling is not obvious from the list of
/// implemented dylibs in `src/dyld/dylib_list.rs`. Everything else is judged
/// against that list, so this table cannot get out of sync with the code.
const LIBRARY_NOTES: &[KnownLibrary] = &[
    KnownLibrary {
        needle: "Metal.framework",
        note: "Metal is only stubbed by src/frameworks/metal.rs: it provides the object and \
               descriptor contract used during startup, but command buffers, pipelines and \
               shaders are not executed. Rendering goes through OpenGL ES 1.x/2.0 (src/gles.rs).",
    },
    KnownLibrary {
        needle: "MetalFX.framework",
        note: "MetalFX (temporal upscaling and denoising) is not implemented and has no OpenGL \
               ES equivalent.",
    },
    KnownLibrary {
        needle: "/swift/",
        note: "The Swift runtime is not implemented; this is a 32-bit Objective-C/C++ environment \
               (Swift did not exist on the iOS versions it targets).",
    },
    KnownLibrary {
        needle: "BackgroundAssets.framework",
        note: "BackgroundAssets is not implemented, so an app that streams its content from a \
               server at first launch cannot get past that step from the IPA alone.",
    },
    KnownLibrary {
        needle: "AVFAudio.framework",
        note: "AVFAudio is not implemented; only the older AVFoundation/OpenAL audio paths are.",
    },
    KnownLibrary {
        needle: "IOSurface.framework",
        note: "IOSurface is not implemented; in practice it is used together with Metal.",
    },
];

/// True when the emulator provides a host implementation for `path`.
///
/// The set of implemented libraries lives in `crate::dyld::DYLIB_LIST`, which is
/// the same list the linker searches, so this stays correct as frameworks are
/// added. Paths match either exactly or through an alias, mirroring how
/// `dyld` resolves them.
fn is_implemented(path: &str) -> bool {
    crate_dylib_list().iter().any(|dylib| {
        dylib.path == path || dylib.aliases.iter().any(|alias| *alias == path)
    })
}

fn crate_dylib_list() -> &'static [&'static crate::dyld::HostDylib] {
    crate::dyld::DYLIB_LIST
}

/// True for libraries that belong to the emulated platform surface, so that
/// "not implemented" is a meaningful thing to say about them. `/usr/lib/*`
/// libraries are deliberately excluded: touchHLE provides those symbols from
/// its own libc/ObjC/C++ shims rather than by loading the dylib, and several of
/// them (`libc++`, `libobjc`) are not in `DYLIB_LIST` at all.
fn is_platform_library(path: &str) -> bool {
    path.starts_with("/System/Library/Frameworks/")
        || path.starts_with("/System/Library/PrivateFrameworks/")
        || path.starts_with("/Library/Frameworks/")
        || path.starts_with("/usr/lib/swift/")
}

/// How severely a finding limits the app.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    /// The bundle cannot be executed at all by this emulator.
    Blocker,
    /// The bundle can be loaded, but the named functionality will be missing.
    Warning,
}

/// One diagnosis line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

/// Facts read from the executable's Mach-O header and load commands.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub is_64bit: bool,
    pub is_big_endian: bool,
    pub cputype: u32,
    pub cpusubtype: u32,
    /// `LC_BUILD_VERSION`/`LC_VERSION_MIN_IPHONEOS` minimum OS, as
    /// `MAJOR.MINOR.PATCH`.
    pub minimum_os: Option<String>,
    /// SDK version from `LC_BUILD_VERSION`, as `MAJOR.MINOR.PATCH`.
    pub sdk: Option<String>,
    /// `cryptid` from `LC_ENCRYPTION_INFO[_64]`; non-zero means the executable
    /// is FairPlay-encrypted and its code is not readable.
    pub cryptid: u32,
    /// Linked libraries (`LC_LOAD_DYLIB` and friends), in load command order.
    pub linked_libraries: Vec<String>,
    /// The subset of [Self::linked_libraries] that is weakly linked. dyld
    /// allows these to be absent, so an unimplemented weak link only matters if
    /// the app actually calls into it.
    pub weak_libraries: Vec<String>,
    pub findings: Vec<Finding>,
}

impl Report {
    /// True when a finding prevents the app from being executed at all.
    pub fn has_blockers(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == Severity::Blocker)
    }

    /// Findings formatted for the log, one line each.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(min_os) = &self.minimum_os {
            out.push(format!("Minimum OS version (from executable): {min_os}"));
        }
        if let Some(sdk) = &self.sdk {
            out.push(format!("Built against SDK version: {sdk}"));
        }
        out.push(format!(
            "CPU: {} (cputype {:#x}, cpusubtype {:#x}), {}-bit",
            cpu_description(self.cputype),
            self.cputype,
            self.cpusubtype & 0x00ff_ffff,
            if self.is_64bit { 64 } else { 32 }
        ));
        if self.cryptid != 0 {
            out.push("Encrypted executable (FairPlay): yes".to_string());
        }
        for finding in &self.findings {
            let prefix = match finding.severity {
                Severity::Blocker => "Blocker",
                Severity::Warning => "Warning",
            };
            out.push(format!("{prefix}: {}", finding.message));
        }
        out
    }

    /// The single most important reason the app cannot run, if there is one.
    /// Used to build a one-line error message at call sites that cannot print
    /// the whole report.
    pub fn summary(&self) -> Option<&str> {
        self.findings
            .iter()
            .find(|f| f.severity == Severity::Blocker)
            .map(|f| f.message.as_str())
    }
}

fn be32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn le32(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

/// Renders an xnu packed version (`A.B.C` in 16/8/8 bits) the way
/// `otool -l` and Xcode do.
fn format_packed_version(packed: u32) -> String {
    format!(
        "{}.{}.{}",
        packed >> 16,
        (packed >> 8) & 0xff,
        packed & 0xff
    )
}

fn cpu_description(cputype: u32) -> &'static str {
    match cputype {
        CPU_TYPE_ARM => "ARM (32-bit)",
        CPU_TYPE_ARM64 => "arm64",
        CPU_TYPE_ARM64_32 => "arm64_32",
        7 => "i386",
        0x0100_0007 => "x86_64",
        _ => "unknown",
    }
}

fn is_arm32(cputype: u32) -> bool {
    cputype == CPU_TYPE_ARM
}

/// Parses `bytes` as a thin Mach-O file and fills in the reported facts.
///
/// Returns `None` if the bytes are not a Mach-O file at all, or are a universal
/// ("fat") file: the loader handles fat selection separately, and mixing the
/// two would report the wrong architecture. `parse_fat` covers that case.
fn parse_thin(bytes: &[u8], report: &mut Report) -> Option<()> {
    let magic = le32(bytes, 0)?;
    let (is_64bit, little_endian) = match magic {
        MH_MAGIC => (false, true),
        MH_MAGIC_64 => (true, true),
        MH_CIGAM => (false, false),
        MH_CIGAM_64 => (true, false),
        _ => return None,
    };
    let read = |offset: usize| -> Option<u32> {
        if little_endian {
            le32(bytes, offset)
        } else {
            be32(bytes, offset)
        }
    };
    report.is_64bit = is_64bit;
    report.is_big_endian = !little_endian;
    report.cputype = read(4)?;
    report.cpusubtype = read(8)?;
    let ncmds = read(16)?;
    let sizeofcmds = read(20)? as usize;
    let header_size: usize = if is_64bit { 32 } else { 28 };
    // A truncated or malformed `sizeofcmds` should still leave the header facts
    // (architecture, bitness) reportable, so clamp instead of giving up: the
    // caller is trying to explain a failure, not to load the file.
    let end = header_size
        .checked_add(sizeofcmds)
        .unwrap_or(usize::MAX)
        .min(bytes.len());

    let mut offset = header_size;
    for _ in 0..ncmds {
        // Every load command starts with `cmd` and `cmdsize`, both `u32`.
        // Truncated command tables are common in half-downloaded IPAs, so stop
        // cleanly rather than reporting the whole file as unparsable.
        if offset + 8 > end {
            break;
        }
        let (Some(cmd), Some(cmdsize)) = (read(offset), read(offset + 4)) else {
            break;
        };
        let cmdsize = cmdsize as usize;
        if cmdsize < 8 || offset + cmdsize > end {
            break;
        }
        let body = offset + 8;
        match cmd {
            LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LAZY_LOAD_DYLIB
            | LC_LOAD_UPWARD_DYLIB => {
                // `dylib_command` embeds a `lc_str` whose offset is relative to
                // the start of the load command.
                if let Some(str_offset) = read(body) {
                    let start = offset.saturating_add(str_offset as usize);
                    if start < offset + cmdsize {
                        let raw = &bytes[start..offset + cmdsize];
                        let name = raw
                            .split(|b| *b == 0)
                            .next()
                            .unwrap_or_default();
                        if let Ok(name) = std::str::from_utf8(name) {
                            let name = name.to_string();
                            if cmd == LC_LOAD_WEAK_DYLIB {
                                report.weak_libraries.push(name.clone());
                            }
                            report.linked_libraries.push(name);
                        }
                    }
                }
            }
            LC_BUILD_VERSION => {
                if let Some(minos) = read(body + 4) {
                    report.minimum_os = Some(format_packed_version(minos));
                }
                if let Some(sdk) = read(body + 8) {
                    report.sdk = Some(format_packed_version(sdk));
                }
            }
            LC_VERSION_MIN_IPHONEOS => {
                if report.minimum_os.is_none() {
                    if let Some(minos) = read(body) {
                        report.minimum_os = Some(format_packed_version(minos));
                    }
                }
            }
            LC_ENCRYPTION_INFO | LC_ENCRYPTION_INFO_64 => {
                if let Some(cryptid) = read(body + 8) {
                    report.cryptid = cryptid;
                }
            }
            _ => (),
        }
        offset += cmdsize;
    }
    Some(())
}

/// Parses a universal ("fat") file far enough to describe its slices. The
/// reported CPU facts come from the first ARM slice if there is one, so that a
/// universal binary containing a 32-bit slice is not misreported as unusable.
fn parse_fat(bytes: &[u8], report: &mut Report) -> Option<()> {
    let magic = be32(bytes, 0)?;
    if magic != FAT_MAGIC && magic != FAT_MAGIC_64 {
        return None;
    }
    let nfat_arch = be32(bytes, 4)? as usize;
    // `fat_header` is 8 bytes; `fat_arch` is 20 bytes (and 32 in `fat_arch_64`).
    let arch_size = if magic == FAT_MAGIC_64 { 32 } else { 20 };
    let mut described = false;
    for index in 0..nfat_arch {
        let base = 8 + index * arch_size;
        let (Some(cputype), Some(cpusubtype)) = (be32(bytes, base), be32(bytes, base + 4)) else {
            break;
        };
        if !described {
            // Fall back to the first slice: if there is no 32-bit ARM slice at
            // all, the report should still name what the binary *is* built for.
            report.cputype = cputype;
            report.cpusubtype = cpusubtype;
            report.is_64bit = cputype == CPU_TYPE_ARM64 || cputype == CPU_TYPE_ARM64_32;
            described = true;
        }
        if is_arm32(cputype) {
            report.cputype = cputype;
            report.cpusubtype = cpusubtype;
            report.is_64bit = false;
            break;
        }
    }
    Some(())
}

fn add_finding(report: &mut Report, severity: Severity, message: impl Into<String>) {
    report.findings.push(Finding {
        severity,
        message: message.into(),
    });
}

/// Reads `bytes` (the app's main executable) and returns a report describing it
/// and everything about it that this emulator cannot provide.
///
/// `plist_minimum_os` is the bundle's `MinimumOSVersion`, used when the
/// executable itself carries no version load command.
pub fn diagnose(executable: &[u8], plist_minimum_os: Option<&str>) -> Report {
    let mut report = Report::default();
    if parse_thin(executable, &mut report).is_none() {
        if parse_fat(executable, &mut report).is_none() {
            add_finding(
                &mut report,
                Severity::Blocker,
                "Executable is not a Mach-O file.",
            );
            return report;
        }
    }

    // Architecture is the first hard gate in `MachO::load_from_bytes`, so
    // report it first and in the same terms.
    if report.cputype != CPU_TYPE_ARM {
        let description = cpu_description(report.cputype);
        add_finding(
            &mut report,
            Severity::Blocker,
            format!(
                "Executable is built for {description}, but this emulator only runs 32-bit ARM \
                 (ARMv6/ARMv7-A) binaries. 64-bit support is not a build option: guest pointers \
                 are 32-bit throughout (see `VAddr` in src/cpu.rs), and the bundled dynarmic is \
                 configured with DYNARMIC_FRONTENDS=A32."
            ),
        );
    } else if report.is_64bit {
        add_finding(
            &mut report,
            Severity::Blocker,
            "Executable uses 64-bit Mach-O structures, which this emulator does not load.",
        );
    }
    if report.is_big_endian {
        add_finding(
            &mut report,
            Severity::Blocker,
            "Executable is big-endian; only little-endian ARM is supported.",
        );
    }
    if report.cryptid != 0 {
        // A warning, not a blocker: some IPAs are decrypted by third-party
        // tools that leave `cryptid` stale, and those run fine. A genuinely
        // encrypted binary will still fail, but now with a named cause.
        add_finding(
            &mut report,
            Severity::Warning,
            "Executable declares FairPlay encryption (LC_ENCRYPTION_INFO cryptid != 0). If the \
             app crashes or executes garbage at startup, the binary is still encrypted and needs \
             to be decrypted on a device you own.",
        );
    }

    if report.minimum_os.is_none() {
        report.minimum_os = plist_minimum_os.map(|s| s.to_string());
    }
    if let Some(min_os) = report.minimum_os.clone() {
        let major = min_os
            .split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        if major > SUPPORTED_MAX_OS_MAJOR {
            // A warning rather than a blocker: some late-32-bit apps still do
            // useful things before reaching an API this emulator lacks, and
            // blocking them here would take away working launches. The
            // architecture and encryption checks above are the real gates.
            add_finding(
                &mut report,
                Severity::Warning,
                format!(
                    "App declares a minimum OS of iOS {min_os}. This emulator implements the \
                     iPhone OS {SUPPORTED_MIN_OS_MAJOR}.x–{SUPPORTED_MAX_OS_MAJOR}.0 era, so newer \
                     frameworks and APIs this app expects are likely missing."
                ),
            );
        }
    }

    // Judge linked libraries against the emulator's own list of implemented
    // dylibs (`crate::dyld::DYLIB_LIST`), plus a small table of notes for the
    // entries whose handling needs explanation.
    let mut messages: Vec<String> = Vec::new();
    for library in &report.linked_libraries {
        if is_platform_library(library) && !is_implemented(library) {
            let weak = report.weak_libraries.contains(library);
            let message = if weak {
                format!(
                    "Weakly links {library}, which this emulator does not implement. dyld would \
                     allow it to be absent, so this only breaks if the app actually uses it."
                )
            } else {
                format!(
                    "Links {library}, which this emulator does not implement \
                     (see src/dyld/dylib_list.rs for the implemented set)."
                )
            };
            if !messages.contains(&message) {
                messages.push(message);
            }
        }
        // Applied whether or not the library is implemented: Metal *is* in
        // DYLIB_LIST, as a stub, and the note is what tells the user what that
        // stub does and does not do.
        if let Some(note) = LIBRARY_NOTES
            .iter()
            .find(|l| library.contains(l.needle))
            .map(|l| l.note)
        {
            if !messages.iter().any(|m| m == note) {
                messages.push(note.to_string());
            }
        }
    }
    for message in messages {
        add_finding(&mut report, Severity::Warning, message);
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal thin Mach-O header followed by `commands`.
    fn thin(cputype: u32, is_64bit: bool, commands: &[u8]) -> Vec<u8> {
        let header_size = if is_64bit { 32 } else { 28 };
        let mut bytes = vec![0u8; header_size];
        let magic = if is_64bit { MH_MAGIC_64 } else { MH_MAGIC };
        bytes[0..4].copy_from_slice(&magic.to_le_bytes());
        bytes[4..8].copy_from_slice(&cputype.to_le_bytes());
        bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2u32.to_le_bytes()); // MH_EXECUTE
        // ncmds and sizeofcmds are filled in after the commands are known.
        let ncmds = count_commands(commands);
        bytes[16..20].copy_from_slice(&ncmds.to_le_bytes());
        bytes[20..24].copy_from_slice(&(commands.len() as u32).to_le_bytes());
        bytes.extend_from_slice(commands);
        bytes
    }

    fn count_commands(commands: &[u8]) -> u32 {
        let mut offset = 0;
        let mut count = 0;
        while offset + 8 <= commands.len() {
            let cmdsize = u32::from_le_bytes([
                commands[offset + 4],
                commands[offset + 5],
                commands[offset + 6],
                commands[offset + 7],
            ]) as usize;
            if cmdsize < 8 {
                break;
            }
            offset += cmdsize;
            count += 1;
        }
        count
    }

    fn build_version(minos: (u32, u32, u32), sdk: (u32, u32, u32)) -> Vec<u8> {
        let mut command = Vec::new();
        command.extend_from_slice(&LC_BUILD_VERSION.to_le_bytes());
        command.extend_from_slice(&24u32.to_le_bytes());
        command.extend_from_slice(&2u32.to_le_bytes()); // platform: iOS
        for (major, minor, patch) in [minos, sdk] {
            command.extend_from_slice(&((major << 16) | (minor << 8) | patch).to_le_bytes());
        }
        command.extend_from_slice(&0u32.to_le_bytes()); // ntools
        assert_eq!(command.len(), 24);
        command
    }

    fn load_dylib(path: &str) -> Vec<u8> {
        dylib_command(LC_LOAD_DYLIB, path)
    }

    fn load_weak_dylib(path: &str) -> Vec<u8> {
        dylib_command(LC_LOAD_WEAK_DYLIB, path)
    }

    fn dylib_command(cmd: u32, path: &str) -> Vec<u8> {
        // `dylib_command` body: name offset, timestamp, current_version,
        // compatibility_version, then the NUL-terminated path.
        let mut body = Vec::new();
        body.extend_from_slice(&24u32.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(path.as_bytes());
        body.push(0);
        while body.len() % 4 != 0 {
            body.push(0);
        }
        let mut command = Vec::new();
        command.extend_from_slice(&cmd.to_le_bytes());
        command.extend_from_slice(&((8 + body.len()) as u32).to_le_bytes());
        command.extend_from_slice(&body);
        command
    }

    #[test]
    fn armv7_ios3_binary_has_no_blockers() {
        let mut commands = load_dylib("/usr/lib/libSystem.B.dylib");
        commands.extend_from_slice(&build_version((3, 0, 0), (4, 3, 0)));
        let bytes = thin(CPU_TYPE_ARM, false, &commands);
        let report = diagnose(&bytes, None);
        assert_eq!(report.minimum_os.as_deref(), Some("3.0.0"));
        assert_eq!(report.sdk.as_deref(), Some("4.3.0"));
        assert!(!report.has_blockers(), "{:?}", report.findings);
        assert_eq!(report.linked_libraries, ["/usr/lib/libSystem.B.dylib"]);
    }

    #[test]
    fn arm64_binary_is_a_blocker_and_names_the_arch() {
        let mut commands = load_dylib("/System/Library/Frameworks/Metal.framework/Metal");
        commands.extend_from_slice(&build_version((17, 0, 0), (17, 5, 0)));
        let bytes = thin(CPU_TYPE_ARM64, true, &commands);
        let report = diagnose(&bytes, Some("17.0"));
        assert!(report.has_blockers());
        let messages: Vec<&str> = report
            .findings
            .iter()
            .map(|f| f.message.as_str())
            .collect();
        assert!(
            messages.iter().any(|m| m.contains("arm64")),
            "should name the architecture: {messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("iOS 17.0.0")),
            "should quote the minimum OS: {messages:?}"
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.severity == Severity::Warning && f.message.contains("Metal")),
            "should warn about Metal: {messages:?}"
        );
    }

    #[test]
    fn implemented_frameworks_are_not_reported_as_missing() {
        let commands = load_dylib("/System/Library/Frameworks/UIKit.framework/UIKit");
        let bytes = thin(CPU_TYPE_ARM, false, &commands);
        let report = diagnose(&bytes, None);
        assert!(report.findings.is_empty(), "{:?}", report.findings);
    }

    #[test]
    fn weak_links_are_recognised_and_reported_separately() {
        // Swift runtime dylibs are weak-linked; see the load command mix of a
        // real iOS 17 binary (11 x LC_LOAD_WEAK_DYLIB, 30 x LC_LOAD_DYLIB).
        let commands = load_weak_dylib("/usr/lib/swift/libswiftCore.dylib");
        let bytes = thin(CPU_TYPE_ARM, false, &commands);
        let report = diagnose(&bytes, None);
        assert_eq!(report.weak_libraries, ["/usr/lib/swift/libswiftCore.dylib"]);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("Weakly links")),
            "{:?}",
            report.findings
        );
    }

    #[test]
    fn encrypted_binary_warns_but_does_not_block() {
        let mut commands = Vec::new();
        commands.extend_from_slice(&LC_ENCRYPTION_INFO.to_le_bytes());
        commands.extend_from_slice(&20u32.to_le_bytes());
        commands.extend_from_slice(&0u32.to_le_bytes()); // cryptoff
        commands.extend_from_slice(&0x1000u32.to_le_bytes()); // cryptsize
        commands.extend_from_slice(&1u32.to_le_bytes()); // cryptid
        let bytes = thin(CPU_TYPE_ARM, false, &commands);
        let report = diagnose(&bytes, None);
        assert_eq!(report.cryptid, 1);
        assert!(!report.has_blockers(), "{:?}", report.findings);
        assert!(report
            .findings
            .iter()
            .any(|f| f.severity == Severity::Warning && f.message.contains("FairPlay")));
    }

    #[test]
    fn fat_binary_with_armv7_slice_is_not_rejected() {
        let slice = thin(CPU_TYPE_ARM, false, &[]);
        let mut bytes = vec![0u8; 8];
        bytes[0..4].copy_from_slice(&FAT_MAGIC.to_be_bytes());
        bytes[4..8].copy_from_slice(&1u32.to_be_bytes());
        let mut arch = Vec::new();
        arch.extend_from_slice(&CPU_TYPE_ARM.to_be_bytes());
        arch.extend_from_slice(&7u32.to_be_bytes());
        arch.extend_from_slice(&(8u32 + 20).to_be_bytes()); // offset
        arch.extend_from_slice(&(slice.len() as u32).to_be_bytes());
        arch.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&arch);
        bytes.extend_from_slice(&slice);
        let report = diagnose(&bytes, None);
        assert!(!report.has_blockers(), "{:?}", report.findings);
        assert_eq!(report.cputype, CPU_TYPE_ARM);
    }

    #[test]
    fn non_macho_is_a_blocker() {
        let report = diagnose(b"#!/bin/sh\necho hello\n", None);
        assert!(report.has_blockers());
    }

    #[test]
    fn truncated_load_commands_do_not_panic() {
        let mut bytes = thin(CPU_TYPE_ARM, false, &[]);
        // Claim way more commands than the file contains.
        bytes[16..20].copy_from_slice(&1000u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&4096u32.to_le_bytes());
        let report = diagnose(&bytes, None);
        // Nothing to report beyond the (correct) architecture.
        assert_eq!(report.cputype, CPU_TYPE_ARM);
    }
}
