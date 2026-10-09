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

## Decisões registradas

Medido em 2026-10-09 com `cargo tree` (só dependências normais, sem dev/build) em projetos de teste
em `.tmp/dep-eval/`. `cargo audit`/`cargo deny` ainda **não** foram rodados (não instalados).

| Uso | Escolha | Dados | Motivo |
|---|---|---|---|
| Memória compartilhada na **layer** (dentro do jogo) | **`rustix` 1.x**, features `std`, `mm` e `process` | 3 crates (`rustix`, `linux-raw-sys`, `bitflags`); Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT; Bytecode Alliance; 1.x | Superfície mínima no processo do jogo. Abrir o arquivo é com `std`; o `rustix` entra só para `mmap`/`munmap`. |
| D-Bus no **app de controle** (portal de atalhos) | **`zbus` 5.x** direto, sem `ashpd` | `zbus` 5.19, MIT; 66 crates externas medidas no `shortcuts-portal` (inclui proc-macros: `serde_derive`, `syn`, `zvariant_derive`); licenças MIT, Apache-2.0 ou Unicode-3.0; o `ashpd` 0.13 soma ~7 a mais | Fica fora do jogo, então o critério é menos duro. O `ashpd` é 0.x (quebra de API frequente) e é um invólucro fino do `zbus`; o protocolo do portal que usamos são 3 chamadas. Adotado em 2026-10-09, isolado no crate `shortcuts-portal` (nenhum outro crate depende dele). |
| Leitura do manifesto do bundle (na **layer**) | **`serde` + `serde_json`** 1.x, `std`, sem outras features | Medido 2026-10-09: 6 crates no binário final (`serde`, `serde_core`, `serde_json`, `itoa`, `memchr`, `zmij`); MIT OR Apache-2.0 (`memchr`: Unlicense OR MIT). `serde_derive`/`syn` só em tempo de compilação | Isolado no crate `fx-bundle`; a layer depende só dele. O `serde_json` só **lê** (o bundle é escrito pela ferramenta C++). |
| Testes do `postfx` (só dev) | `ash` com feature `loaded` (traz `libloading`) | Só em `[dev-dependencies]`; não entra na layer | Permite rodar o passe numa GPU de software (lavapipe) sem janela. |
| Descartada | `libc` (1 crate) | Última estável é a 0.2.x; a 1.0 está em alpha | `rustix` dá API segura para criar/fechar mapeamentos com menos `unsafe` nosso. |

## Avaliação: toolkit da interface gráfica (proposta, aguardando decisão)

Medido em 2026-10-09 com `cargo tree` (só dependências normais), em projetos de teste em `.tmp/dep-eval/ui/`.
A interface roda **no app de controle** (fora do jogo), então o critério é o menos duro, mas o número de crates e
o comportamento no Wayland importam.

| Opção | Crates | Pacotes de sistema para compilar | Observações |
|---|---|---|---|
| **GTK4** (`gtk4-rs` 0.11, MIT, org gtk-rs) | **50** | `libgtk-4-dev` (hoje **ausente** nesta máquina) | Toolkit nativo do GNOME: Wayland, HiDPI, tema escuro e gerenciamento de janela corretos. Só Linux. |
| Terminal (`ratatui`) | 71 | nenhum | Não é uma janela; sliders possíveis, mas longe do painel que se imagina. |
| `egui` (`eframe`, OpenGL + Wayland) | 127 | nenhum | Parecido com o ImGui do ReShade, multiplataforma; contexto GL próprio no processo de controle. |
| `iced` | 133 | nenhum | |
| `egui` com as opções padrão (wgpu) | 256 | nenhum | Descartado pelo tamanho. |
| Web local (`tiny_http`) | 6 | nenhum | Interface em HTML no navegador. Um servidor local é superfície de ataque (qualquer página aberta no navegador poderia tentar chamá-lo): exigiria token e checagem de `Host`. |

**Proposta: GTK4**, por ter o menor número de crates entre os toolkits de janela e o melhor comportamento
nativo no GNOME/Wayland. O custo é um pacote de sistema (`libgtk-4-dev`, entra no `scripts/dev.sh setup`) e
a interface ficar ligada ao GTK. Para isso não virar retrabalho, a lógica da interface (lista de efeitos,
controles de cada parâmetro, atualização do estado) fica num crate **independente de toolkit**, testável sem
display; trocar o toolkit, ou ter um segundo (por exemplo, para Windows um dia), só exige reescrever a casca.

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
