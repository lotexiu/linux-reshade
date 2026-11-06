//! Instalar o linux-reshade na máquina de quem o usa pelo AppImage (ou de qualquer instalação que não seja a do repositório): a camada do
//! Vulkan (a biblioteca fica numa pasta fixa e um manifesto a anuncia ao carregador do Vulkan) e o início do daemon com a sessão.
//!
//! O AppImage monta numa pasta diferente a cada execução; por isso a biblioteca é COPIADA para a pasta de dados e o manifesto aponta
//! para a cópia (que se atualiza quando o programa muda).

use std::path::{Path, PathBuf};

const LAYER_LIB: &str = "liblinux_reshade_layer.so";
const MANIFEST_NAME: &str = "VK_LAYER_LOTEXIU_linux_reshade.json";
const AUTOSTART_NAME: &str = "linux-reshade-daemon.desktop";

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback))).unwrap_or_else(|| PathBuf::from("."))
}

fn data_dir() -> PathBuf {
    xdg("XDG_DATA_HOME", ".local/share")
}

/// Onde o carregador do Vulkan procura as camadas implícitas do usuário.
pub fn manifest_path() -> PathBuf {
    data_dir().join("vulkan/implicit_layer.d").join(MANIFEST_NAME)
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
    std::fs::create_dir_all(target.parent().ok_or("sem pasta")?).map_err(|e| e.to_string())?;
    // Escreve ao lado e troca: um jogo aberto que já mapeou a biblioteca antiga não a vê mudar por baixo.
    let tmp = target.with_extension("so.new");
    std::fs::copy(&source, &tmp).map_err(|e| format!("copiando a camada: {e}"))?;
    std::fs::rename(&tmp, &target).map_err(|e| e.to_string())?;
    let manifest = manifest_path();
    std::fs::create_dir_all(manifest.parent().ok_or("sem pasta")?).map_err(|e| e.to_string())?;
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
    let command = daemon_command().ok_or("não achei o comando do daemon")?;
    std::fs::create_dir_all(path.parent().ok_or("sem pasta")?).map_err(|e| e.to_string())?;
    let desktop = format!("[Desktop Entry]\nType=Application\nName=linux-reshade (daemon)\nComment=Compila os efeitos dos jogos abertos e registra os atalhos\nExec={command}\nTerminal=false\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n");
    std::fs::write(&path, desktop).map_err(|e| e.to_string())
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
