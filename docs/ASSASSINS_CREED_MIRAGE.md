# Assassin's Creed Mirage (iOS) — feasibility report

This document records why this app does not run under HyperHLE-Reborn, with the
evidence used to reach that conclusion. It is written so the same analysis can be
repeated for any other bundle: the emulator now does most of it automatically
(see "Preflight" below).

Bundle used for testing: `Assassins-Creed-Mirage-v1.3.0.ipa`
(`com.ubisoft.assassinscreed.mirage.mobile.action.adventure`, 1.3.0).

## What the bundle actually is

| Property | Value | Source |
| --- | --- | --- |
| Executable | `Payload/scimitar.app/scimitar`, 129,368,672 bytes | IPA listing |
| Architecture | `arm64`, 64-bit Mach-O (`MH_MAGIC_64`) | `LC_BUILD_VERSION` / header |
| Minimum OS | 17.4 (executable), 17.0 (`MinimumOSVersion`) | `LC_BUILD_VERSION`, Info.plist |
| SDK | 17.5 | `LC_BUILD_VERSION` |
| FairPlay encryption | **not encrypted** (`cryptid = 0`) | `LC_ENCRYPTION_INFO_64` |
| Required capabilities | `arm64`, `iphone-performance-gaming-tier` | Info.plist |
| Size on disk | 4.2 GB IPA; `Assets.car` alone is 2.2 GB | IPA listing |
| Additional download | `BAEssentialMaxInstallSize` = 7.5 GB (`~ipad`: 9.6 GB) | Info.plist |

The `cryptid = 0` result is the one genuinely good piece of news: the executable
is decrypted, so its code and load commands are readable. Everything else is the
opposite of what this emulator targets.

### Linked frameworks (41 dylibs)

Full list from the load commands (`30 × LC_LOAD_DYLIB`, `11 × LC_LOAD_WEAK_DYLIB`).
The preflight output above lists each one's status; the notable ones are:

- Not implemented at all: `AppTrackingTransparency`, `AuthenticationServices`,
  `CoreHaptics`, `IOSurface`, `MetalFX`, `Network`, `BackgroundAssets`, `AVFAudio`,
  and the Swift runtime (`/usr/lib/swift/*`, 13 dylibs — `libswiftCore`,
  `libswift_Concurrency` and 11 weak-linked ones such as `libswiftMetal`)
- Implemented only as a contract, not as a renderer: `Metal`
  (`src/frameworks/metal.rs` provides the object/descriptor objects used during
  startup; command buffers, pipelines and shaders are not executed)
- Implemented: `Foundation`, `UIKit`, `QuartzCore`, `CoreGraphics`,
  `CoreFoundation`, `CoreMedia`, `CoreVideo`, `AVFoundation`, `AudioToolbox`,
  `Security`, `GameController`, `GameKit`, `StoreKit`, `VideoToolbox`,
  `WebKit`, `SystemConfiguration`

So the gap is not "a few missing methods": the app's game code is written against
Metal command streams, MetalFX, the Swift runtime, iOS 17 `UIScene` lifecycle,
`BackgroundAssets` and `AuthenticationServices`, and the emulator has a stub or
nothing for all of those.

Even if the CPU and graphics problems were solved, the realistic ceiling would be
"Objective-C parts of a 2023 Ubisoft title launch and immediately hit an
unimplemented API", because the game's own game code is written against
Metal + Swift + iOS 17 UIKit.

## Why this cannot be fixed by adding an option

### 1. Guest pointers are 32-bit by construction

- `src/mem.rs:25` — `pub type GuestUSize = u32;`
- `src/mem.rs:40` — `type VAddr = GuestUSize;`
- `src/cpu.rs` — `type VAddr = u32;`

Every guest pointer, allocation size, page number and address in the emulator
derives from these aliases, and the entire `objc`, `dyld`, `mach_o`, `libc`,
`frameworks` and `gles` trees are written against them. A 64-bit guest is not a
compile-time switch that can be flipped; it changes the width of every pointer
that crosses the host/guest boundary, the ObjC runtime representation, the dyld
linker, the `NSObject` layout assumptions, and the address-space model in
`Mem`. That is a rewrite of the emulator's core, not a patch.

### 2. The bundled CPU core is 32-bit only

`src/cpu/dynarmic_wrapper/build.rs` configures dynarmic with

```rust
build.define("DYNARMIC_FRONTENDS", "A32"); // We don't need 64-bit
```

and `src/cpu/dynarmic_wrapper/lib.cpp` includes only
`dynarmic/interface/A32/*`. Even though dynarmic upstream has an A64 frontend, the
wrapper, the callbacks, the exception table and the Rust side (`CpuContext`,
`GuestFunction`, `get_reg`, `set_reg`, …) are all A32-only, and the 32-bit
address space above makes an A64 frontend pointless without change #1.

### 3. The renderer is OpenGL ES 1.x/2.0, and the asset pipeline assumes Metal

`src/gles.rs` implements `EAGL`/`GLES` on top of the host GPU. The game links
`Metal`, `MetalFX` and `IOSurface`, ships `ShaderData.bin` (Metal shader
container) and a 2.2 GB `Assets.car` compiled for Metal-era asset catalogs.

`MetalFX` in particular (temporal upscaling + denoising) has no OpenGL ES
equivalent at all — it is an Apple-specific post-process that operates on Metal
textures and history buffers.

### 4. There is no data to run even on a real device

`BAManifestURL` points at `ubiservices.cdn.ubi.com`, and
`BAEssentialMaxInstallSize` is 7.5 GB while the IPA is 4.2 GB. This is a
BackgroundAssets "stream install" build: on first launch it downloads the rest of
the game. `BackgroundAssets` is not implemented.

## About the "Metal → Vulkan" idea

The user-facing layer that exists and ships is **MoltenVK**, and it goes the other
way: Vulkan calls on top of Metal. The reverse — Metal calls translated to Vulkan,
which is what running an iOS game on an Android GPU would need — has no mature
open-source implementation; it exists only as an idea in discussions such as
[darlinghq/darling#1646](https://github.com/darlinghq/darling/discussions/1646).

More importantly, even a perfect Metal→Vulkan layer would not make this app run
here:

1. The CPU problem comes first. The code is arm64; the emulator cannot decode,
   let alone execute, one instruction of it. A GPU translation layer has nothing
   to render until #1 and #2 are solved.
2. iOS 17 UIKit + Swift `Foundation`/`os`/`XPC`/`Concurrency` behaviour would have
   to be implemented; the game uses `UIScene` lifecycle, `BackgroundAssets`,
   `AuthenticationServices` and `StoreKit`.
3. MetalFX and the game's Metal compute pipeline would have to be emulated on top
   of a Vulkan translation that does not exist. The existing Metal layer here
   (`src/frameworks/metal.rs`) is deliberately contract-only: it satisfies the
   object/descriptor calls an app makes during startup so the app can get
   further, and executes nothing.

Papers/projects that "run iOS on Android hardware" either virtualise a real
iOS kernel on an Apple SoC (Corellium-style, needs Apple hardware) or are not
running iOS apps at all (Winlator/DXVK-class projects translate *Windows*/D3D
binaries, and D3D→Vulkan is a solved problem in a way Metal→Vulkan is not).

## What the emulator does now instead

`src/preflight.rs` reads the main executable's Mach-O header and load commands
before anything is mapped into guest memory, and reports:

- architecture and bitness, from the header (thin or fat)
- `MinimumOSVersion` and SDK, from `LC_BUILD_VERSION`/`LC_VERSION_MIN_IPHONEOS`
- FairPlay encryption state, from `LC_ENCRYPTION_INFO[_64]`
- every linked framework that is not in the linker's own list of implemented
  libraries (`src/dyld/dylib_list.rs`), with the reason, and whether the link is
  weak (`LC_LOAD_WEAK_DYLIB`), which dyld allows to be absent

Hard blockers (non-32-bit ARM, big-endian, 64-bit structures) stop the launch with
a one-line, actionable error instead of a failure deep in the loader. Everything
else is a warning, so apps that partially work keep working exactly as before.
A stale `cryptid` (some decryption tools leave it set) is a warning too, not a
blocker, for the same reason.

For this IPA, `--info` now prints:

```
Executable preflight:
- Minimum OS version (from executable): 17.4.0
- Built against SDK version: 17.5.0
- CPU: arm64 (cputype 0x100000c, cpusubtype 0x0), 64-bit
- Blocker: Executable is built for arm64, but this emulator only runs 32-bit ARM ...
- Warning: App declares a minimum OS of iOS 17.4.0. ...
- Warning: Metal is not implemented ...
- Warning: MetalFX (upscaling/denoising) is not implemented ...
- Warning: The Swift runtime and its frameworks are not implemented ...
...
```

## Bottom line

Assassin's Creed Mirage cannot be made to open in HyperHLE-Reborn. The blockers
are structural (guest address width, CPU frontend, GPU API, Swift, iOS 17
frameworks, streaming assets), not configuration. Making it run would be a new
emulator, not a change to this one.

The emulator now says so clearly, in one command, instead of failing obscurely —
and the Android build (see `.github/workflows/build-apk.yml`) does not change.
