//! Converte as capturas cruas que as layers deixam no estado compartilhado em PNG na pasta de capturas da pessoa
//! (`<pasta>/<jogo>/<jogo>_<data>_<hora>[_sem-efeitos].png`).

use ipc::shot::{self, Kind, RawShot};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use i18n::fmt;

/// Guarda a hora de cada pedido (as duas capturas de um pedido — com e sem efeitos — levam o mesmo nome base).
#[derive(Default)]
pub struct Saver {
    stamps: HashMap<(String, String), String>,
}

impl Saver {
    /// Converte tudo o que está pendente. Devolve os arquivos PNG gravados.
    pub fn save_pending(&mut self, state_dir: &Path, out_root: &Path) -> Vec<PathBuf> {
        let mut saved = Vec::new();
        for raw in shot::pending_raw_in(state_dir) {
            let number = raw.file_name().and_then(|n| n.to_str()).and_then(|n| n.split("__").nth(1)).unwrap_or("0").to_string();
            let parsed = shot::read_raw(&raw);
            let _ = std::fs::remove_file(&raw);
            let Some(shot) = parsed else { continue };
            let stamp = self.stamps.entry((shot.game.clone(), number)).or_insert_with(ipc::file_stamp).clone();
            match write_png(out_root, &shot, &stamp) {
                Ok(path) => saved.push(path),
                Err(why) => crate::elogln!("{}", fmt("screenshot of {}: {}", &[&shot.game, &why])),
            }
        }
        // Não acumula horas de pedidos antigos.
        if self.stamps.len() > 64 {
            self.stamps.clear();
        }
        saved
    }
}

fn write_png(out_root: &Path, shot: &RawShot, stamp: &str) -> Result<PathBuf, String> {
    let rgba = shot.order.to_rgba8(&shot.pixels);
    let bytes = image_file::encode_png(shot.width, shot.height, &rgba).map_err(|e| e.to_string())?;
    let game: String = shot.game.chars().map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
    let dir = out_root.join(&game);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let suffix = if shot.kind == Kind::WithoutEffects { "_sem-efeitos" } else { "" };
    let path = dir.join(format!("{game}_{stamp}{suffix}.png"));
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::shot::Order;

    #[test]
    fn raw_shots_become_pngs_named_by_game_and_pair_up() {
        let base = std::env::temp_dir().join(format!("lr-shots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (state, out) = (base.join("state"), base.join("out"));
        let mk = |kind| RawShot { game: "Meu Jogo".into(), kind, width: 2, height: 1, order: Order::Bgra, pixels: vec![10, 20, 30, 255, 40, 50, 60, 255] };
        shot::write_raw_in(&state, &mk(Kind::WithoutEffects), 7).unwrap();
        shot::write_raw_in(&state, &mk(Kind::WithEffects), 7).unwrap();
        let saved = Saver::default().save_pending(&state, &out);
        assert_eq!(saved.len(), 2);
        let names: Vec<String> = saved.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|n| n.ends_with("_sem-efeitos.png")) && names.iter().any(|n| !n.ends_with("_sem-efeitos.png")), "{names:?}");
        assert!(saved[0].starts_with(out.join("Meu_Jogo")));
        let png = image_file::decode("x.png", &std::fs::read(saved.iter().find(|p| !p.to_string_lossy().contains("sem-efeitos")).unwrap()).unwrap()).unwrap();
        assert_eq!(png.pixels[..4], [30, 20, 10, 255], "BGRA virou RGBA");
        assert!(shot::pending_raw_in(&state).is_empty(), "as cruas foram consumidas");
    }
}
