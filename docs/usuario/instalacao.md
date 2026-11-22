# Instalar e usar sem comandos

## AppImage
`scripts/appimage.sh` monta `dist/linux-reshade-<versão>-x86_64.AppImage` (painel, `lrctl`, a camada do Vulkan, o compilador de efeitos
e os efeitos que vêm com o programa). Ele baixa o `linuxdeploy` (com o plugin do GTK) e o `appimagetool` para `dist/tools` na primeira
vez. `scripts/appimage.sh --appdir-only` monta só a pasta `dist/AppDir`, sem baixar nada, para testar o `AppRun`.

A distribuição que roda o AppImage precisa ter a glibc pelo menos tão nova quanto a da máquina que o montou.

Ao abrir o AppImage (duplo clique):
1. cria a pasta de dados (`~/.local/share/linux-reshade`, com Shaders, Textures, Presets e `config.ini`) e copia os efeitos que vêm junto,
   se ela ainda não existe;
2. instala a camada do Vulkan para o seu usuário (a biblioteca vai para `~/.local/share/linux-reshade/lib`, uma pasta fixa, e um manifesto
   em `~/.local/share/vulkan/implicit_layer.d` a anuncia: o AppImage monta noutra pasta a cada execução);
3. inicia o daemon se não há um rodando (um só: ele segura a trava `daemon.lock`).

Ao abrir, o painel confere o daemon: um daemon de **outra build** (por exemplo o que sobrou de um AppImage anterior) é trocado pelo desta
build. O daemon publica na trava (`daemon.lock`) o carimbo do próprio executável, e `lrctl daemon-ensure` (que o `scripts/dev.sh panel` usa)
faz a mesma conferência; `lrctl daemon-stop` o encerra. A camada do Vulkan também é copiada de novo quando difere da que acompanha o programa.

A aba **Sistema** das Configurações mostra o estado de cada item e liga o início do daemon com a sessão. A linha de comando continua
disponível: `./linux-reshade.AppImage setup`, `... --lrctl flatpak list`, `... --daemon`.

Para um jogo que **não** é Flatpak, ligue a camada com `LINUX_RESHADE=1` nas opções de lançamento (Steam: `LINUX_RESHADE=1 %command%`).

## Jogos Flatpak
Um app Flatpak roda numa caixa que não vê a camada do sistema. O painel faz o que o Flatseal faria, e mais:
- **Jogo ▸ "＋ Adicionar jogo Flatpak…"** (último item da lista de jogos): lista os apps instalados. **Ativar** instala a extensão da camada
  para o runtime do app (`org.freedesktop.Platform.VulkanLayer.linux_reshade`, uma por versão) e dá ao app a memória compartilhada, a
  leitura da pasta dos efeitos compilados e as variáveis `LINUX_RESHADE*`. Vale na próxima vez que o app abrir.
- **Remover alterações…** (no app ligado) avisa o que será desfeito e desfaz só as permissões e variáveis deste app.
- Quando o jogo escolhido na lista é um app Flatpak ligado, aparece o botão **Flatpak…** ao lado dele.
- **Configurações ▸ Flatpak ▸ Aplicar a todos os apps Flatpak**: liga na configuração global (como a "todos os aplicativos" do Flatseal).
  Para tirar um app só, use "Desligar neste app" ou, no Flatseal, a variável `LINUX_RESHADE=0` nele.

Pela linha de comando: `lrctl flatpak list`, `lrctl flatpak enable|disable <app-id>`, `lrctl flatpak global on|off`.
