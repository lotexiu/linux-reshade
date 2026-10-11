//! O que dizer à pessoa sobre a profundidade do jogo escolhido: está sendo usada, o jogo precisa ser reaberto, ou ainda
//! não apareceu uma imagem de profundidade.

use control_core::layers::depth_format_name;
use ipc::LiveLayer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepthNote {
    pub text: String,
    /// Algo que a pessoa precisa fazer (reabrir o jogo, abrir uma cena 3D).
    pub attention: bool,
}

/// `uses_depth`: algum efeito ligado no preset lê a profundidade. `layer`: o jogo aberto, se está aberto.
pub fn note(uses_depth: bool, layer: Option<&LiveLayer>) -> Option<DepthNote> {
    let say = |text: String, attention: bool| Some(DepthNote { text, attention });
    let Some(layer) = layer else {
        return uses_depth.then(|| DepthNote { text: "Este preset tem efeitos de profundidade: valem a partir da próxima abertura do jogo.".into(), attention: false });
    };
    let chosen = layer.status.depth.iter().find(|d| d.chosen);
    match (chosen, uses_depth) {
        (Some(d), true) if d.sampled && d.layout != 0 => {
            let orientation = match d.reversed {
                Some(true) => "invertida",
                Some(false) => "normal",
                None => "orientação desconhecida",
            };
            say(format!("Profundidade em uso: {}x{} {}, {orientation}.", d.width, d.height, depth_format_name(d.format)), false)
        }
        (Some(_), true) => say("O jogo precisa ser reaberto para os efeitos usarem a profundidade (abra-o com o daemon rodando, ou use LINUX_RESHADE_DEPTH=1).".into(), true),
        (None, true) => say("Ainda não achei a profundidade do jogo: abra uma cena 3D e espere um instante.".into(), true),
        (Some(d), false) => say(format!("O jogo tem profundidade disponível ({}x{} {}); nenhum efeito do preset a usa.", d.width, d.height, depth_format_name(d.format)), false),
        (None, false) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{DepthStatus, LayerStatus};

    fn layer(depth: Vec<DepthStatus>) -> LiveLayer {
        LiveLayer { id: "l".into(), status: LayerStatus { depth, depth_passes: String::new(), techniques: Vec::new(), exe: "Jogo".into(), app: String::new(), swapchains: Vec::new() } }
    }

    fn candidate(sampled: bool, layout: i32) -> DepthStatus {
        DepthStatus { width: 1920, height: 1080, format: 130, samples: 1, draws: 10, vertices: 0, passes: 3, load: "clear".into(), store: "store".into(), reversed: Some(true), last_pass: 3, chosen: true, unusable: None, sampled, layout }
    }

    #[test]
    fn says_nothing_when_nothing_involves_depth() {
        assert_eq!(note(false, None), None);
        assert_eq!(note(false, Some(&layer(Vec::new()))), None);
    }

    #[test]
    fn closed_game_with_depth_effects_waits_for_the_next_launch() {
        let n = note(true, None).unwrap();
        assert!(!n.attention && n.text.contains("próxima abertura"));
    }

    #[test]
    fn usable_depth_is_reported_with_its_orientation() {
        let n = note(true, Some(&layer(vec![candidate(true, 3)]))).unwrap();
        assert!(!n.attention);
        assert!(n.text.contains("1920x1080") && n.text.contains("D32_SFLOAT_S8") && n.text.contains("invertida"), "{}", n.text);
    }

    #[test]
    fn not_sampleable_means_reopen_and_missing_means_open_a_3d_scene() {
        assert!(note(true, Some(&layer(vec![candidate(false, 3)]))).unwrap().text.contains("reaberto"));
        assert!(note(true, Some(&layer(Vec::new()))).unwrap().text.contains("cena 3D"));
        assert!(note(true, Some(&layer(Vec::new()))).unwrap().attention);
    }
}
