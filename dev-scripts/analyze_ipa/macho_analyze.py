#!/usr/bin/env python3
"""Mach-O reconnaissance for the Subway Surfers IPA (Fase 0).

Parses header + load commands of thin/fat binaries. Read-only analysis,
prints: arch, cryptid (encryption), min OS/SDK, linked dylibs, segments,
and flags useful for emulator planning. No Apple/game code is copied.
"""
import struct
import sys
import json

LC_REQ_DYLD = 0x80000000
LC_NAMES = {
    0x01: "LC_SEGMENT", 0x02: "LC_SYMTAB", 0x03: "LC_SYMSEG", 0x04: "LC_THREAD",
    0x05: "LC_UNIXTHREAD", 0x0B: "LC_DYSYMTAB", 0x0C: "LC_LOAD_DYLIB",
    0x0D: "LC_ID_DYLIB", 0x0E: "LC_LOAD_WEAK_DYLIB",
    0x10: "LC_PREBOUND_DYLIB", 0x11: "LC_ROUTINES", 0x12: "LC_SUB_FRAMEWORK",
    0x13: "LC_SUB_UMBRELLA", 0x14: "LC_SUB_CLIENT", 0x15: "LC_SUB_LIBRARY",
    0x16: "LC_TWOLEVEL_HINTS", 0x17: "LC_PREBIND_CKSUM",
    0x18 | LC_REQ_DYLD: "LC_LOAD_WEAK_DYLIB",
    0x19: "LC_SEGMENT_64", 0x1A: "LC_ROUTINES_64", 0x1B: "LC_UUID",
    0x1C | LC_REQ_DYLD: "LC_RPATH", 0x1D: "LC_CODE_SIGNATURE",
    0x1E: "LC_SEGMENT_SPLIT_INFO", 0x1F | LC_REQ_DYLD: "LC_REEXPORT_DYLIB",
    0x20: "LC_LAZY_LOAD_DYLIB", 0x21: "LC_ENCRYPTION_INFO",
    0x22: "LC_DYLD_INFO", 0x22 | LC_REQ_DYLD: "LC_DYLD_INFO_ONLY",
    0x23 | LC_REQ_DYLD: "LC_LOAD_UPWARD_DYLIB", 0x24: "LC_VERSION_MIN_MACOSX",
    0x25: "LC_VERSION_MIN_IPHONEOS", 0x26: "LC_FUNCTION_STARTS",
    0x27: "LC_DYLD_ENVIRONMENT", 0x28 | LC_REQ_DYLD: "LC_MAIN",
    0x29: "LC_DATA_IN_CODE", 0x2A: "LC_SOURCE_VERSION",
    0x2B: "LC_DYLIB_CODE_SIGN_DRS", 0x2C: "LC_ENCRYPTION_INFO_64",
    0x2D: "LC_LINKER_OPTION", 0x2E: "LC_LINKER_OPTIMIZATION_HINT",
    0x2F: "LC_VERSION_MIN_TVOS", 0x30: "LC_VERSION_MIN_WATCHOS",
    0x31: "LC_NOTE", 0x32: "LC_BUILD_VERSION",
    0x33 | LC_REQ_DYLD: "LC_DYLD_EXPORTS_TRIE",
    0x34 | LC_REQ_DYLD: "LC_DYLD_CHAINED_FIXUPS",
    0x35 | LC_REQ_DYLD: "LC_FILESET_ENTRY",
}
CPU_TYPES = {7: "x86", 7 | 0x01000000: "x86_64", 12: "arm",
             12 | 0x01000000: "arm64", 12 | 0x02000000: "arm64_32"}
FLAGS_INTERESTING = {
    0x1: "MH_NOUNDEFS", 0x2: "MH_INCRLINK", 0x4: "MH_DYLDLINK",
    0x8: "MH_BINDATLOAD", 0x10: "MH_PREBOUND", 0x20: "MH_SPLIT_SEGS",
    0x20000: "MH_TWOLEVEL", 0x800000: "MH_PIE", 0x4000000: "MH_NO_HEAP_EXECUTION",
    0x8000000: "MH_HAS_OBJC", 0x20000000: "MH_DYLIB_IN_CACHE",
}


def read_lc(dyldata, off):
    cmd, cmdsize = struct.unpack_from("<II", dyldata, off)
    name = LC_NAMES.get(cmd, f"LC_{cmd:#x}")
    return cmd, cmdsize, name


def analyze_thin(data, label):
    magic = struct.unpack_from("<I", data, 0)[0]
    if magic == 0xFEEDFACF:  # 64-bit LE
        cputype, cpusubtype, filetype, ncmds, sizeofcmds, flags, reserved = \
            struct.unpack_from("<iiIIIII", data, 4)
        is64 = True
        header_size = 32
    elif magic == 0xFEEDFACE:  # 32-bit LE
        cputype, cpusubtype, filetype, ncmds, sizeofcmds, flags = \
            struct.unpack_from("<iiIIII", data, 4)
        is64 = False
        header_size = 28
    else:
        print(f"  [{label}] NOT a Mach-O (magic {magic:#x})")
        return None

    out = {
        "label": label,
        "cputype": CPU_TYPES.get(cputype, hex(cputype)),
        "cputype_raw": cputype, "filetype": filetype,
        "ncmds": ncmds, "flags_hex": hex(flags),
        "flags": [n for b, n in FLAGS_INTERESTING.items() if flags & b],
        "minos": None, "sdk": None, "cryptid": None, "crypt_off": None,
        "crypt_size": None, "dylibs": [], "weak_dylibs": [],
        "segments": [], "has_chained_fixups": False, "has_dyld_info": False,
        "entry_off": None,
    }
    print(f"  [{label}] cputype={out['cputype']} filetype={filetype} "
          f"ncmds={ncmds} flags={out['flags']}")

    off = header_size
    for _ in range(ncmds):
        cmd, cmdsize, name = read_lc(data, off)
        if cmd in (0x19, 0x01):  # LC_SEGMENT(_64)
            if cmd == 0x19:
                segname = data[off + 8:off + 24].split(b"\0")[0].decode()
                vmaddr, vmsize, fileoff, filesize = struct.unpack_from(
                    "<qqQQ", data, off + 24)
                out["segments"].append((segname, vmaddr, vmsize, fileoff, filesize))
                print(f"    seg {segname:<16} vm={vmaddr:#x}+{vmsize:#x}")
        elif cmd == 0x32:  # LC_BUILD_VERSION
            plat, minos, sdk_, = struct.unpack_from("<III", data, off + 8)
            out["minos"], out["sdk"] = minos, sdk_
            print(f"    LC_BUILD_VERSION: platform={plat} minos="
                  f"{minos >> 16}.{(minos >> 8) & 0xFF}.{minos & 0xFF} sdk="
                  f"{sdk_ >> 16}.{(sdk_ >> 8) & 0xFF}.{sdk_ & 0xFF}")
        elif cmd in (0x24, 0x25, 0x2F, 0x30):  # LC_VERSION_MIN_*
            ver, = struct.unpack_from("<I", data, off + 8)
            out["minos"] = ver
            print(f"    {name}: {ver >> 16}.{(ver >> 8) & 0xFF}.{ver & 0xFF}")
        elif cmd in (0x21, 0x2C):  # LC_ENCRYPTION_INFO(_64)
            cryptoff, cryptsize, cryptid = struct.unpack_from("<III", data, off + 8)
            out["cryptid"], out["crypt_off"], out["crypt_size"] = \
                cryptid, cryptoff, cryptsize
            print(f"    {name}: cryptoff={cryptoff} cryptsize={cryptsize} "
                  f"cryptid={cryptid}  <-- 1 = ENCRYPTED (FairPlay)")
        elif cmd in (0x0C, 0x0E, 0x18 | LC_REQ_DYLD, 0x1F | LC_REQ_DYLD,
                     0x20, 0x23 | LC_REQ_DYLD):
            dylib_data = data[off + 24:off + cmdsize]
            dylib = dylib_data.split(b"\0")[0].decode(errors="replace")
            weak = cmd == 0x18 | LC_REQ_DYLD
            (out["weak_dylibs"] if weak else out["dylibs"]).append(dylib)
        elif cmd == 0x34 | LC_REQ_DYLD:
            out["has_chained_fixups"] = True
            print("    LC_DYLD_CHAINED_FIXUPS present (iOS 15+ style binds)")
        elif cmd in (0x22, 0x22 | LC_REQ_DYLD):
            out["has_dyld_info"] = True
        elif cmd == 0x28 | LC_REQ_DYLD:  # LC_MAIN
            entryoff, = struct.unpack_from("<Q", data, off + 8)
            out["entry_off"] = entryoff
        off += cmdsize

    print(f"    dylibs linked: {len(out['dylibs'])} strong, "
          f"{len(out['weak_dylibs'])} weak; chained_fixups="
          f"{out['has_chained_fixups']} dyld_info={out['has_dyld_info']}")
    if out["dylibs"]:
        for d in out["dylibs"]:
            print(f"      + {d}")
    if out["weak_dylibs"]:
        for d in out["weak_dylibs"]:
            print(f"      w {d}")
    return out


def analyze(path):
    with open(path, "rb") as f:
        data = f.read()
    print(f"\n=== {path} ({len(data)} bytes) ===")
    magic = struct.unpack_from("<I", data, 0)[0]
    results = []
    if magic in (0xCAFEBABE, 0xCAFEBABF):  # fat
        narch = struct.unpack_from(">I", data, 4)[0]
        print(f"  FAT binary, {narch} architectures")
        for i in range(narch):
            cputype, cpusub, off_, size, align = struct.unpack_from(
                ">iiIII", data, 8 + i * 20)
            sub = data[off_:off_ + size]
            r = analyze_thin(sub, f"{path}#arch{i}")
            if r:
                results.append(r)
    else:
        r = analyze_thin(data, path)
        if r:
            results.append(r)
    return results


if __name__ == "__main__":
    all_results = []
    for p in sys.argv[1:]:
        all_results.extend(analyze(p))
    with open("macho_report.json", "w") as f:
        json.dump(all_results, f, indent=1, default=str)
    print("\nJSON report written")
