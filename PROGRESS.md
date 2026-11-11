# PROGRESS.md — Subway Surfers v3.70.1 no HyperHLE-Reborn

> Arquivo vivo de progresso. Toda sessão deve lê-lo primeiro e atualizá-lo ao
> final. Branch de trabalho: `subway-surfers-support` (a partir de
> `material-you-3-expressive`, que já contém `trunk` + o redesign Material You;
> `trunk` permanece intocado). Nada disso vai para `trunk` sem revisão.

## Estado atual (Fase 1 — P2 concluído: fixups + binds + ponte HLE)

**Fase 0 completa; Fase 1 (CPU A64 + loader arm64) com linking completo
e validado nos binários reais do jogo.** O `src/dyld64.rs` aplica chained
fixups (`DYLD_CHAINED_PTR_64_OFFSET`) e os streams clássicos de
rebase/bind do `LC_DYLD_INFO` (incluindo binds **threaded**), resolve
imports via trampolines `svc #0x81` (ponte HLE A64) e coleta constructors
(`__mod_init_func` + `__init_offsets`). Validado contra os 3 binários
reais: **923.514 fixups aplicados, 0 anomalias** (stub 55+56,
UnityFramework 835.519+81.008, SubwaySurfers.dylib 1.565+5.367).
Suite completa: **492 testes, 0 falhas**.

- Marco atual: **M1 ~completo** (loader+linking+execução até `main`
  demonstrados em teste de integração loader→CPU→HLE; falta montar o
  processo real — stack/argv/auxv — e rodar o stub verdadeiro).
- Última mudança: `feat(dyld64)` — fixups encadeados, binds clássicos
  e threaded, trampolines HLE A64, constructors; 5 testes novos +
  1 smoke test real (ignorado por padrão, roda com o IPA local).

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

1. **P1 (F1) — CONCLUÍDA**: dynarmic A64 + `Mem64` + testes (sessão 1).
   *Falta (performance, F5):* fastmem/page-table no dynarmic.
2. **P2 (F1) — CONCLUÍDA**: loader Mach-O arm64 (`mach_o64.rs`: parse
   thin/fat, seções, `LC_MAIN`, recusa FairPlay; `dyld64.rs`: chained
   fixups 64_OFFSET, binds clássicos + threaded, ponte HLE `svc #0x81`,
   constructors). Validação nos binários reais do jogo: 923.514 fixups,
   0 anomalias. Descobertas medidas estão nas notas de formato abaixo.
3. **P3 (F1) — próxima**: montar o processo para executar o stub real
   (stack com argc/argv/envp/auxv, `program_cookie`/`__progname`),
   loop de execução que despacha `Svc(0x81)` → trampolines e `Svc(0x80)`
   → syscalls, e um `--dump` da lista de símbolos HLE ausentes.
4. **P4 (F2)**: runtime Swift real (build swift.org para Android) — maior
   risco não-GPU; UnityFramework importa ~6.600 símbolos Swift.
5. **P5 (F3)**: tradutor MSL→GLSL ES + camada Metal sobre GLES3.
6. **P6**: texturas (ASTC) e áudio — confirmar formatos no primeiro boot.
7. **P7**: TBI (top-byte-ignore) no `Mem64` — rebases `64_OFFSET` com
   `high8=0x80` existem nos binários; hoje o valor é armazenado como o
   dyld armazenaria, e o guest que desreferenciar precisará do TBI.
3. **P3 (F1)**: constructors/`UIApplicationMain` com UIScene.
4. **P4 (F2)**: runtime Swift real (build swift.org para Android) — validar
   cedo porque é o maior risco não-GPU.
5. **P5 (F3)**: tradutor MSL→GLSL ES para padrões Unity 2022 + camada Metal.
6. **P6**: preflight deve detectar Unity/IL2CPP + Swift obrigatório e
   relatar (melhora o diagnóstico de qualquer IPA moderno).
7. **P7**: texturas (ASTC) e áudio (codecs) — confirmar formatos reais no
   primeiro boot.

## Notas de formato medidas (loader arm64 — referência para F2+)

Fixups encadeados (stub e UnityFramework, `DYLD_CHAINED_PTR_64_OFFSET`):
* `page_start[]` fica no offset **22** de `dyld_chained_starts_in_segment`
  (logo após `page_count`) e é um **byte offset** dentro da página — ler o
  offset 24 pega o primeiro byte da tabela de imports e gera cadeias
  falsas (foi a causa de um bug sutil desta sessão).
* rebase: valor final = `load_addr + (high8 << 56 | target)`; há entradas
  com `high8=0x80` (tag TBI) no UnityFramework.
* bind: `ordinal:24 + addend:8` (bits 63 = bind), `next` em bits 51..63,
  stride 4 bytes, `next==0` termina a cadeia.
* `reserved` NÃO é validado pelo dyld — entradas com bits de reserved
  não-nulos existem e são aplicáveis.

Binds clássicos (SubwaySurfers.dylib, `LC_DYLD_INFO`):
* A tabela canônica de opcodes difere da que se encontra em fontes
  antigas: **0xA0 = `DO_BIND_ADD_ADDR_ULEB`** (bind + avança `uleb+8`),
  **0xB0 = `DO_BIND_ADD_ADDR_IMM_SCALED`** (avança `imm*8+8`),
  **0xC0 = `DO_BIND_ULEB_TIMES_SKIPPING_ULEB`**, **0xD0 = `THREADED`**
  (sub 0x00 = tamanho da tabela de ordinais; sub 0x01 = APPLY, que
  percorre os slots encadeados estilo arm64e: bit62 = bind, ordinal
  0..16 na tabela, delta bits 51..62 × 8). Não existem
  `DO_BIND_WITH_ADDEND_ULEB`/`DO_BIND_ADDEND_ULEB`/`0xE0`.
* `ADD_ADDR_ULEB` recebe deltas **negativos** codificados como uleb de 10
  bytes; dyld soma com wraparound de 64 bits — idêntico a somar o sleb.

## Stubs e gambiarras

Nenhum ainda. Regra: todo stub entra aqui com motivo e plano de remoção.
(A `TrapResolver` do `dyld64.rs` não é um stub do jogo: é o mecanismo de
resolução M1 que manda todo import para um trampoline `svc #0x81` e
registra o símbolo — a F2 substitui por funções HLE reais.)

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

### Sessão 2 — P2 concluído: linking arm64 completo validado no jogo (2026-11-11)

- Re-baixado o IPA (o zip anterior foi limpo do disco); extraídos os 3
  binários + `global-metadata.dat` em `ipa-work/` (fora do repo).
- Diagnóstico clean-room dos formatos de fixup com scripts próprios
  (`scripts/fixup_diag.py`, `fixup_walk.py`) + fontes públicas do dyld
  (comportamento): `page_start` no offset 22 em BYTES; rebase 64_OFFSET =
  base + (high8<<56|target); tabela canônica de bind opcodes; binds
  threaded 0xD0; deltas negativos com wraparound.
- **`src/dyld64.rs` novo (~900 linhas)**: parse da tabela de imports
  (formatos 1/2/3), aplicação de chained fixups, interpretador clássico
  de rebase/bind (com threaded), `TrampolinePage` (`svc #0x81` em
  0x7000_0000_0000), `TrapResolver`, `init_slots`/`read_init_functions`
  (`__mod_init_func` + `__init_offsets`), `load_and_link` completo.
- `mach_o64.rs`: seções (stride 80 confirmado nos binários), `LC_DYLD_INFO`,
  `LC_DYLD_CHAINED_FIXUPS`, `map_with_entry` com slide correto (descoberta
  de bug: o mapeamento ignorava o slide de dylibs), `__PAGEZERO` deixado
  NÃO mapeado (PROT_NONE; mapear 4 GiB materializava 1M páginas e esgotava
  a memória).
- Ponte HLE A64 validada em teste de integração loader→CPU: `main` chama
  import pelo GOT → trampoline → host devolve 42 → `main` armazena → `svc
  #0x80` sai limpo.
- Smoke real (`TOUCHHLE_A64_IPA_APP`): stub 55+56, player 835.519+81.008,
  game 1.565+5.367 (threaded binds inclusos) — **0 anomalias** nos 3.
- Correções de bugs com causa raiz documentada: `BIND_OPCODE_DONE`
  indefinido virava *binding pattern* que casava com qualquer opcode;
  section_64 tem `reserved3` (80 bytes, não 76); ULEB overlong satura.
- Suite: **492/492**. CI deve rodar no push da branch.

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
- `build-apk.yml` agora roda por push na branch (além de workflow_dispatch);
  primeira rodada da branch: **SUCCESS** (run 42921967795).
- **Segundo commit de F1 (`feat(macho64)`)**: loader Mach-O arm64 com
  parse thin/fat, mapeamento de segmentos, `LC_MAIN`/`LC_UNIXTHREAD`,
  recusa de FairPlay/não-arm64; 5 testes novos incluindo integração
  loader→CPU até o `svc`. Suite: 487/487.
- Nota: os números de fixup citados acima (41+18 no stub) eram da leitura
  com offset/unidade errados de `page_start`; os corretos são 56+55.
