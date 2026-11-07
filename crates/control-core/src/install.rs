//! Instalar o linux-reshade na máquina de quem o usa pelo AppImage (ou de qualquer instalação que não seja a do repositório): a camada do
//! Vulkan (a biblioteca fica numa pasta fixa e um manifesto a anuncia ao carregador do Vulkan) e o início do daemon com a sessão.
//!
//! O AppImage monta numa pasta diferente a cada execução; por isso a biblioteca é COPIADA para a pasta de dados e o manifesto aponta
//! para a cópia (que se atualiza quando o programa muda).

use std::path::{Path, PathBuf};
use i18n::{fmt, tr};

const LAYER_LIB: &str = "liblinux_reshade_layer.so";
const MANIFEST_NAME: &str = "VK_LAYER_LOTEXIU_linux_reshade.json";
const AUTOSTART_NAME: &str = "linux-reshade-daemon.desktop";

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback))).unwrap_or_else(|| PathBuf::from("."))
}

fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

/// A pasta dos recursos que vêm com o programa (o `fx2spv`, os efeitos e os cabeçalhos do ReShade), com a mesma disposição do
/// repositório: `LINUX_RESHADE_RESOURCES`; a `../share/linux-reshade` do executável (AppImage e instalações); ou o próprio repositório
/// (`target/release/lrctl` dentro dele).
pub fn resource_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LINUX_RESHADE_RESOURCES").map(PathBuf::from).filter(|d| d.is_dir()) {
        return Some(dir);
    }
    let exe = std::env::current_exe().ok()?;
    let installed = exe.parent()?.join("../share/linux-reshade");
    if installed.is_dir() {
        return installed.canonicalize().ok().or(Some(installed));
    }
    exe.ancestors().nth(3).map(Path::to_path_buf)
}

/// Primeira execução fora do repositório: cria a pasta de dados (Shaders, Textures, Presets, config.ini) e, se os Shaders estão vazios,
/// copia para eles os efeitos que vêm com o programa. Não faz nada se o usuário já tem uma pasta de dados configurada.
pub fn first_run() -> Result<(), String> {
    let env = |name: &str| std::env::var_os(name);
    // A pasta de dados do usuário (a configurada, ou a padrão) já tem Shaders: nada a fazer.
    let dir = config::Config::load().data_dir.or_else(|| config::default_data_dir(&env)).ok_or(tr("do not know where your data lives (HOME is empty)"))?;
    if dir.join("Shaders").is_dir() {
        return Ok(());
    }
    config::init(&dir, &env).map_err(|e| format!("{}: {e}", dir.display()))?;
    let Some(resources) = resource_root() else { return Ok(()) };
    let (shaders, textures) = (dir.join("Shaders"), dir.join("Textures"));
    let empty = std::fs::read_dir(&shaders).map(|mut d| d.next().is_none()).unwrap_or(true);
    if empty {
        for from in ["effects", "third_party/reshade-shaders-slim/Shaders", "third_party/reshade-shaders-legacy/Shaders"] {
            copy_tree(&resources.join(from), &shaders);
        }
        for from in ["third_party/reshade-shaders-slim/Textures", "third_party/reshade-shaders-legacy/Textures"] {
            copy_tree(&resources.join(from), &textures);
        }
    }
    Ok(())
}

/// Copia `from` para dentro de `to` sem sobrescrever o que já existe.
fn copy_tree(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    let _ = std::fs::create_dir_all(to);
    for entry in entries.flatten() {
        let (path, target) = (entry.path(), to.join(entry.file_name()));
        if path.is_dir() {
            copy_tree(&path, &target);
        } else if !target.exists() {
            let _ = std::fs::copy(&path, &target);
        }
    }
}

/// Onde o carregador do Vulkan procura as camadas implícitas do usuário.
pub fn manifest_path() -> PathBuf {
    data_dir().join("vulkan/implicit_layer.d").join(MANIFEST_NAME)
}

/// O identificador do programa (o mesmo do painel e do portal de atalhos).
pub const APP_ID: &str = "dev.lotexiu.LinuxReshade";

fn desktop_entry_path() -> PathBuf {
    data_dir().join("applications").join(format!("{APP_ID}.desktop"))
}

/// O ícone do programa (o do ReShade; ver `packaging/ICON-NOTICE.md`), gravado na pasta de ícones do programa. Devolve o caminho.
pub fn ensure_icon() -> Option<PathBuf> {
    const SVG: &str = include_str!("../../../packaging/linux-reshade.svg");
    let path = data_dir().join("linux-reshade").join("icons/hicolor/scalable/apps/linux-reshade.svg");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(SVG) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, SVG).ok()?;
    }
    Some(path)
}

/// Cria (ou atualiza) `~/.local/share/applications/dev.lotexiu.LinuxReshade.desktop`. O portal de atalhos só aceita que um programa sem `app id`
/// (como o daemon de um AppImage, que não herda o escopo de nenhum programa) se identifique se existe um `.desktop` com o nome do `app id`;
/// sem ele o registro dos atalhos falha com `An app id is required` e o KDE nem mostra a janela de confirmação. Não aparece no menu.
pub fn ensure_desktop_entry() {
    let exec = if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|a| !a.is_empty()) {
        format!("{:?}", appimage.to_string_lossy())
    } else if let Some(lrpanel) = std::env::current_exe().ok().and_then(|e| Some(e.parent()?.join("lrpanel"))).filter(|p| p.is_file()) {
        format!("{:?}", lrpanel.display().to_string())
    } else {
        return;
    };
    let icon = ensure_icon().map_or_else(|| "linux-reshade".to_string(), |p| p.display().to_string());
    let desktop = format!(
        "[Desktop Entry]\nType=Application\nName=linux-reshade\nComment={}\nExec={exec}\nIcon={icon}\nTerminal=false\nNoDisplay=true\nStartupWMClass={APP_ID}\n",
        tr("ReShade-compatible Vulkan layer for Linux")
    );
    let path = desktop_entry_path();
    if std::fs::read_to_string(&path).ok().as_deref() != Some(desktop.as_str()) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, desktop);
    }
}

/// A cópia da biblioteca que o manifesto anuncia.
pub fn installed_library() -> PathBuf {
    data_dir().join("linux-reshade/lib").join(LAYER_LIB)
}

fn autostart_path() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("autostart").join(AUTOSTART_NAME)
}

/// O manifesto da camada para a biblioteca em `library`.
pub fn manifest_json(library: &Path) -> String {
    format!(
        r#"{{
  "file_format_version": "1.2.0",
  "layer": {{
    "name": "VK_LAYER_LOTEXIU_linux_reshade",
    "type": "GLOBAL",
    "library_path": {library:?},
    "api_version": "1.3.0",
    "implementation_version": "1",
    "description": "linux-reshade",
    "functions": {{
      "vkNegotiateLoaderLayerInterfaceVersion": "vkNegotiateLoaderLayerInterfaceVersion"
    }},
    "enable_environment": {{ "LINUX_RESHADE": "1" }},
    "disable_environment": {{ "LINUX_RESHADE_DISABLE": "1" }}
  }}
}}
"#,
        library = library.display().to_string()
    )
}

/// A camada está instalada e atual (o manifesto aponta para a cópia, e a cópia é igual à biblioteca que acompanha o programa)?
pub fn layer_installed() -> bool {
    let Ok(source) = crate::flatpak::layer_library() else { return false };
    let target = installed_library();
    let same = std::fs::read(&source).ok().zip(std::fs::read(&target).ok()).is_some_and(|(a, b)| a == b);
    let manifest = std::fs::read_to_string(manifest_path()).unwrap_or_default();
    same && manifest.contains(&target.display().to_string())
}

/// Instala (ou atualiza) a camada do Vulkan: copia a biblioteca e escreve o manifesto. Devolve a pasta da biblioteca.
pub fn install_layer() -> Result<PathBuf, String> {
    let source = crate::flatpak::layer_library()?;
    let target = installed_library();
    std::fs::create_dir_all(target.parent().ok_or(tr("no folder"))?).map_err(|e| e.to_string())?;
    // Escreve ao lado e troca: um jogo aberto que já mapeou a biblioteca antiga não a vê mudar por baixo.
    let tmp = target.with_extension("so.new");
    std::fs::copy(&source, &tmp).map_err(|e| fmt("copying the layer: {}", &[&e]))?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    let manifest = manifest_path();
    std::fs::create_dir_all(manifest.parent().ok_or(tr("no folder"))?).map_err(|e| e.to_string())?;
    std::fs::write(&manifest, manifest_json(&target)).map_err(|e| format!("{}: {e}", manifest.display()))?;
    Ok(target)
}

/// Tira o manifesto (e a cópia): a camada deixa de existir para o Vulkan.
pub fn uninstall_layer() {
    let _ = std::fs::remove_file(manifest_path());
    let _ = std::fs::remove_file(installed_library());
}

/// O comando que inicia o daemon: o AppImage (`$APPIMAGE --daemon`) ou o `lrctl` ao lado do executável.
pub fn daemon_command() -> Option<String> {
    if let Some(appimage) = std::env::var_os("APPIMAGE").filter(|a| !a.is_empty()) {
        return Some(format!("{:?} --daemon", appimage.to_string_lossy()));
    }
    let exe = std::env::current_exe().ok()?;
    let lrctl = exe.parent()?.join("lrctl");
    lrctl.is_file().then(|| format!("{:?} daemon", lrctl.display().to_string()))
}

pub fn autostart_enabled() -> bool {
    autostart_path().is_file()
}

/// Liga ou desliga o início do daemon com a sessão (um `.desktop` em `~/.config/autostart`).
pub fn set_autostart(on: bool) -> Result<(), String> {
    let path = autostart_path();
    if !on {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let command = daemon_command().ok_or(tr("could not find the daemon command"))?;
    std::fs::create_dir_all(path.parent().ok_or(tr("no folder"))?).map_err(|e| e.to_string())?;
    let desktop = format!("[Desktop Entry]\nType=Application\nName=linux-reshade (daemon)\nComment={}\nExec={command}\nTerminal=false\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n", tr("Compiles the effects of open games and registers the shortcuts"));
    std::fs::write(&path, desktop).map_err(|e| e.to_string())
}

/// O `lrctl` que acompanha este programa (o que o painel iniciaria).
fn own_lrctl() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("lrctl")).filter(|p| p.is_file())
}

/// Há um daemon rodando que não é da build deste programa (sobrou de uma versão anterior)?
pub fn daemon_outdated() -> bool {
    let Some(running) = ipc::daemon_lock::running_stamp() else { return false };
    own_lrctl().and_then(|p| ipc::daemon_lock::exe_stamp(&p)).is_some_and(|own| own != running)
}

/// Os processos filhos diretos de `pid` (o daemon tem o `lrctl keys`).
fn children_of(pid: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter(|child| {
            // "pid (comm) estado ppid ...": o nome pode ter espaços e parênteses, então se conta a partir do último ')'.
            std::fs::read_to_string(format!("/proc/{child}/stat")).ok().and_then(|s| s.rsplit_once(')').map(|(_, rest)| rest.split_whitespace().nth(1).and_then(|p| p.parse::<u32>().ok()))).flatten() == Some(pid)
        })
        .collect()
}

/// Encerra o daemon em execução (e o processo dos atalhos dele) e espera a trava ser solta. Devolve se não há mais daemon.
pub fn stop_daemon() -> bool {
    let Some(pid) = ipc::daemon_lock::holder_pid() else { return !ipc::daemon_lock::is_running() };
    let term = |p: u32| {
        let _ = std::process::Command::new("kill").arg("-TERM").arg(p.to_string()).status();
    };
    for child in children_of(pid) {
        term(child);
    }
    term(pid);
    for _ in 0..40 {
        if !ipc::daemon_lock::is_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    false
}

/// O que `ensure_daemon_current` fez.
#[derive(Debug, PartialEq, Eq)]
pub enum DaemonAction {
    /// Já havia um daemon desta build.
    Kept,
    Started,
    /// Havia um de outra build: foi trocado.
    Replaced,
    /// Não deu para iniciar (ou o antigo não quis sair).
    Failed,
}

/// Garante um daemon desta build: inicia se não há, troca se há um de outra build.
pub fn ensure_daemon_current() -> DaemonAction {
    let outdated = daemon_outdated();
    if outdated && !stop_daemon() {
        return DaemonAction::Failed;
    }
    if start_daemon_if_needed() {
        return if outdated { DaemonAction::Replaced } else { DaemonAction::Started };
    }
    if ipc::daemon_lock::is_running() { DaemonAction::Kept } else { DaemonAction::Failed }
}

/// Inicia o daemon em segundo plano, se não há um rodando. Devolve se iniciou.
pub fn start_daemon_if_needed() -> bool {
    if ipc::daemon_lock::is_running() {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else { return false };
    let Some(lrctl) = exe.parent().map(|d| d.join("lrctl")).filter(|p| p.is_file()) else { return false };
    use std::os::unix::process::CommandExt;
    let log = std::fs::OpenOptions::new().create(true).append(true).open(xdg("XDG_STATE_HOME", ".local/state").join("linux-reshade-daemon.out")).ok();
    let mut command = std::process::Command::new(lrctl);
    command.arg("daemon").stdin(std::process::Stdio::null());
    if let Some(log) = log {
        command.stdout(log.try_clone().map_or_else(|_| std::process::Stdio::null(), std::process::Stdio::from)).stderr(std::process::Stdio::from(log));
    }
    // Sessão própria: o daemon continua quando o painel fecha.
    command.process_group(0).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_names_the_installed_copy_and_the_enable_variable() {
        let json = manifest_json(Path::new("/home/u/.local/share/linux-reshade/lib/liblinux_reshade_layer.so"));
        assert!(json.contains("\"library_path\": \"/home/u/.local/share/linux-reshade/lib/liblinux_reshade_layer.so\""), "{json}");
        assert!(json.contains("\"LINUX_RESHADE\": \"1\"") && json.contains("VK_LAYER_LOTEXIU_linux_reshade"));
    }
}
