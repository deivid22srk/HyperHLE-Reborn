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

The shaders it loads are **precompiled Metal binaries**: `ShaderData.bin` (84 MB)
is a `SBIN` container holding 5,924 `MTLB`/`.metallib` entries, and every one of
them wraps an **LLVM bitcode** blob (Apple's `0x0B17C0DE` wrapper followed by
`BC C0 DE`).

That is better news than it sounds, and it was measured rather than assumed. All
300 sampled shaders disassembled with stock `llvm-dis` (LLVM 18), no patches:

```
; ModuleID = 'shader_0.bc'
source_filename = "F7c281e684c3043199463b033cdb02339c98f84697b25f618b31096952d845a18"
target datalayout = "e-p:64:64:64-f32:32:32-..."
target triple = "air64-apple-ios17.0.0"

define %ret_ShadePixel_ty @F7c281e6...(<4 x float> %0, ..., ptr addrspace(2) %4, ...) {
  %21 = call { <4 x float>, i8 } @air.sample_texture_2d_array.v4f32(
          ptr addrspace(1) %..unpack14, ptr addrspace(2) %.1.unpack, ...)
```

What the sample of 300 shows:

- Functions are named by **SHA-256 hash** of the source; there are no readable
  names.
- **104 distinct `air.*` names** in total, and only part of those are compute or
  texture operations. The real ones are a small, finite list: `air.fma.f32`,
  `air.fast_{fmin,fmax,fabs,sqrt,rsqrt,exp2,log2,rint,saturate}.f32`,
  `air.sample_texture_2d_array/3d`, `air.gather_texture_2d_array`,
  `air.write_texture_2d_array`, `air.dot.v3f32/v4f32`, `air.simdgroup.barrier`,
  `air.discard_fragment`, `air.convert.f.f32.f.f16`. The rest are metadata names
  (`air.arg_name`, `air.version`, `air.max_textures`, …).
- **70 % of the sample are compute shaders** (211 `!air.kernel` vs 53
  `!air.fragment` and 36 `!air.vertex`) — consistent with the GPU-driven,
  indirect-command-buffer design the Metal classes suggested.
- **Every shader dereferences pointers** in address space 2 and 1
  (`%struct__desc = type { ptr addrspace(1), ptr addrspace(1), i64 }`, then
  `getelementptr`/`load` chains). This is Metal's argument-buffer model: the
  shader is handed GPU addresses and follows them. Vulkan has no pointer
  dereference, so this has to become descriptor indexing or buffer device
  addressing — the same class of problem DXVK/VKD3D solve for D3D12.
- There are traces of Apple's own DXIL→Air converter in the IR
  (`irconverter_discard`, `dx.types.CBufRet.f32`), which suggests the original
  shaders were HLSL compiled with DXC and converted, not hand-written MSL.

So the shader step is a *bounded* reverse-engineering and recompilation job with
a measured API surface, not an open-ended one. The hard part is the argument
buffer/pointer ABI, not the intrinsic list.

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
| 5 | Shaders | 5,924 precompiled `metallib` (Air bitcode) → SPIR-V/GLSL | bitcode is readable with stock LLVM; 104 `air.*` names, but the argument-buffer pointer ABI must be reimplemented |
| 6 | Content | 2.2 GB `Assets.car` + 7.5 GB streamed from Ubisoft's CDN + online auth | `BackgroundAssets` not implemented |

Even a perfect layer 4 leaves 1, 2, 3, 5 and 6. Layer 5 is a separate problem
with a measured size: the Air bitcode reads with stock LLVM (see above), so the
job is writing an Air→SPIR-V/GLSL pass for a small intrinsic set plus Metal's
argument-buffer pointer model, rather than prying open an opaque format. MoltenVK
does not help in this direction — it generates MSL *source* and lets Apple's own
compiler build it. MetalFX has no Vulkan equivalent at all.

The architecture that could plausibly work is a different project: run the arm64
code **natively** on an arm64 Android host (no CPU emulation needed at all),
provide a real iOS userland and framework stack, and put a Metal→Vulkan plus
Air→SPIR-V stack underneath. That is Darling/Corellium-class work — years, not a
patch — plus Ubisoft's streaming and online services. Since October 2026 there is
also a much shorter way to get the userland: boot iOS itself (next section). For
playing the game today, a real device, a Mac, or cloud streaming are the only
routes.

Prior art worth knowing: **Darling** (macOS userland on Linux, x86_64, no GPU
story for Metal-heavy apps), **Corellium** (virtualises real iOS on Apple
hardware — needs Apple silicon, not an Android phone), **Winlator/Box64 + DXVK**
(Windows/D3D games on Android; this is the "translation layers run anything now"
story people remember, and D3D→Vulkan is a solved problem in a way Metal→Vulkan
is not), and **Cider** (a 2014 research project that ran iOS apps on Android in
the x86 era; it never became usable).

## iOS itself now boots on Android hardware

This part is real as of October 2026, and it changes the analysis above: instead
of emulating iPhone OS, you can run Apple's actual iOS userland on a Snapdragon
phone.

- **[dipa-studio/ios-sweet](https://github.com/dipa-studio/ios-sweet)** — Apple's
  open-source XNU kernel (xnu-8796, the iOS 16 generation) ported to a Redmi Note
  10 Pro (SM7150 / Snapdragon 732G), with the unmodified iPhone X iOS 16.7.16
  userland on top: `launchd`, `backboardd`, SpringBoard and stock apps. It reached
  the home screen on 2026-10-18. Own UFS driver, HFS+ root in `userdata`, kernel
  `IOSurface` + `IOMobileFramebuffer` (`AppleCLCD`) services, touch/buttons,
  iPhone X identity, and activation through `mobileactivationd`'s own
  "Hactivation" path with no Apple servers. Rendering is done **entirely on the
  CPU** (20–30 fps) and there is no Wi-Fi, Bluetooth, cellular, audio, camera,
  sensors or GPS.
- **[joranv1/ios-sailfish](https://github.com/joranv1/ios-sailfish)** — the same
  approach for a Google Pixel 1 (Snapdragon 821 / **Adreno 530**), running the
  iPhone 8 16.7.16 userland. It goes much further: **GPU acceleration** with Mesa's
  freedreno (GLES 3.1) running inside `backboardd` as `sweetgl.dylib`, behind an
  EAGL shim that CoreAnimation's own OpenGL ES renderer uses as if it were Apple's;
  double-buffered panel present at up to 60 Hz; working Wi-Fi (WPA2), battery,
  brightness and display sleep; boot to lock screen in about 20 seconds.

Their own README states the limit that matters here, in one line:

> **No Metal.** CoreAnimation falls back to its OpenGL ES renderer, which is what
> runs on the GPU. Apps that need Metal do not work.

So the gap between "iOS runs on an Android phone" and "Assassin's Creed Mirage
runs on an Android phone" is now a small, nameable set of components rather than
"everything":

| Gap | Detail |
| --- | --- |
| iOS version | These ports run iOS 16.7.16. AC Mirage needs ≥ 17.0 (SDK 17.5), so XNU + userland must move to the iOS 17 generation |
| **Metal** | There is an EAGL→GLES shim (`sweetgl.dylib`) but no Metal. This is *the* missing piece for this title |
| Shaders | Air→SPIR-V/GLSL: bitcode reads with stock LLVM (measured), 104 `air.*` names, but Metal's argument-buffer pointers need a descriptor-indexing model |
| MetalFX | No equivalent; would have to be stubbed (identity scale) to start |
| SoC | Pixel 1 / Adreno 530 is 2016 hardware; the app asks for `iphone-performance-gaming-tier` (A12 class), so a modern Snapdragon with an unlocked bootloader is needed |
| Content | Wi-Fi works on the sailfish port, but a 7.5 GB first-launch download plus Ubisoft account services still have to succeed |

### The concrete path

The `sweetgl.dylib` trick — inject a library that implements an Apple graphics
API on top of Mesa, and let the real iOS userland call it — is exactly the shape a
Metal port would take: a `sweetmtl.dylib` that implements `MTLDevice`,
`MTLCommandQueue`, `MTLBuffer`/`MTLTexture`, render and compute pipelines,
`MTLHeap`, `MTLIndirectCommandBuffer`, samplers and depth/stencil state on top of
Vulkan (freedreno/turnip) or GLES. The work is large but the target is unusually
well defined, because the app tells you what it uses: its symbol table names the
19 `MTL*Descriptor` classes it drives (listed above).

Ordered steps, smallest useful first:

1. Port the XNU patches and the userland to the iOS 17 generation on a modern
   Snapdragon, and get SpringBoard up as the existing ports do.
2. Write `sweetmtl.dylib` far enough to run a Metal "hello triangle" inside real
   iOS (device, queue, command buffer, render pass, pipeline, draw).
3. Translate the shaders: the `metallib` container and the Air bitcode are
   readable already, so this is an LLVM pass over a small intrinsic set plus a
   descriptor-indexing model for the argument buffers — still the biggest single
   piece, but a software-engineering one now, not a reverse-engineering unknown.
4. Grow the API along the app's own call sites, stubbing MetalFX to an identity
   scaler so the game can reach a frame.
5. Only then worry about performance, streaming assets and account services.

Honest sizing: this is a DXVK/VKD3D-scale component (those took years and many
contributors) plus an OS port, so it is a multi-person, multi-year project — but
for the first time it is a *path*, not a wall. It is also a legal grey area:
Apple's licence only permits iOS on Apple hardware, Apple sued Corellium over
virtual iPhones, and these ports need an unlocked bootloader and wipe the phone.

For playing the game today, a real device, a Mac, or cloud streaming remain the
only routes.

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
frameworks, streaming assets), not configuration. Making it run here would be a
new emulator, not a change to this one.

Running it on Android at all is a different question, and since October 2026 it
has an answer that is not "impossible": the `ios-sweet`/`ios-sailfish` line of
work boots real iOS 16 on Snapdragon phones with GPU-accelerated CoreAnimation.
What stands between that and this title is a Metal runtime plus an Air→SPIR-V
shader path on the iOS 17 generation — years of work, but a defined direction
rather than a wall. See the section above.

The emulator now says all of this clearly, in one command, instead of failing
obscurely — and the Android build (see `.github/workflows/build-apk.yml`) does
not change.
