# Canal entre o app de controle e a layer

Os dois processos conversam por **um arquivo em memória compartilhada**, `/dev/shm/linux-reshade-<uid>/state`
(modo 0700, só do próprio usuário). O app de controle (`lrctl`) escreve; a layer (dentro do jogo) só
lê, uma vez por frame, sem lock e sem syscall. Implementação: crate `ipc`.

Por que `/dev/shm`: os contêineres da Steam (pressure-vessel) não expõem `/run/user/<uid>`, mas
expõem `/dev/shm`. A layer só mapeia um arquivo que pertença ao próprio usuário.

## Conteúdo (versão 3)

| Região | O quê |
|---|---|
| `State` | `magic`, `version`, e `flags` (bit 0: efeitos ligados) |
| `Params` | o bloco de parâmetros, abaixo |
| `Selection` | o efeito escolhido, abaixo |

### Bloco de parâmetros

É a **imagem do buffer de uniforms** do efeito (até 16 KiB), pronta para ser copiada para a GPU, mais o
`bundle_id` do efeito a que pertence (`fx_bundle::Bundle::id`, hash do `manifest.json`).

- **A layer só aplica o bloco se o `bundle_id` for o do bundle que ela carregou.** Valores de outro
  efeito nunca vazam para este. Se o bloco deixa de ser deste efeito depois de aplicado, ela volta
  aos valores padrão do manifesto, uma vez.
- **Protocolo seqlock**: o contador `seq` é ímpar enquanto alguém escreve e par quando o conteúdo é
  consistente. O leitor só aceita uma cópia se `seq` era par e não mudou durante a leitura; se a
  leitura cai no meio de uma escrita, tenta no frame seguinte. Os dados são palavras atômicas
  (`Relaxed`), então não há corrida de dados.
- **Vários escritores** se revezam por CAS no `seq`. Um escritor que morre no meio da escrita (deixa
  o `seq` ímpar) é substituído pelo próximo, depois de 50 ms.
- A layer escreve o bloco inteiro no buffer de uniforms a cada frame e **depois** os uniforms
  alimentados pelo runtime (`timer` etc.), que por isso sempre prevalecem.

### Efeito escolhido

O caminho (UTF-8, até 1 KiB) do diretório do bundle que a layer deve executar, publicado por
`lrctl use <diretório>` (o `lrctl` confere que o bundle carrega antes de publicar). Usa o mesmo
mecanismo seqlock; a *geração* do bloco muda a cada publicação, então a layer olha a cada present só
essa geração (uma leitura atômica) e lê o caminho e o disco apenas quando ela mudou.

- **Ordem de prioridade na layer:** o efeito escolhido com `lrctl use` e, se não houver, a variável
  `LINUX_RESHADE_BUNDLE`. `lrctl use none` desfaz a escolha.
- **Troca em tempo de execução:** quando o bundle muda, a layer espera a GPU ficar ociosa, descarta o
  efeito construído com o bundle antigo e reconstrói com o novo no frame seguinte. Cada troca sobe uma
  *época*; tudo que foi construído a partir do bundle guarda a época e se refaz quando ela muda.
- Os comandos de parâmetros do `lrctl` usam, por padrão, o efeito escolhido (ordem: `--bundle`, efeito
  escolhido, `LINUX_RESHADE_BUNDLE`).

## O que vale enquanto não reiniciar

O estado fica em `/dev/shm`: sobrevive ao fechamento do jogo e do `lrctl`, e some ao reiniciar a
máquina. Portanto o efeito escolhido e os parâmetros alterados continuam valendo da próxima vez que um
jogo carregar a layer, até um `lrctl use none` / `lrctl reset`. O arquivo de uma versão anterior do layout é reinicializado.
