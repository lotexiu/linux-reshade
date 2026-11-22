//! Marcadores na pasta de estado que a layer lê: "processando" (o vigia compilando), o controle que a pessoa edita (overlay) e a medição ligada.

use super::*;

/// Quem está compilando efeitos para os jogos (o vigia do daemon) deixa um marcador `busy-<pid>-<n>` na pasta de estado enquanto trabalha: a layer
/// o vê e mostra o ícone de "processando" no jogo. O marcador some ao soltar o guarda; um que ficou para trás (processo morto) vale só por
/// [`BUSY_MAX_AGE`].
pub struct BusyGuard {
    path: Option<PathBuf>,
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = fs::remove_file(path);
        }
    }
}

/// Quanto tempo um marcador de "processando" é crível sem ser renovado.
pub const BUSY_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(120);

pub fn busy_guard_in(dir: &std::path::Path) -> BusyGuard {
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if ensure_private_dir(dir).is_err() {
        return BusyGuard { path: None };
    }
    let path = dir.join(format!("busy-{}-{}", std::process::id(), COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    BusyGuard { path: fs::write(&path, b"").ok().map(|()| path) }
}

pub fn busy_guard() -> BusyGuard {
    busy_guard_in(&state_dir())
}

/// Há alguém compilando para os jogos agora?
pub fn busy_now_in(dir: &std::path::Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    entries.flatten().any(|e| {
        e.file_name().to_string_lossy().starts_with("busy-") && e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < BUSY_MAX_AGE)
    })
}

pub fn busy_now() -> bool {
    busy_now_in(&state_dir())
}

/// O controle que a pessoa está editando no painel, para os efeitos que mostram um auxílio visual enquanto se edita
/// (`source = "overlay_active"`, como os pontos do autofoco do Depth of Field). O arquivo guarda o efeito e o uniform; vale
/// por pouco tempo depois da última edição.
fn overlay_file(dir: &std::path::Path, game: &str) -> Option<PathBuf> {
    (!game.is_empty() && !game.starts_with('.') && !game.contains(['/', '\\', '\0'])).then(|| dir.join(format!("overlay-{game}")))
}

pub fn set_overlay_in(dir: &std::path::Path, game: &str, effect: &str, uniform: &str) -> io::Result<()> {
    let Some(file) = overlay_file(dir, game) else { return Ok(()) };
    ensure_private_dir(dir)?;
    fs::write(file, format!("{effect}\n{uniform}\n"))
}

pub fn set_overlay(game: &str, effect: &str, uniform: &str) -> io::Result<()> {
    set_overlay_in(&state_dir(), game, effect, uniform)
}

/// `(efeito, uniform)` editado nos últimos `max_age`; `None` se faz mais tempo (ou nunca).
pub fn overlay_in(dir: &std::path::Path, game: &str, max_age: std::time::Duration) -> Option<(String, String)> {
    let file = overlay_file(dir, game)?;
    let age = fs::metadata(&file).and_then(|m| m.modified()).ok()?.elapsed().unwrap_or_default();
    if age > max_age {
        return None;
    }
    let text = fs::read_to_string(file).ok()?;
    let mut lines = text.lines();
    Some((lines.next()?.to_string(), lines.next()?.to_string()))
}

pub fn overlay(game: &str, max_age: std::time::Duration) -> Option<(String, String)> {
    overlay_in(&state_dir(), game, max_age)
}

/// A medição de desempenho da layer liga e desliga por um marcador no estado compartilhado (medir custa um pouco: a layer só mede
/// quando alguém pediu).
const STATS_FILE: &str = "stats-on";

pub fn stats_on_in(dir: &std::path::Path) -> bool {
    dir.join(STATS_FILE).exists()
}

pub fn stats_on() -> bool {
    stats_on_in(&state_dir())
}

pub fn set_stats_on_in(dir: &std::path::Path, on: bool) -> io::Result<()> {
    match (on, stats_on_in(dir)) {
        (true, false) => {
            ensure_private_dir(dir)?;
            fs::write(dir.join(STATS_FILE), b"")
        }
        (false, true) => fs::remove_file(dir.join(STATS_FILE)),
        _ => Ok(()),
    }
}

pub fn set_stats_on(on: bool) -> io::Result<()> {
    set_stats_on_in(&state_dir(), on)
}
