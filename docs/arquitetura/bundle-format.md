# Formato do bundle (versão 1)

Um **bundle** é um diretório produzido pela ferramenta `fx2spv` a partir de um efeito `.fx` e lido
pela layer. A layer nunca compila FX; só lê o bundle.

```
meu-efeito/
  manifest.json     descrição do efeito (este documento)
  <entry>.spv       um módulo SPIR-V por entry point (vertex ou pixel), nomeado em `entry_points`
```

## Contrato com o compilador (reshadefx, modo Vulkan)

Cada módulo SPIR-V contém **um** entry point. Os recursos usam três *descriptor sets* fixos:

| Set | Binding | O quê |
|---|---|---|
| 0 | 0 | buffer global de uniforms (std140); cada uniform em `offset` bytes, `total_uniform_size` no total |
| 1 | `b` | sampler (`COMBINED_IMAGE_SAMPLER`); o `b` de cada sampler **é por passe**, ver `Pass.samplers` |
| 2 | `j` | imagem de armazenamento (**não suportado na v1**: a ferramenta recusa o efeito) |

O compilador renumera os bindings por entry point e os alinha entre o vertex e o pixel shader de
um mesmo passe, então o `Pass.samplers` vale para os dois estágios. Passes de *compute* **não são
suportados na v1**. O compilador já recusa amostrar uma textura que é também alvo do mesmo passe.

## manifest.json

Todos os nomes de campo são `snake_case`. Campos desconhecidos devem ser ignorados pelo leitor
(compatibilidade para frente); mudar o significado de um campo existente exige subir `version`.

```jsonc
{
  "format": "linux-reshade-bundle",
  "version": 1,
  "source": "Sepia.fx",
  "compiler": { "name": "reshadefx", "reshade_commit": "…", "spirv_headers_commit": "…" },
  // Valores das macros BUFFER_* com que o efeito foi compilado. O runtime só usa o bundle se a
  // resolução do swapchain for a mesma (o efeito pode ter constantes dependentes dela).
  "target": { "width": 1920, "height": 1080, "color_space": "srgb" },
  "defines": { "NOME": "valor" },          // macros do usuário (-D) que afetaram a compilação
  "spirv_version": "1.0",                  // versão do SPIR-V dos módulos ("1.0" ou "1.3")
  "y_flipped": true,                       // o vertex shader já inverte o Y (D3D -> Vulkan); o runtime exige true
  "entry_points": [ { "name": "PS_Sepia", "stage": "pixel", "file": "PS_Sepia.spv" } ],
  "total_uniform_size": 16,
  "textures":  [ Texture ],
  "samplers":  [ Sampler ],
  "uniforms":  [ Uniform ],
  "techniques":[ Technique ]
}
```

**Texture**: `{ name, unique_name, semantic, width, height, depth, levels, type, format, render_target, annotations }`
- `semantic`: `"COLOR"` (o frame do jogo), `"DEPTH"` (buffer de profundidade) ou `null`.
  A v1 só suporta `COLOR`; um bundle que use `DEPTH` é recusado até o marco 5.
- `type`: `"1d" | "2d" | "3d"` (v1: só `"2d"`). `format`: `r8, r16f, r16, r32f, r32u, r32i, rg8, rg16f, rg16, rg32f, rgba8, rgba16f, rgba16, rgba32f, rgba32u, rgba32i, rgb10a2, rg11b10f, unknown`.

**Sampler**: `{ name, unique_name, texture, min_filter, mag_filter, mip_filter, anisotropic, address_u, address_v, address_w, min_lod, max_lod, lod_bias, srgb }`
- `texture` é o `unique_name` de uma textura. O índice do sampler é a posição dele em `samplers`.
- filtros: `"point" | "linear"`; endereçamento: `"wrap" | "mirror" | "clamp" | "border"`.

**Uniform**: `{ name, unique_name, type, offset, size, default, annotations }`
- `type`: `{ base: "bool"|"int"|"uint"|"float", rows, cols, array }` (`array` 0 = não é array).
- `default`: valor inicial (escalar ou lista de `rows*cols` números) ou `null`.

**Technique**: `{ name, annotations, passes: [ Pass ] }`

**Pass**: `{ name, vertex_entry, pixel_entry, compute_entry, render_targets, samplers, clear, srgb_write, blend, topology, num_vertices, viewport }`
- `samplers`: `[ { sampler, binding } ]`: quais samplers o passe usa (índice em `samplers`) e o binding de cada um no set 1.
- `render_targets`: lista de `unique_name` de textura, na ordem dos anexos; `[]` (ou `null` no item)
  significa "o frame do jogo" (o alvo final).
- `blend`: por anexo, `{ enable, src_color, dst_color, color_op, src_alpha, dst_alpha, alpha_op, write_mask }`.
- `viewport`: `[largura, altura]`, `[0, 0]` = tamanho do alvo.

**annotations** (em qualquer objeto): mapa `nome -> valor`, onde o valor é string, número, booleano
ou lista de números. É o que a interface usa (`ui_label`, `ui_type`, `ui_min`, `ui_max`, …).

## Uniforms alimentados pelo runtime (`source`)

Um uniform com a anotação `source` não é um controle: o runtime o preenche a cada frame.

| `source` | Valor |
|---|---|
| `timer` | milissegundos desde o início do jogo |
| `frametime` | milissegundos que o frame anterior levou |
| `framecount` | número do frame |
| `date` | `float4(ano, mês, dia, segundos do dia)` em UTC |
| `random` | inteiro ou float entre `min` e `max`, novo a cada frame |
| `pingpong` | `float2(valor, direção)` oscilando entre `min` e `max` a `step` por segundo |

Os demais (`key`, `mousebutton`, `mousepoint`, `mousedelta`, `mousewheel`, `overlay_*`,
`bufready_depth`, `screenshot`) **não são alimentados** (a layer não lê entrada por decisão de
projeto): ficam no valor padrão do manifesto, e a layer registra quais são.

