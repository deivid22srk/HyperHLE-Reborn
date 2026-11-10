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

### It is a native Metal engine with precompiled shaders

The executable's own symbol table names the Metal object model it drives directly
(not through a cross-platform engine — there are no Unity/Unreal/il2cpp strings
in the binary at all):

```
MTLBinaryArchiveDescriptor   MTLComputePipelineDescriptor  MTLRenderPipelineDescriptor
MTLIndirectCommandBufferDescriptor  MTLHeapDescriptor  MTLFunctionDescriptor
MTLFXTemporalScalerDescriptor  MTLFunctionConstantValues  MTLLinkedFunctions
MTLRenderPassDescriptor  MTLDepthStencilDescriptor  MTLSamplerDescriptor
MTLTextureDescriptor  MTLVertexDescriptor  MTLVertexBufferLayoutDescriptor  ...
```

`MTLIndirectCommandBufferDescriptor` means GPU-driven rendering (draw arguments
built on the GPU), `MTLBinaryArchiveDescriptor` means it loads compiled pipeline
archives, and `MTLFXTemporalScalerDescriptor` means it uses MetalFX temporal
upscaling.

The shaders it loads are **precompiled Metal binaries**: `ShaderData.bin` (81 MB)
contains 5,924 `MTLB` (`.metallib`) entries, each holding LLVM-based Apple
"Air" bytecode, not shader source.

This matters for the Metal→Vulkan question below: a runtime translation layer
needs to produce SPIR-V for Vulkan, but the input here is Air bytecode. Producing
Metal from SPIR-V is solved (MoltenVK: SPIRV-Cross emits MSL text, then Apple's
own compiler builds it at runtime). Going the other way — Air/metallib to
OpenGL ES or SPIR-V — has no production tool; the format has only been
partially reverse-engineered for research, and Apple's own "Metal shader
converter" also converts *towards* Metal (DXIL → metallib).

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

### 3. The renderer is OpenGL ES 1.x/2.0, and the shaders are precompiled Metal

`src/gles.rs` implements `EAGL`/`GLES` on top of the host GPU. The game links
`Metal`, `MetalFX` and `IOSurface`, ships `ShaderData.bin` (81 MB, 5,924
precompiled `.metallib` entries — see below) and a 2.2 GB `Assets.car` built for
Metal-era asset catalogs.

`MetalFX` in particular (temporal upscaling + denoising) has no OpenGL ES
equivalent at all — it is an Apple-specific post-process that operates on Metal
textures and history buffers.

### 4. There is no data to run even on a real device

`BAManifestURL` points at `ubiservices.cdn.ubi.com`, and
`BAEssentialMaxInstallSize` is 7.5 GB while the IPA is 4.2 GB. This is a
BackgroundAssets "stream install" build: on first launch it downloads the rest of
the game. `BackgroundAssets` is not implemented.

## Would a Metal → Vulkan layer make this run?

Short answer: no, and it is not the first wall — it is the fifth of six.

The well-known, mature layer is **MoltenVK**, and it goes the other way: Vulkan
on top of Metal. The direction Android needs — Metal calls executed on a Vulkan
GPU — was for years only an idea
([darlinghq/darling#1646](https://github.com/darlinghq/darling/discussions/1646))
and is now being worked on experimentally ("MoltenMTL", and a paravirtualised
Metal→Vulkan GPU for QEMU so a macOS guest can render without Metal). So the
direction is real; it is just early, and it targets hosts that already run a real
Apple userland.

Running this game means getting through six layers. A Metal→Vulkan layer is layer
4 (or 5); the app dies at layer 1:

| # | Layer | What AC Mirage needs | Status here |
| --- | --- | --- | --- |
| 1 | CPU/ISA | arm64 code execution | 32-bit A32 only (`VAddr = u32`, `DYNARMIC_FRONTENDS=A32`) |
| 2 | Loader + OS | Mach-O dyld, iOS BSD syscalls, Apple ObjC and Swift runtimes | 32-bit iPhone OS subset |
| 3 | Frameworks | iOS 17 `UIScene`, `BackgroundAssets`, `AuthenticationServices`, `GameKit`, `StoreKit`, `AVFoundation`, … | partial, old versions only |
| 4 | GPU API | Metal + **MetalFX** (temporal upscaler) | contract-only stub; no command execution |
| 5 | Shaders | 5,924 precompiled `metallib` (Air bytecode) → SPIR-V | no production Air→SPIR-V path exists |
| 6 | Content | 2.2 GB `Assets.car` + 7.5 GB streamed from Ubisoft's CDN + online auth | `BackgroundAssets` not implemented |

Even a perfect layer 4 leaves 1, 2, 3, 5 and 6. And layer 5 is a separate,
equally hard problem: MoltenVK works because it *generates MSL source* and lets
Apple's own compiler build it; there is no equivalent that turns Apple's
precompiled Air bytecode into SPIR-V for a Vulkan driver. MetalFX has no Vulkan
equivalent at all.

The architecture that could plausibly work is a different project: run the arm64
code **natively** on an arm64 Android host (no CPU emulation needed at all), ship
or rebuild the Swift runtime (it is open source), reimplement the iOS userland
and frameworks the title touches, and put a Metal→Vulkan + Air→SPIR-V stack
underneath. That is Darling/Corellium-class work — years, not a patch — plus
Ubisoft's streaming and online services. For actually playing the game, a real
device, a Mac, or cloud streaming are the only realistic routes.

Prior art worth knowing: **Darling** (macOS userland on Linux, x86_64, no GPU
story for Metal-heavy apps), **Corellium** (virtualises real iOS on Apple
hardware — needs Apple silicon, not an Android phone), **Winlator/Box64 + DXVK**
(Windows/D3D games on Android; this is the "translation layers run anything now"
story people remember, and D3D→Vulkan is a solved problem in a way Metal→Vulkan
is not), and **Cider** (a 2014 research project that ran iOS apps on Android in
the x86 era; it never became usable).

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
