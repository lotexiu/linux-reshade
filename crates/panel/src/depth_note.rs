//! O que dizer à pessoa sobre a profundidade do jogo escolhido: está sendo usada, o jogo precisa ser reaberto, ou ainda
//! não apareceu uma imagem de profundidade.

use control_core::layers::depth_format_name;
use ipc::LiveLayer;
use control_core::i18n::{fmt, tr};

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
        return uses_depth.then(|| DepthNote { text: tr("This preset has depth effects: they apply from the next time the game opens.").into(), attention: false });
    };
    let chosen = layer.status.depth.iter().find(|d| d.chosen);
    match (chosen, uses_depth) {
        (Some(d), true) if d.sampled && d.layout != 0 => {
            let orientation = match d.reversed {
                Some(true) => tr("reversed"),
                Some(false) => tr("normal"),
                None => tr("unknown orientation"),
            };
            say(fmt("Depth in use: {}x{} {}, {}.", &[&d.width, &d.height, &depth_format_name(d.format), &orientation]), false)
        }
        (Some(_), true) => say(tr("The game needs to be reopened for the effects to use depth (open it with the daemon running, or use LINUX_RESHADE_DEPTH=1).").into(), true),
        (None, true) => say(tr("Depth of the game not found yet: open a 3D scene and wait a moment.").into(), true),
        (Some(d), false) => say(fmt("The game has depth available ({}x{} {}); no effect in the preset uses it.", &[&d.width, &d.height, &depth_format_name(d.format)]), false),
        (None, false) => None,
    }
}

/// Uma candidata de profundidade que a pessoa pode escolher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepthChoice {
    /// A posição na lista da layer (de 1), o que `ipc::DepthPick::Candidate` espera.
    pub number: u32,
    pub label: String,
    /// A layer escolheu esta sozinha.
    pub automatic: bool,
    /// Por que não serve (proporção, sem desenhos...), se não serve: ainda dá para escolher, mas avisa.
    pub unusable: Option<String>,
}

/// As candidatas do jogo aberto, na ordem da layer.
pub fn choices(layer: &LiveLayer) -> Vec<DepthChoice> {
    layer
        .status
        .depth
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let orientation = match d.reversed {
                Some(true) => tr(", reversed"),
                Some(false) => tr(", normal"),
                None => "",
            };
            let sample = if d.sampled { "" } else { tr(" (the game needs to be reopened)") };
            DepthChoice {
                number: i as u32 + 1,
                label: format!("{}{orientation}{sample}", fmt("{}x{} {} · {} draws, {} passes", &[&d.width, &d.height, &depth_format_name(d.format), &d.draws, &d.passes])),
                automatic: d.automatic,
                unusable: d.unusable.as_deref().map(control_core::i18n::tr_dyn),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{DepthStatus, LayerStatus};

    fn layer(depth: Vec<DepthStatus>) -> LiveLayer {
        LiveLayer { id: "l".into(), status: LayerStatus { perf: None, depth, depth_passes: String::new(), techniques: Vec::new(), exe: "Jogo".into(), app: String::new(), swapchains: Vec::new() } }
    }

    fn candidate(sampled: bool, layout: i32) -> DepthStatus {
        DepthStatus { width: 1920, height: 1080, format: 130, samples: 1, draws: 10, vertices: 0, passes: 3, load: "clear".into(), store: "store".into(), reversed: Some(true), last_pass: 3, chosen: true, automatic: true, unusable: None, sampled, layout }
    }

    #[test]
    fn choices_follow_the_layers_list_and_mark_the_automatic_one() {
        let mut second = candidate(false, 0);
        second.chosen = false;
        second.automatic = false;
        second.unusable = Some("different aspect ratio than the screen".into());
        let list = choices(&layer(vec![candidate(true, 3), second]));
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].number, list[0].automatic), (1, true));
        assert_eq!((list[1].number, list[1].automatic), (2, false));
        assert!(list[0].label.contains("1920x1080") && list[1].label.contains("reopened"), "{list:?}");
        assert_eq!(list[1].unusable.as_deref(), Some("different aspect ratio than the screen"));
    }

    #[test]
    fn the_automatic_mark_stays_on_what_the_heuristic_picked_when_the_person_chose_another() {
        // A pessoa escolheu a 2ª (em uso: `chosen`); a regra automática continua sendo a 1ª.
        let mut first = candidate(true, 3);
        first.chosen = false;
        let mut second = candidate(true, 3);
        second.automatic = false;
        let list = choices(&layer(vec![first, second]));
        assert_eq!((list[0].automatic, list[1].automatic), (true, false));
    }

    #[test]
    fn says_nothing_when_nothing_involves_depth() {
        assert_eq!(note(false, None), None);
        assert_eq!(note(false, Some(&layer(Vec::new()))), None);
    }

    #[test]
    fn closed_game_with_depth_effects_waits_for_the_next_launch() {
        let n = note(true, None).unwrap();
        assert!(!n.attention && n.text.contains("next time the game opens"));
    }

    #[test]
    fn usable_depth_is_reported_with_its_orientation() {
        let n = note(true, Some(&layer(vec![candidate(true, 3)]))).unwrap();
        assert!(!n.attention);
        assert!(n.text.contains("1920x1080") && n.text.contains("D32_SFLOAT_S8") && n.text.contains(tr("reversed")), "{}", n.text);
    }

    #[test]
    fn not_sampleable_means_reopen_and_missing_means_open_a_3d_scene() {
        assert!(note(true, Some(&layer(vec![candidate(false, 3)]))).unwrap().text.contains("reopened"));
        assert!(note(true, Some(&layer(Vec::new()))).unwrap().text.contains("3D scene"));
        assert!(note(true, Some(&layer(Vec::new()))).unwrap().attention);
    }
}
