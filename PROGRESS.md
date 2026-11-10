# PROGRESS.md — Subway Surfers v3.70.1 no HyperHLE-Reborn

> Arquivo vivo de progresso. Toda sessão deve lê-lo primeiro e atualizá-lo ao
> final. Branch de trabalho: `subway-surfers-support` (a partir de
> `material-you-3-expressive`, que já contém `trunk` + o redesign Material You;
> `trunk` permanece intocado). Nada disso vai para `trunk` sem revisão.

## Estado atual (Fase 1 — núcleo A64, groundwork concluída e testada)

**Fase 0 completa; Fase 1 (CPU A64) tem fundação funcionando e testada.**
O núcleo dynarmic A64 está integrado, compila no workspace inteiro e executa
código A64 real em testes unitários (MOVZ/ADD/STR/LDR/MOVK até um `svc`,
com estado de registradores e memória corretos). Nenhum código do jogo é
carregado ainda — o loader arm64 é o próximo passo (P2).

- Marco atual: **M0 → M1 em progresso** (CPU A64 executa código; falta o
  loader + runtime HLE para chegar ao `main` do app).
- Última mudança: `feat(cpu-a64)` — dynarmic A32;A64, `src/cpu/a64.rs`,
  `Mem64` por páginas, 4 testes novos; suite completa: **482 testes, 0 falhas**.

## Dados do jogo (medidos, não presumidos)

IPA: `subway-surfers-v3.70.1-iosvizor.ipa` (249.550.819 bytes, zip íntegro).

| Propriedade | Valor | Fonte |
| --- | --- | --- |
| Executável principal | `Payload/SubwaySurf.app/SubwaySurf` — 110.224 bytes (stub) | zip |
| Player | `Frameworks/UnityFramework.framework/UnityFramework` — 134.010.784 bytes, `__TEXT` de 115 MB | Mach-O |
| Código do jogo (IL2CPP) | `Frameworks/SubwaySurfers.dylib` — 14.315.600 bytes | Mach-O |
| Metadados IL2CPP | `Data/Managed/Metadata/global-metadata.dat` — 16,5 MB, magic `0xFAB11BAF`, **versão 31** | header |
| Engine | **Unity 2022.3.72f1** (string no `globalgamemanagers`) | dados |
| Arquitetura | **arm64 puro** nos 3 binários (sem slice armv7) | Mach-O header |
| `MinimumOSVersion` (plist) | 10.0 (obsoleto) | Info.plist |
| `minos` real (LC_BUILD_VERSION) | **15.0** no stub e no UnityFramework; 13.0 no SubwaySurfers.dylib | Mach-O |
| SDK de build | 27.1 (`DTSDKName: iphoneos27.1`, Xcode 27.1) | Info.plist |
| **Criptografia FairPlay** | **`cryptid = 0` nos 3 binários — NÃO criptografado** | LC_ENCRYPTION_INFO_64 |
| Fixups | stub e UnityFramework usam **LC_DYLD_CHAINED_FIXUPS** (iOS 15+); SubwaySurfers.dylib usa binds clássicos (`LC_DYLD_INFO`) | Mach-O |
| Bundle ID | `com.kiloo.subwaysurfers`, versão 3.70.1, build 96703 | Info.plist |
| Orientação | Portrait apenas (`UISupportedInterfaceOrientations`) | Info.plist |
| Ciclo de vida | **`UIApplicationSceneManifest` presente → ciclo UIScene (iOS 13+)** | Info.plist |
| Gráficos | **Metal exclusivo**: UnityFramework linka `Metal.framework`, `OpenAL.framework`… e **NÃO linka `OpenGLES.framework`** | load commands |
| Shaders | **MSL em texto** no `unity default resources` (42× `#include <metal_stdlib>`); nenhum container `MTLB`/metallib real no UnityFramework (os 32 hits eram type-encodings `@"<MTLBuffer>"`); shaders do jogo em MSL nos arquivos de dados | scan de bytes |
| Audio | OpenAL (FMOD) + AudioToolbox/AVFoundation; codecs no lado dos dados (Vorbis/ADPCM esperados — confirmar em runtime) | load commands |
| Texturas | A confirmar em runtime; padrão Unity iOS 2022 = ASTC (decodificável por hardware GLES 3.2 no Android moderno ou por software) | pendente |
| Data | Addressables em `Data/Raw/aa/iOS/3.70.1/*.bundle` (centenas) + `resources.assets`, `sharedassets0.assets`, `level0` | zip |

### Superfície de APIs importadas (medida com scripts próprios)

Scripts: `dev-scripts/analyze_ipa/` (ver "Como reproduzir"). Números exatos:

| Binário | Imports | Destaques |
| --- | --- | --- |
| UnityFramework | **6.613** | **~6.000 símbolos Swift manglados (`$s…`)** de `libswiftCore` etc.; 379 classes ObjC (94 NS\*, 69 UI\*, 26 FBSDK\*, 21 MTL\*); 55 pthread; 43 dispatch; vtables `__cxxabiv1` |
| SubwaySurfers.dylib | 2.122 | 96 classes ObjC; pthread 23; dispatch 17; UIKit/MessageUI/Social/Accelerate |
| SubwaySurf (stub) | 50 | dlopen/dlsym do UnityFramework; StoreKit/GameKit/AdSupport/AudioToolbox/AVFoundation/WebKit/Security; 20 dylibs Swift **weak** |

**Classes Metal referenciadas pelo UnityFramework** (a API Metal mínima real):
`CAMetalDisplayLink` (iOS 17), `MTLRenderPipelineDescriptor`,
`MTLComputePipelineDescriptor`, `MTLRenderPassDescriptor`,
`MTLTextureDescriptor`, `MTLSamplerDescriptor`, `MTLDepthStencilDescriptor`,
`MTLHeapDescriptor`, `MTLFunctionConstantValues`, `MTLVertexDescriptor`,
`MTLBufferLayoutDescriptor`, `MTLCompileOptions`, `MTLCaptureManager`…

### Verificação obrigatória de integridade do pacote

O IPA **não é uma cópia limpa da App Store** — é uma re-empacotada
(sideload), com injeções identificadas:

- `SignedByEsign` (marcador da ferramenta eSign) e assinatura refeita em
  2026-10-14.
- `Frameworks/iGameGod.framework` (22 MB, Swift) — ferramenta de cheat/trainer
  **injetada no load commands do stub** (`@rpath/iGameGod.framework/iGameGod`).
- `Frameworks/CydiaSubstrate.framework` + `Libraries/Substrate*.dylib` —
  infra de tweak.
- `libswiftIU.dylib` solto na raiz do `.app` (`@executable_path/libswiftIU.dylib`).
- Os arquivos do jogo original têm data 2026-10-12; os injetados, 2026-10-09/14.

**Decisão (registrada como decisão técnica D4):** o emulador deve tratar
`iGameGod`, `CydiaSubstrate` e `libswiftIU` como **pulado por política**
(skip de load por nome na tabela de dylibs do emulador, com aviso no log),
não como parte do jogo. Não adicionamos comportamento de cheat; o objetivo é
que o jogo original rode. Nenhum asset do jogo será copiado para o repo.

## O que o emulador tem hoje vs. o que o jogo exige

| Camada | Jogo exige (medido) | Emulador hoje | Gap |
| --- | --- | --- | --- |
| 1. CPU | arm64 (A64) usuário, PIE, ~25M+ instruções no UnityFramework | dynarmic **A32** apenas; `VAddr = u32` | **total** |
| 2. Memória guest | espaço de 64 bits, `__PAGEZERO` 4 GB, endereços `0x100000000+` | `GuestUSize = u32` em toda a árvore | **total** |
| 3. Loader | Mach-O 64 + chained fixups + binds clássicos + constructors | loader Mach-O 32 | **total** |
| 4. ObjC | objc4 moderno (isa não-ponteiro, ARC, ~475 classes referenciadas) | runtime ObjC 32-bit próprio | grande |
| 5. Swift | **~6.000 imports de libswiftCore e afins** | `src/dyld/swift_runtime.rs` (stub mínimo) | **grande e crítico** |
| 6. C++ | libc++/libc++abi, exceções (IL2CPP usa), RTTI | `libc/cxxabi.rs` parcial 32-bit | grande |
| 7. GPU | Metal (21+ classes MTL\*), `CAMetalDisplayLink`, shaders MSL runtime | `frameworks/metal.rs` só contratos de objetos | **total** |
| 8. UIKit | ciclo **UIScene**, UIKit moderno | UIKit iPhone OS 2.x–4.x | grande |
| 9. Frameworks | AVFoundation/AVKit (vídeo), OpenAL, CFNetwork, WebKit, Security, StoreKit, GameKit, ATT, UserNotifications, CoreMotion, GameController, CoreHaptics, CoreTelephony | maioria parcial para a era 2.x–4.x | médio/grande |
| 10. SDKs 3ºs | Facebook (5 dylibs), AppLovin, InMobi, Moloco, Helpshift, AdMob, Firebase | stubs antigos (mopub etc.) | médio (falha educada offline) |

## Decisões técnicas

- **D1 — CPU: frontend A64 do dynarmic** (a alternativa Unicorn/QEMU TCG é
  mais pesada e pior integrada; execução nativa fica como otimização futura,
  não como v1). O dynarmic vendoredo (`KlugKlugTG/dynarmic`,
  `dbbbd5a`) **já contém** `frontend/A64` e `interface/A64` completos; a
  mudança é habilitar `DYNARMIC_FRONTENDS="A32;A64"` e escrever o wrapper
  A64 espelhando o A32. Motivo: mesma arquitetura de integração, JIT cache
  maduro no Android, menor risco.
- **D2 — Memória: modelo de 64 bits como evolução do `Mem` atual**
  (`GuestUSize = u64` para apps arm64), **sem quebrar os apps 32-bit**:
  a largura de ponteiro passa a ser uma propriedade do processo guest
  escolhida pelo loader (armv6/armv7 → 32; arm64 → 64). Primeiro passo:
  tipar o núcleo (`mem.rs`, `abi.rs`, `cpu.rs`) por processo; depois
  atualizar as árvores `libc`, `objc`, `frameworks` na ordem do fluxo de
  boot. É o item mais longo do projeto (rewrite de núcleo, como o
  relatório do AC Mirage já antecipava).
- **D3 — Swift: usar um runtime Swift real em vez de reimplementar**.
  A leitura dos 6.000 imports mostra que são metadados genéricos e
  funções do core do Swift (`_$sSQMp`, `_$sSH9hashValue…`), não lógica de
  jogo em Swift. Plano: compilar `libswiftCore` (swift.org, alvo Android
  arm64 — suportado oficialmente) no host e expor seus símbolos ao guest
  via a tabela de símbolos HLE (padrão Darling). As partes que ponteiam
  com Foundation/ObjC da Apple ganham shims. **Swift nunca vai ser
  "stub de contrato"** — sem runtime real o Unity não passa da inicialização.
- **D4 — Injeções do repack (iGameGod, CydiaSubstrate, libswiftIU):
  pular por política**, com aviso no log. Ver seção de integridade acima.
- **D5 — Metal sobre GLES 3.x primeiro, Vulkan depois.** O backend GLES do
  emulador já é maduro (gles3_on_gl3 etc.); Unity gera MSL padronizado de
  máquina, o que torna um tradutor MSL→GLSL ES limitado a padrões Unity
  viável como v1 (air2glsl já provou o fluxo AIR→GLSL no caso Mirage;
  aqui a entrada é ainda melhor: MSL texto). Vulkan fica para performance.
- **D6 — Rede offline estrita**: NSURLSession/CFNetwork falham rápido
  (erro de domínio), SDKs de anúncio falham educadamente. O jogo precisa
  bootar 100% offline (verificado que SDKs iniciam conexões na arrancada).

## Ordem de execução (fases)

1. **F1 — Núcleo A64** (gate de tudo): `DYNARMIC_FRONTENDS="A32;A64"` +
   wrapper A64 + `Mem` 64-bit + loader Mach-O arm64 com chained fixups e
   `LC_MAIN`; objetivo: `main` do stub rodando (M1).
2. **F2 — Runtime**: objc4 arm64 (ARC, tagged pointers), blocks, libc++
   exceções/unwind, pthread/dispatch/mach 64-bit, syscalls BSD; depois
   Foundation mínimo + UIScene/UIApplicationMain (M2).
3. **F3 — Metal**: camada MTL\* sobre GLES3 + tradutor MSL→GLSL ES +
   `CAMetalLayer`/`CAMetalDisplayLink` e apresentação (M3: primeiro frame;
   M4: menu com toque).
4. **F4 — Áudio/entrada**: OpenAL (já existe no emulador), AudioToolbox,
   AVFoundation para vídeo de splash; multitoque e gestos (M5, M6).
5. **F5 — Android/perf**: rodar no host arm64 real, fastmem, otimização de
   texturas/ASTC, ciclo de vida (M7, M8).

## Pendências (prioridade)

1. **P1 (F1) — CONCLUÍDA (groundwork)**: dynarmic A64 habilitado
   (`DYNARMIC_FRONTENDS=A32;A64`), wrapper C++ (`lib_a64.cpp`), lado Rust
   (`src/cpu/a64.rs`) com contexto, TPIDRRO_EL0, CNTFRQ 24 MHz, halting
   idêntico ao A32, e `Mem64` por páginas. 4 testes unitários cobrem
   execução real até SVC, erro de memória, step único e troca de contexto.
   *Falta:* fastmem/page-table no dynarmic (performance) e integração com
   hooks HLE — ambos junto com o loader.
2. **P2 (F1)**: loader Mach-O arm64 (chained fixups: parses já validados nos
   scripts; binds clássicos do SubwaySurfers.dylib — `LC_DYLD_INFO`;
   constructors; `LC_MAIN`).
3. **P3 (F1)**: constructors/`UIApplicationMain` com UIScene.
4. **P4 (F2)**: runtime Swift real (build swift.org para Android) — validar
   cedo porque é o maior risco não-GPU.
5. **P5 (F3)**: tradutor MSL→GLSL ES para padrões Unity 2022 + camada Metal.
6. **P6**: preflight deve detectar Unity/IL2CPP + Swift obrigatório e
   relatar (melhora o diagnóstico de qualquer IPA moderno).
7. **P7**: texturas (ASTC) e áudio (codecs) — confirmar formatos reais no
   primeiro boot.

## Stubs e gambiarras

Nenhum ainda. Regra: todo stub entra aqui com motivo e plano de remoção.

## Como reproduzir e testar o estado atual

```bash
# análise do IPA (sem o IPA no repo; baixe você mesmo e aponte o caminho)
python3 dev-scripts/analyze_ipa/macho_analyze.py <ipa-extract>/SubwaySurf
python3 dev-scripts/analyze_ipa/api_surface.py   # edita os paths no topo

# build do emulador (host)
RUSTFLAGS="-C link-arg=-latomic" cargo build

# APK (CI)
gh workflow run build-apk.yml --ref subway-surfers-support
```

O IPA **nunca** vai para o repositório nem sua URL para qualquer arquivo
versionado (política do usuário). Os scripts de análise leem o IPA de um
caminho local e imprimem apenas métricas.

## Log de sessões

### Sessão 1 — Fase 0 concluída; Fase 1 com núcleo A64 funcionando (2026-11-10)

- Baixado e extraído o IPA (238 MB); zip íntegro.
- Verificações obrigatórias: `cryptid=0` nos 3 binários (não criptografado);
  arm64 puro; minOS real 15.0; chained fixups no stub e no player.
- Medida da superfície de APIs: 6.613 + 2.122 + 50 imports; Swift é a maior
  família; classes Metal listadas; OpenGLES ausente (Metal exclusivo);
  shaders MSL em texto; Unity 2022.3.72f1; IL2CPP metadata v31.
- Detectadas as injeções do repack (iGameGod, CydiaSubstrate, eSign).
- Branch `subway-surfers-support` criada a partir de
  `material-you-3-expressive`; trunk intocado.
- Scripts de análise commitados em `dev-scripts/analyze_ipa/`.
- **Groundwork F1 commitado (`feat(cpu-a64)`)**: dynarmic A32;A64 +
  `lib_a64.cpp` + `src/cpu/a64.rs` + `Mem64` + 4 testes A64; suite toda:
  482/482. Sandbox affordances documentadas no AGENTS.md
  (`TOUCHHLE_AUDIO_LIB_PREFIX`, `TOUCHHLE_NO_DEP_DEBUG_INFO`,
  `CMAKE_POLICY_VERSION_MINIMUM`).
- `build-apk.yml` agora roda por push na branch (além de workflow_dispatch).
