#!/usr/bin/env python3
"""Final measured API surface for the Subway Surfers IPA.

- UnityFramework: imports via LC_DYLD_CHAINED_FIXUPS (zlib-free, symbols
  uncompressed despite symbols_format=1; layout: ordinal 8b, weak bit8,
  symbol offset bits 9-31).
- SubwaySurfers.dylib + SubwaySurf: classic undefined symbols in LC_SYMTAB
  (n_type == N_EXT|N_UNDF == 0x01).
"""
import struct
import sys
import json
import re
from collections import Counter

LC_DYLD_CHAINED_FIXUPS = 0x34 | 0x80000000

FAMILIES = [
    (r"^objc_msgSend", "objc_msgSend (dispatch)"),
    (r"^objc_(retains|releases|autorelease|retain|release|storeStrong|loadRetained|loadWeak|copyWeak|initWeak|destroyAutoreleasePool|autoreleasepoolPush|autoreleasepoolPop|beginAutoreleasePool|endAutoreleasePool|isARC|getClass|allocateClassPair|registerClassPair|addMethod|addIvar|addProperty|setAssociatedObject|getAssociatedObject|copyClassList|lookUpClass|getProtocol|allocateProtocol|registerProtocol|adoptProtocol|copyProtocolList|getProperty|copyImageName|enumerateClasses|copyClassNamesForImage|sel_registerName|sel_getName|registerName)", "Objective-C runtime (objc4)"),
    (r"^_OBJC_CLASS_\$_", "ObjC classes referenced"),
    (r"^_OBJC_METACLASS_\$_", "ObjC metaclasses"),
    (r"^_OBJC_PROTOCOL_\$_", "ObjC protocols"),
    (r"^\$s|^_$s", "Swift runtime (mangled)"),
    (r"^swift", "Swift runtime (C API)"),
    (r"^_?dispatch_", "libdispatch"),
    (r"^__?cxa_", "libc++abi (exceptions)"),
    (r"^_ZT[vVSiIlL]?[A-Z]|^_ZTv0|^_ZTI", "C++ typeinfo/vtables"),
    (r"^_ZSt", "C++ stdlib (libc++)"),
    (r"^_Z", "C++ mangled (engine/app)"),
    (r"^pthread_|^_pthread_", "pthread"),
    (r"^mach_|^vm_[a-z]|^mach_host|^mach_msg", "Mach kernel APIs"),
    (r"^CF[A-Z]", "CoreFoundation C API"),
    (r"^os_(log|fault|error|release|retain|atomic|unfair|signpost)", "os APIs"),
    (r"^AudioUnit|^AudioConverter|^AudioFile|^AudioQueue|^ExtAudio|^AudioSession|^AudioComponent|^AudioObject", "AudioToolbox"),
    (r"^Sec[A-Z]|^kSec", "Security/Keychain"),
    (r"^vDSP|^vImage|^cblas|^vMath|^vv", "Accelerate"),
    (r"^AVA", "AVFoundation C symbols"),
    (r"^NSConcrete", "Blocks runtime"),
    (r"^_block", "Blocks runtime"),
    (r"^GLES|^gl[A-Z]|^EAGL", "OpenGL ES / EAGL"),
    (r"^MTL", "Metal C symbols"),
]


def classify(name):
    for pat, fam in FAMILIES:
        if re.search(pat, name):
            return fam
    return "other C/system"


def imports_from_chained_fixups(data):
    ncmds, = struct.unpack_from("<I", data, 16)
    off = 32
    cf_off = None
    for _ in range(ncmds):
        cmd, cmdsize = struct.unpack_from("<II", data, off)
        if cmd == LC_DYLD_CHAINED_FIXUPS:
            cf_off, = struct.unpack_from("<I", data, off + 8)
        off += cmdsize
    if cf_off is None:
        return None
    h = struct.unpack_from("<IIIIII", data, cf_off)
    _, _, imports_off, symbols_off, imports_count, sym_fmt = h
    strblob = data[cf_off + symbols_off:]
    names = []
    for i in range(imports_count):
        e = struct.unpack_from("<I", data, cf_off + imports_off + i * 4)[0]
        weak = (e >> 8) & 1
        so = (e >> 9) & 0x7FFFFF
        end = strblob.find(b"\x00", so)
        if end > so and end - so < 400:
            names.append(strblob[so:end].decode(errors="replace"))
    return names


def imports_from_symtab(data):
    ncmds, = struct.unpack_from("<I", data, 16)
    off = 32
    symtab = None
    for _ in range(ncmds):
        cmd, cmdsize = struct.unpack_from("<II", data, off)
        if cmd == 0x02:
            symoff, nsyms, stroff, strsize = struct.unpack_from("<IIII", data, off + 8)
            symtab = (symoff, nsyms, stroff, strsize)
        off += cmdsize
    if not symtab:
        return []
    symoff, nsyms, stroff, strsize = symtab
    strtab = data[stroff:stroff + strsize]
    names = []
    for i in range(nsyms):
        n_strx, n_type = struct.unpack_from("<IB", data, symoff + i * 16)
        # undefined external: N_TYPE bits clear, N_EXT set
        if (n_type & 0x0E) == 0 and (n_type & 0x01) == 0x01:
            end = strtab.find(b"\0", n_strx)
            if end > n_strx:
                names.append(strtab[n_strx:end].decode(errors="replace"))
    return names


def report(path, label):
    with open(path, "rb") as f:
        data = f.read()
    names = imports_from_chained_fixups(data)
    method = "chained fixups"
    if names is None:
        names = imports_from_symtab(data)
        method = "symtab"
    fams = Counter(classify(n) for n in names)
    print(f"\n=== {label}: {len(names)} imports ({method}) ===")
    for fam, cnt in fams.most_common():
        print(f"  {fam}: {cnt}")
    cls = sorted(set(n for n in names if n.startswith("_OBJC_CLASS_$_")))
    mtl_cls = [c for c in cls if "MTL" in c or "CAMetal" in c]
    print(f"  ObjC classes referenced: {len(cls)} total; Metal-related: {len(mtl_cls)}")
    if mtl_cls:
        print("    " + ", ".join(sorted(set(mtl_cls))))
    return {"label": label, "total": len(names), "method": method,
            "families": dict(fams), "objc_classes": cls}


if __name__ == "__main__":
    # Usage: api_surface.py <bin1> [bin2 ...] [--json OUT.json]
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    json_out = None
    if "--json" in sys.argv:
        json_out = sys.argv[sys.argv.index("--json") + 1]
    out = []
    for p in args:
        out.append(report(p, p.rsplit("/", 1)[-1]))
    if json_out:
        with open(json_out, "w") as f:
            json.dump(out, f, indent=1)
        print("\nJSON written to", json_out)
