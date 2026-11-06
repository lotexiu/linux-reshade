//! Ligar o linux-reshade a jogos Flatpak, como o Flatseal liga permissões: o app Flatpak roda numa caixa que não vê a layer do sistema,
//! então a layer entra como uma EXTENSÃO do runtime (`org.freedesktop.Platform.VulkanLayer.linux_reshade`, uma por versão do runtime) e o
//! app ganha as permissões e variáveis que a layer precisa (`flatpak override --user`).
//!
//! Tudo passa pelo comando `flatpak` (instalação do usuário): nada de root. Cada função que roda o comando devolve o texto do erro se ele
//! falha; as partes que leem a saída dele são puras e têm testes.

use std::path::{Path, PathBuf};
use std::process::Command;

pub const EXTENSION_ID: &str = "org.freedesktop.Platform.VulkanLayer.linux_reshade";
const EXTENSION_NAME: &str = "linux_reshade";
const LAYER_LIB: &str = "liblinux_reshade_layer.so";
const LAYER_MANIFEST: &str = "VK_LAYER_LOTEXIU_linux_reshade.json";
/// A variável que liga a layer num app (o manifesto da layer a pede com valor 1); `0` num app a desliga só nele.
pub const ENABLE_VAR: &str = "LINUX_RESHADE";

/// Um app Flatpak instalado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    /// `org.gnome.Platform/x86_64/51`.
    pub runtime: String,
}

/// Como um app está em relação ao linux-reshade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Sem nada do linux-reshade.
    Off,
    /// Ligado só neste app (`LINUX_RESHADE=1` na configuração dele).
    On,
    /// Ligado por valer para todos os apps (configuração global).
    Global,
    /// A configuração global liga tudo, e este app a desliga (`LINUX_RESHADE=0`).
    DisabledHere,
}

// ---- partes puras ----

/// Os apps da saída de `flatpak list --app --columns=application,name,runtime` (colunas separadas por tabulação).
pub fn parse_apps(output: &str) -> Vec<App> {
    let mut apps: Vec<App> = Vec::new();
    for line in output.lines() {
        let mut columns = line.split('\t');
        let (Some(id), Some(name), runtime) = (columns.next(), columns.next(), columns.next()) else { continue };
        let id = id.trim();
        // O cabeçalho ("Application ID") e linhas soltas não são apps: um id tem pontos e nenhum espaço.
        if !id.contains('.') || id.contains(' ') || apps.iter().any(|a| a.id == id) {
            continue;
        }
        apps.push(App { id: id.to_string(), name: name.trim().to_string(), runtime: runtime.unwrap_or("").trim().to_string() });
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// A versão do ponto de extensão `VulkanLayer` nos metadados de um runtime (`flatpak info --show-metadata`).
pub fn parse_extension_version(metadata: &str) -> Option<String> {
    let mut inside = false;
    for line in metadata.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[Extension org.freedesktop.Platform.VulkanLayer]";
        } else if inside && let Some(value) = line.strip_prefix("version") {
            return value.trim_start().strip_prefix('=').map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// O valor de `LINUX_RESHADE` na seção `[Environment]` de uma saída de `flatpak override --show`.
pub fn parse_enable_var(show: &str) -> Option<String> {
    let mut inside = false;
    for line in show.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[Environment]";
        } else if inside && let Some(value) = line.strip_prefix(&format!("{ENABLE_VAR}=")) {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// O estado de um app, dado o `override --show` dele e o do global.
pub fn state_from(app_show: &str, global_show: &str) -> State {
    match (parse_enable_var(app_show).as_deref(), parse_enable_var(global_show).as_deref()) {
        (Some("0"), _) => State::DisabledHere,
        (Some("1"), _) => State::On,
        (_, Some("1")) => State::Global,
        _ => State::Off,
    }
}

/// O manifesto da layer como a extensão o instala: o caminho da biblioteca é o de dentro da caixa.
pub fn extension_manifest(layer_name: &str) -> String {
    format!(
        r#"{{
  "file_format_version": "1.2.0",
  "layer": {{
    "name": "{layer_name}",
    "type": "GLOBAL",
    "library_path": "/usr/lib/extensions/vulkan/{EXTENSION_NAME}/lib/{LAYER_LIB}",
    "api_version": "1.3.0",
    "implementation_version": "1",
    "description": "linux-reshade",
    "functions": {{
      "vkNegotiateLoaderLayerInterfaceVersion": "vkNegotiateLoaderLayerInterfaceVersion"
    }},
    "enable_environment": {{ "{ENABLE_VAR}": "1" }},
    "disable_environment": {{ "{ENABLE_VAR}_DISABLE": "1" }}
  }}
}}
"#
    )
}

/// O que um app Flatpak precisa para a layer funcionar: acesso à memória compartilhada (o estado), a pasta dos efeitos compilados (só
/// leitura) e as variáveis. `cache` é a pasta do cache do linux-reshade na máquina.
pub fn grants(cache: &Path) -> Vec<String> {
    vec![
        "--device=shm".to_string(),
        format!("--filesystem={}:ro", cache.display()),
        format!("--env={ENABLE_VAR}=1"),
        "--env=LINUX_RESHADE_DEPTH=1".to_string(),
        "--env=LINUX_RESHADE_LOG=/tmp/linux-reshade-flatpak.log".to_string(),
        "--env=LINUX_RESHADE_SIGNALS=1".to_string(),
    ]
}

/// O inverso de [`grants`].
pub fn revokes(cache: &Path) -> Vec<String> {
    vec![
        "--nodevice=shm".to_string(),
        format!("--nofilesystem={}", cache.display()),
        format!("--unset-env={ENABLE_VAR}"),
        "--unset-env=LINUX_RESHADE_DEPTH".to_string(),
        "--unset-env=LINUX_RESHADE_LOG".to_string(),
        "--unset-env=LINUX_RESHADE_SIGNALS".to_string(),
    ]
}

// ---- o comando `flatpak` ----

fn flatpak(args: &[&str]) -> Result<String, String> {
    let out = Command::new("flatpak").args(args).output().map_err(|e| format!("não consegui rodar o flatpak: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("flatpak {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Há o comando `flatpak` nesta máquina?
pub fn available() -> bool {
    Command::new("flatpak").arg("--version").output().is_ok_and(|o| o.status.success())
}

pub fn installed_apps() -> Result<Vec<App>, String> {
    Ok(parse_apps(&flatpak(&["list", "--app", "--columns=application,name,runtime"])?))
}

fn show(app: Option<&str>) -> String {
    let mut args = vec!["override", "--user", "--show"];
    args.extend(app);
    flatpak(&args).unwrap_or_default()
}

pub fn state(app: &str) -> State {
    state_from(&show(Some(app)), &show(None))
}

/// A configuração global (vale para todos os apps) liga o linux-reshade?
pub fn global_enabled() -> bool {
    parse_enable_var(&show(None)).as_deref() == Some("1")
}

/// A versão do ponto de extensão da layer de gráficos que o runtime de um app oferece.
pub fn extension_version(runtime: &str) -> Result<String, String> {
    let metadata = flatpak(&["info", "--show-metadata", runtime])?;
    parse_extension_version(&metadata).ok_or_else(|| format!("o runtime {runtime} não tem o ponto de extensão VulkanLayer"))
}

// ---- onde estão a layer e os arquivos de trabalho ----

/// A biblioteca da layer: `LINUX_RESHADE_LAYER_LIB`, a que vem junto do programa (`../lib` do executável, como num AppImage) ou a do
/// lado dele (o `target/release` de quem compilou).
pub fn layer_library() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("LINUX_RESHADE_LAYER_LIB").map(PathBuf::from)
        && path.is_file()
    {
        return Ok(path);
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("sem pasta do executável")?;
    [dir.join("../lib").join(LAYER_LIB), dir.join(LAYER_LIB)]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| format!("não achei {LAYER_LIB} (defina LINUX_RESHADE_LAYER_LIB)"))
}

fn work_dir() -> PathBuf {
    let home = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))).unwrap_or_else(|| PathBuf::from("."));
    home.join("linux-reshade/flatpak")
}

fn cache_dir() -> PathBuf {
    config::Config::load().cache_dir.parent().map(Path::to_path_buf).unwrap_or_else(|| config::Config::load().cache_dir)
}

/// Marca da biblioteca (tamanho e data): a extensão instalada só é refeita se a layer mudou.
fn lib_stamp(lib: &Path) -> String {
    let meta = std::fs::metadata(lib).ok();
    let secs = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    format!("{}:{secs}", meta.map_or(0, |m| m.len()))
}

fn remote_name(version: &str) -> String {
    format!("lr-local-{version}")
}

/// Garante a extensão da layer instalada para a versão `version` do runtime (empacota e instala se falta ou se a layer mudou).
pub fn ensure_extension(version: &str) -> Result<(), String> {
    let lib = layer_library()?;
    let base = work_dir();
    let stamp_file = base.join(format!("ext-{version}.stamp"));
    let stamp = lib_stamp(&lib);
    let ref_name = format!("{EXTENSION_ID}//{version}");
    let installed = flatpak(&["--user", "info", &ref_name]).is_ok();
    if installed && std::fs::read_to_string(&stamp_file).is_ok_and(|s| s == stamp) {
        return Ok(());
    }
    let ext = base.join(format!("ext-{version}"));
    let repo = base.join(format!("repo-{version}"));
    let _ = std::fs::remove_dir_all(&ext);
    std::fs::create_dir_all(ext.join("files/lib")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(ext.join("files/share/vulkan/implicit_layer.d")).map_err(|e| e.to_string())?;
    std::fs::copy(&lib, ext.join("files/lib").join(LAYER_LIB)).map_err(|e| format!("copiando a layer: {e}"))?;
    std::fs::write(
        ext.join("metadata"),
        format!("[Runtime]\nname={EXTENSION_ID}\nruntime=org.freedesktop.Platform/x86_64/{version}\nsdk=org.freedesktop.Sdk/x86_64/{version}\n\n[ExtensionOf]\nref=runtime/org.freedesktop.Platform/x86_64/{version}\n"),
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(ext.join("files/share/vulkan/implicit_layer.d").join(LAYER_MANIFEST), extension_manifest("VK_LAYER_LOTEXIU_linux_reshade")).map_err(|e| e.to_string())?;
    // Um repositório novo a cada vez: reaproveitar o antigo deixava o flatpak instalando o commit velho ("nada a atualizar").
    let _ = flatpak(&["--user", "uninstall", "-y", &ref_name]);
    let remote = remote_name(version);
    let _ = flatpak(&["--user", "remote-delete", "--force", &remote]);
    // A configuração de antes (um repositório só para todas as versões) sai: o app só usa os novos.
    let _ = flatpak(&["--user", "remote-delete", "--force", "lr-local"]);
    let _ = std::fs::remove_dir_all(&repo);
    let (repo_s, ext_s) = (repo.to_string_lossy().into_owned(), ext.to_string_lossy().into_owned());
    flatpak(&["build-export", "--runtime", &repo_s, &ext_s, version])?;
    flatpak(&["--user", "remote-add", "--no-gpg-verify", &remote, &repo_s])?;
    flatpak(&["--user", "install", "-y", &remote, &ref_name])?;
    std::fs::write(&stamp_file, stamp).map_err(|e| e.to_string())?;
    Ok(())
}

fn override_with(app: Option<&str>, flags: &[String]) -> Result<(), String> {
    let mut args: Vec<&str> = vec!["override", "--user"];
    args.extend(flags.iter().map(String::as_str));
    args.extend(app);
    flatpak(&args).map(drop)
}

/// Liga o linux-reshade neste app: extensão da versão do runtime dele, permissões e variáveis.
pub fn enable(app: &App) -> Result<(), String> {
    ensure_extension(&extension_version(&app.runtime)?)?;
    override_with(Some(&app.id), &grants(&cache_dir()))
}

/// Desfaz o que [`enable`] fez neste app (a extensão fica: outros apps e a configuração global podem usá-la).
pub fn disable(app_id: &str) -> Result<(), String> {
    override_with(Some(app_id), &revokes(&cache_dir()))
}

/// Desliga só neste app, mantendo a configuração global (o equivalente a pôr `LINUX_RESHADE=0` no Flatseal).
pub fn disable_here(app_id: &str) -> Result<(), String> {
    override_with(Some(app_id), &[format!("--env={ENABLE_VAR}=0")])
}

/// Liga em TODOS os apps Flatpak (a configuração global, como a do Flatseal): a extensão de cada versão de runtime em uso e as
/// permissões e variáveis na configuração global. Um app pode se excluir com `LINUX_RESHADE=0`.
pub fn enable_global() -> Result<(), String> {
    let mut versions: Vec<String> = Vec::new();
    for app in installed_apps()? {
        if let Ok(v) = extension_version(&app.runtime)
            && !versions.contains(&v)
        {
            versions.push(v);
        }
    }
    for v in &versions {
        ensure_extension(v)?;
    }
    override_with(None, &grants(&cache_dir()))
}

pub fn disable_global() -> Result<(), String> {
    override_with(None, &revokes(&cache_dir()))
}

/// Tira a extensão de todas as versões (depois de desligar tudo).
pub fn remove_extensions() {
    if let Ok(out) = flatpak(&["--user", "list", "--runtime", "--columns=ref"]) {
        for line in out.lines().map(str::trim).filter(|l| l.contains(EXTENSION_ID)) {
            let _ = flatpak(&["--user", "uninstall", "-y", line]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_app_list_skips_the_header_and_sorts_by_name() {
        let out = "Application ID\tName\tRuntime\ncom.discordapp.Discord\tDiscord\torg.freedesktop.Platform/x86_64/26.08\norg.vinegarhq.Sober\tSober\torg.gnome.Platform/x86_64/51\nai.lmstudio.lm-studio\tLM Studio\torg.freedesktop.Platform/x86_64/25.08\n";
        let apps = parse_apps(out);
        assert_eq!(apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Discord", "LM Studio", "Sober"]);
        assert_eq!(apps[2], App { id: "org.vinegarhq.Sober".into(), name: "Sober".into(), runtime: "org.gnome.Platform/x86_64/51".into() });
    }

    #[test]
    fn the_vulkan_layer_extension_version_is_read_from_the_runtime_metadata() {
        let metadata = "[Runtime]\nname=org.gnome.Platform\n\n[Extension org.freedesktop.Platform.GL]\nversion = 24.08\n\n[Extension org.freedesktop.Platform.VulkanLayer]\nversion = 26.08\ndirectory = lib/extensions/vulkan\n";
        assert_eq!(parse_extension_version(metadata).as_deref(), Some("26.08"));
        assert_eq!(parse_extension_version("[Runtime]\nname=x\n"), None);
    }

    #[test]
    fn the_state_of_an_app_combines_its_own_override_and_the_global_one() {
        let on = "[Context]\ndevices=shm;\n\n[Environment]\nLINUX_RESHADE=1\nLINUX_RESHADE_DEPTH=1\n";
        let off = "[Context]\ndevices=shm;\n\n[Environment]\nLINUX_RESHADE=0\n";
        let none = "[Context]\nshared=ipc\n";
        assert_eq!(state_from(on, none), State::On);
        assert_eq!(state_from(none, none), State::Off);
        assert_eq!(state_from(none, on), State::Global);
        assert_eq!(state_from(off, on), State::DisabledHere);
        assert_eq!(state_from(on, off), State::On, "ligado no app vale mesmo sem o global");
        // `LINUX_RESHADE_DEPTH=1` não é `LINUX_RESHADE=1`.
        assert_eq!(parse_enable_var("[Environment]\nLINUX_RESHADE_DEPTH=1\n"), None);
    }

    #[test]
    fn the_extension_manifest_points_inside_the_sandbox_and_grants_match_revokes() {
        let manifest = extension_manifest("VK_LAYER_X");
        assert!(manifest.contains("/usr/lib/extensions/vulkan/linux_reshade/lib/liblinux_reshade_layer.so"));
        assert!(manifest.contains("\"LINUX_RESHADE\": \"1\""));
        let cache = Path::new("/home/u/.cache/linux-reshade");
        let (g, r) = (grants(cache), revokes(cache));
        assert!(g.contains(&"--filesystem=/home/u/.cache/linux-reshade:ro".to_string()) && r.contains(&"--nofilesystem=/home/u/.cache/linux-reshade".to_string()));
        assert_eq!(g.len(), r.len(), "cada permissão dada tem a sua retirada");
    }
}
