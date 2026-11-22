//! O que se diz à layer sobre a profundidade de um jogo, por arquivos na pasta de estado: armar a observação, o pedido da pessoa (imagem, passe ou limpeza) e a política.

use super::*;

/// O marcador "este jogo precisa da profundidade" (`depth-arm-<jogo>`): o app de controle o cria quando o preset do
/// jogo tem efeitos que a usam, e a layer o lê ao criar o dispositivo (antes de o jogo criar as imagens), para armar a
/// observação sem a pessoa precisar de `LINUX_RESHADE_DEPTH=1`.
fn depth_arm_file(dir: &std::path::Path, game: &str) -> Option<PathBuf> {
    (!game.is_empty() && !game.starts_with('.') && !game.contains(['/', '\\', '\0'])).then(|| dir.join(format!("depth-arm-{game}")))
}

pub fn depth_arm_wanted_in(dir: &std::path::Path, game: &str) -> bool {
    depth_arm_file(dir, game).is_some_and(|f| f.exists())
}

pub fn depth_arm_wanted(game: &str) -> bool {
    depth_arm_wanted_in(&state_dir(), game)
}

/// Cria ou remove o marcador; não mexe no disco se já está como se pede.
pub fn set_depth_arm_in(dir: &std::path::Path, game: &str, on: bool) -> io::Result<()> {
    let Some(file) = depth_arm_file(dir, game) else { return Ok(()) };
    match (on, file.exists()) {
        (true, false) => {
            ensure_private_dir(dir)?;
            fs::write(file, b"")
        }
        (false, true) => fs::remove_file(file),
        _ => Ok(()),
    }
}

pub fn set_depth_arm(game: &str, on: bool) -> io::Result<()> {
    set_depth_arm_in(&state_dir(), game, on)
}

/// O pedido da pessoa para a profundidade de um jogo: a layer lê o arquivo `depth-pick-<id da layer>` (ver `docs/arquitetura/depth.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthPick {
    /// A heurística da layer.
    #[default]
    Auto,
    /// A N-ésima candidata (de 1) da lista que a layer publica.
    Candidate(u32),
    /// A profundidade logo depois do N-ésimo passe (de 1) que a desenha: a layer faz uma cópia nesse ponto.
    Pass(u32),
    /// A profundidade logo antes da N-ésima limpeza (de 1) dela no quadro, como o "copiar antes de limpar" do ReShade.
    Clear(u32),
}

impl DepthPick {
    /// Como se grava no `game.ini` (`pass:2`, `candidate:1`); `None` para a automática.
    pub fn text(self) -> Option<String> {
        match self {
            DepthPick::Auto => None,
            DepthPick::Candidate(n) => Some(format!("candidate:{n}")),
            DepthPick::Pass(n) => Some(format!("pass:{n}")),
            DepthPick::Clear(n) => Some(format!("clear:{n}")),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let (kind, n) = text.trim().split_once(':')?;
        let n: u32 = n.trim().parse().ok().filter(|n| *n > 0)?;
        match kind.trim() {
            "candidate" => Some(DepthPick::Candidate(n)),
            "pass" => Some(DepthPick::Pass(n)),
            "clear" => Some(DepthPick::Clear(n)),
            _ => None,
        }
    }
}

fn depth_pick_file(dir: &std::path::Path, layer_id: &str) -> Option<PathBuf> {
    (!layer_id.is_empty() && !layer_id.contains(['/', '\\', '\0'])).then(|| dir.join(format!("depth-pick-{layer_id}")))
}

pub fn set_depth_pick_in(dir: &std::path::Path, layer_id: &str, pick: DepthPick) -> io::Result<()> {
    let Some(path) = depth_pick_file(dir, layer_id) else { return Ok(()) };
    match pick {
        DepthPick::Auto => match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
        DepthPick::Candidate(n) => {
            ensure_private_dir(dir)?;
            fs::write(path, format!("candidate={n}\n"))
        }
        DepthPick::Pass(n) => {
            ensure_private_dir(dir)?;
            fs::write(path, format!("pass={n}\n"))
        }
        DepthPick::Clear(n) => {
            ensure_private_dir(dir)?;
            fs::write(path, format!("clear={n}\n"))
        }
    }
}

pub fn set_depth_pick(layer_id: &str, pick: DepthPick) -> io::Result<()> {
    set_depth_pick_in(&state_dir(), layer_id, pick)
}

/// O que a pessoa pediu (o que o arquivo diz; `Auto` se não há arquivo).
pub fn depth_pick_in(dir: &std::path::Path, layer_id: &str) -> DepthPick {
    let Some(path) = depth_pick_file(dir, layer_id) else { return DepthPick::Auto };
    let text = fs::read_to_string(path).unwrap_or_default();
    for line in text.lines() {
        match line.split_once('=') {
            Some(("candidate", n)) => return n.trim().parse().map_or(DepthPick::Auto, DepthPick::Candidate),
            Some(("pass", n)) => return n.trim().parse().map_or(DepthPick::Auto, DepthPick::Pass),
            Some(("clear", n)) => return n.trim().parse().map_or(DepthPick::Auto, DepthPick::Clear),
            _ => {}
        }
    }
    DepthPick::Auto
}

/// Os ajustes de política da profundidade de um jogo (`split`, `copy`, `snapshot` = `on`/`off`), em `depth-policy-<id da layer>`: a layer os
/// lê de tempos em tempos e vale na hora, sem reiniciar o jogo. Chave ausente = o padrão da layer.
pub const DEPTH_POLICY_KEYS: [&str; 4] = ["split", "copy", "snapshot", "resignal"];

fn depth_policy_file(dir: &std::path::Path, layer_id: &str) -> Option<PathBuf> {
    (!layer_id.is_empty() && !layer_id.contains(['/', '\\', '\0'])).then(|| dir.join(format!("depth-policy-{layer_id}")))
}

/// Os ajustes escritos para a layer.
pub fn depth_policy_in(dir: &std::path::Path, layer_id: &str) -> Vec<(String, String)> {
    let Some(path) = depth_policy_file(dir, layer_id) else { return Vec::new() };
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| DEPTH_POLICY_KEYS.contains(&k.trim()))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

pub fn depth_policy(layer_id: &str) -> Vec<(String, String)> {
    depth_policy_in(&state_dir(), layer_id)
}

/// Muda um ajuste (`value` `None`: volta ao padrão da layer).
pub fn set_depth_policy_in(dir: &std::path::Path, layer_id: &str, key: &str, value: Option<&str>) -> io::Result<()> {
    let Some(path) = depth_policy_file(dir, layer_id) else { return Ok(()) };
    if !DEPTH_POLICY_KEYS.contains(&key) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{key}")));
    }
    let mut entries = depth_policy_in(dir, layer_id);
    entries.retain(|(k, _)| k != key);
    if let Some(v) = value {
        entries.push((key.to_string(), v.to_string()));
    }
    if entries.is_empty() {
        return match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    ensure_private_dir(dir)?;
    fs::write(path, entries.iter().map(|(k, v)| format!("{k}={v}\n")).collect::<String>())
}

pub fn set_depth_policy(layer_id: &str, key: &str, value: Option<&str>) -> io::Result<()> {
    set_depth_policy_in(&state_dir(), layer_id, key, value)
}

pub fn depth_pick(layer_id: &str) -> DepthPick {
    depth_pick_in(&state_dir(), layer_id)
}
