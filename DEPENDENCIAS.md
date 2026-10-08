# Dependências e libs próprias

## Política

Usar libs de terceiros **só** quando passarem nos critérios abaixo. Se uma lib não passar, ou o
projeto precisar de um comportamento que ela não dá, **criamos a nossa**. Os critérios reduzem
risco, não garantem que uma lib nunca terá problema, então toda dependência entra por decisão
explícita registrada aqui.

Critérios (todos, ou justificativa escrita):

- Uso amplo e antigo no ecossistema, ou mantida por organização conhecida (Rust project, Khronos, Google, `gfx-rs`/`wgpu`, BurntSushi etc.).
- Releases estáveis (1.x, ou 0.x que a comunidade trata como estável), sem histórico de breaking changes frequentes.
- Licença permissiva (MIT/Apache-2.0/BSD/zlib).
- Poucas dependências transitivas, `unsafe` contido e justificável.
- Passa em `cargo audit` e `cargo deny`, e cada versão nova é fixada (`Cargo.lock` commitado).
- Código dentro do processo do jogo (a layer) exige critério mais duro que o app de controle.

## Terceiros candidatos (nada instalado ainda; confirmar versão/estado antes de adotar)

| Uso | Candidata | Observação |
|---|---|---|
| Bindings Vulkan | `ash` | Padrão de fato. Na layer, preferir fino. |
| UI do app de controle | `egui` (+ `eframe`) | Só no app de controle, fora do jogo. |
| Serialização | `serde` + `toml`/`serde_json` | Manifesto e presets. |
| Erros / logs | `thiserror`, `tracing` | |
| SPIR-V | `rspirv` (se um dia gerarmos SPIR-V direto), `spirv-tools` (validação) | Avaliar necessidade. |
| Compilar GLSL/HLSL | `shaderc` ou DXC | Exigem toolchain C++ no build. |
| Texto / busca | `aho-corasick`, `unicode-segmentation` | Em vez de portar os nossos. |
| Sistema | `rustix` ou `nix`, `memmap2` | Socket Unix e memória compartilhada. |

## Libs próprias candidatas

**Nenhuma existe ainda.** Cada uma só nasce quando o gatilho acontece.

| Lib candidata | Gatilho | Observação |
|---|---|---|
| `vk-layer` (dispatch tables, chain de create-info, negociação com o loader) | Marco 1 | Não achei crate maduro para isso em Rust. É provável que tenhamos que escrever. |
| `fx-bundle` (formato e leitor/escritor do bundle) | Marco 3 | Compartilhado entre layer e app de controle. |
| `depth-tracker` | Marco 5 | Núcleo do diferencial, isolado do Vulkan o máximo possível para ser testável sem GPU. |
| `ipc` (protocolo layer ↔ controle) | Marco 4 | Socket Unix, mensagens versionadas. |
| `reactive` (signal/derived em Rust) | Quando o app de controle precisar de parâmetros reativos | Antes de escrever, avaliar `sycamore-reactive` e `reactive_graph`. Só vale uma própria se os requisitos (handles por id, FFI, sem `Arc`/lock, síncrona) não forem atendidos. |
| `fx-parser` (front-end FX em Rust) | Decidido depois, se vier a substituir o C++ | Opção: `winnow`/`chumsky`/`logos`. |

## Ligação com `@lotexiu/typescript`

Alguns conceitos da lib TS (`signal`/`derived`, `Mask`, `Parser`/`Language`, `Palette`, `ValueHistory`)
são candidatos naturais a versão em Rust. **Não portar especulativamente**: mesma regra do
CLAUDE.md daquele projeto. Só quando uma necessidade concreta daqui aparecer, e olhando antes se o
design em Rust pede outra forma (handles por id em vez de closures, escolha de thread-safety etc.).
