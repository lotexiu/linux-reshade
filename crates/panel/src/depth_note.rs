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
    /// Não dá para usar de jeito nenhum (não só "a heurística não a escolheria"): o painel não deixa escolher.
    pub blocking: bool,
}

/// Uma limpeza da profundidade no quadro do jogo: quantos desenhos vieram antes dela (desde a anterior) e se a layer consegue copiar ali.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthClear {
    pub draws_before: u32,
    pub splittable: bool,
}

/// As limpezas do último quadro, lidas do resumo que a layer publica (`desenhos antes de cada uma: 0 276!`; o `!` marca a que não dá para
/// partir). Vazio se o resumo ainda não as traz.
pub fn clears(layer: &LiveLayer) -> Vec<DepthClear> {
    parse_clears(&layer.status.depth_passes)
}

pub fn parse_clears(summary: &str) -> Vec<DepthClear> {
    const KEY: &str = "desenhos antes de cada uma:";
    let Some(start) = summary.find(KEY) else { return Vec::new() };
    let rest = &summary[start + KEY.len()..];
    let list = rest.split(']').next().unwrap_or("");
    list.split_whitespace()
        .filter_map(|token| {
            let splittable = !token.ends_with('!');
            Some(DepthClear { draws_before: token.trim_end_matches('!').parse().ok()?, splittable })
        })
        .collect()
}

/// Um passe de renderização que desenha na profundidade em uso: quantos desenhos faz e o que faz com ela ao começar e ao terminar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepthPass {
    pub draws: u32,
    pub load: String,
    pub store: String,
}

/// Os passes do último quadro, lidos do fim do resumo da layer (`1:311d/0i Clear->Store 2:233d/0i Load->Store`). O resumo é cortado em 700
/// caracteres: um jogo com muitos passes fica com a lista incompleta, e quem usa deve tratar isso (a quantidade certa vem da candidata).
pub fn passes(layer: &LiveLayer) -> Vec<DepthPass> {
    parse_passes(&layer.status.depth_passes)
}

pub fn parse_passes(summary: &str) -> Vec<DepthPass> {
    let Some(tail) = summary.rsplit(" | ").next() else { return Vec::new() };
    let mut out = Vec::new();
    let mut tokens = tail.split_whitespace();
    while let (Some(head), Some(ops)) = (tokens.next(), tokens.next()) {
        let Some((index, rest)) = head.split_once(':') else { break };
        let Ok(index) = index.parse::<usize>() else { break };
        let Some(draws) = rest.split_once('d').and_then(|(d, _)| d.parse::<u32>().ok()) else { break };
        let Some((load, store)) = ops.split_once("->") else { break };
        if index != out.len() + 1 {
            break;
        }
        out.push(DepthPass { draws, load: load.to_string(), store: store.to_string() });
    }
    out
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
                blocking: d.unusable.as_deref().is_some_and(|u| u.starts_with("MSAA without SAMPLED")),
                unusable: d.unusable.as_deref().map(control_core::i18n::tr_dyn),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_passes_come_from_the_end_of_the_summary() {
        let summary = "limpeza escolhida: 0 de 0 [desenhos antes de cada uma: ]; x | tela: 1:pass 99d DontCare 2:barreira->1000001002 | 1:311d/0i Clear->Store 2:233d/0i Load->Store";
        let passes = parse_passes(summary);
        assert_eq!(passes.len(), 2);
        assert_eq!((passes[0].draws, passes[0].load.as_str(), passes[0].store.as_str()), (311, "Clear", "Store"));
        assert_eq!(passes[1].draws, 233);
        assert!(parse_passes("sem passes").is_empty());
    }

    #[test]
    fn the_clears_come_from_the_layers_summary() {
        let summary = "trocas da imagem: 3; limpeza escolhida: 2 de 3 [desenhos antes de cada uma: 0 322 2]; última mudança";
        assert_eq!(parse_clears(summary).iter().map(|c| (c.draws_before, c.splittable)).collect::<Vec<_>>(), vec![(0, true), (322, true), (2, true)]);
        let in_pass = "limpeza escolhida: 1 de 2 [desenhos antes de cada uma: 0 276!]; x";
        assert_eq!(parse_clears(in_pass).iter().map(|c| (c.draws_before, c.splittable)).collect::<Vec<_>>(), vec![(0, true), (276, false)]);
        assert!(parse_clears("").is_empty());
    }

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
    fn only_what_cannot_be_sampled_at_all_blocks_a_choice() {
        let mut ratio = candidate(true, 3);
        ratio.unusable = Some("different aspect ratio than the screen".into());
        let mut msaa = candidate(false, 0);
        msaa.unusable = Some("MSAA without SAMPLED usage (reopen the game with the observation armed)".into());
        let list = choices(&layer(vec![ratio, msaa]));
        assert_eq!((list[0].unusable.is_some(), list[0].blocking), (true, false), "proporção diferente: dá para escolher, com aviso");
        assert_eq!((list[1].unusable.is_some(), list[1].blocking), (true, true), "MSAA sem SAMPLED: não dá");
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
