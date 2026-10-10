//! A lista de jogos com a layer ativa, como `lrctl layers` a mostra.

use ipc::{EffectState, LiveLayer, TechniqueState};

/// O nome do `VkFormat` mais comum em swapchains; o número, se for outro.
pub fn format_name(format: i32) -> String {
    match format {
        37 => "R8G8B8A8_UNORM".into(),
        43 => "R8G8B8A8_SRGB".into(),
        44 => "B8G8R8A8_UNORM".into(),
        50 => "B8G8R8A8_SRGB".into(),
        58 => "A2R10G10B10_UNORM".into(),
        64 => "A2B10G10R10_UNORM".into(),
        97 => "R16G16B16A16_SFLOAT".into(),
        other => format!("formato {other}"),
    }
}

pub fn effect_text(state: &EffectState) -> String {
    match state {
        EffectState::Idle => "nenhum efeito escolhido".into(),
        EffectState::Waiting => "aguardando o bundle desta resolução".into(),
        EffectState::Ready => "pronto".into(),
        EffectState::Unavailable(why) => format!("indisponível: {why}"),
        EffectState::Failed(why) => format!("falhou: {why}"),
    }
}

/// O número do processo dentro do id de uma layer (`layer-4321-998` → `4321`): o que o usuário digita em `--layer`.
pub fn pid_of(id: &str) -> Option<&str> {
    id.strip_prefix("layer-")?.split('-').next().filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Acha a layer que o usuário quis dizer: o id inteiro, o número do processo, ou o nome do jogo (sem
/// diferenciar maiúsculas; precisa ser único).
pub fn find<'a>(layers: &'a [LiveLayer], query: &str) -> Result<&'a LiveLayer, String> {
    let query = query.trim();
    let describe = |l: &LiveLayer| format!("{} (pid {})", if l.status.app.is_empty() { &l.id } else { &l.status.app }, pid_of(&l.id).unwrap_or("?"));
    if let Some(l) = layers.iter().find(|l| l.id == query) {
        return Ok(l);
    }
    let by_pid: Vec<_> = layers.iter().filter(|l| pid_of(&l.id) == Some(query)).collect();
    let matches = if by_pid.is_empty() { layers.iter().filter(|l| l.status.app.eq_ignore_ascii_case(query) || l.status.exe.eq_ignore_ascii_case(query)).collect() } else { by_pid };
    match matches.as_slice() {
        [one] => Ok(one),
        [] if layers.is_empty() => Err(format!("nenhum jogo com a layer ativa (procurei {query:?})")),
        [] => Err(format!("nenhum jogo com a layer chamado {query:?}; abertos: {}", layers.iter().map(describe).collect::<Vec<_>>().join(", "))),
        many => Err(format!("{query:?} serve a mais de um jogo ({}); use o número do processo", many.iter().map(|l| describe(l)).collect::<Vec<_>>().join(", "))),
    }
}

type Row = (String, String, String, String, String);

/// A tabela de jogos e swapchains. Vazia, explica como ligar a layer.
pub fn render(layers: &[LiveLayer]) -> String {
    let rows: Vec<Row> = layers
        .iter()
        .flat_map(|l| {
            let app = if l.status.app.is_empty() { l.id.clone() } else { l.status.app.clone() };
            let pid = pid_of(&l.id).unwrap_or("?").to_string();
            l.status.swapchains.iter().map(move |s| (app.clone(), pid.clone(), format!("{}x{}", s.width, s.height), format_name(s.format), effect_text(&s.effect)))
        })
        .collect();
    if rows.is_empty() {
        let mut text = String::from("nenhum jogo com a layer ativa\n");
        text.push_str("(abra um jogo com LINUX_RESHADE=1 nas opções de lançamento; a layer só informa depois de criar o swapchain)\n");
        return text;
    }
    let width = |f: fn(&Row) -> usize| rows.iter().map(f).max().unwrap_or(0);
    let (w1, w2, w3, w4) = (width(|r| r.0.chars().count()).max(4), width(|r| r.1.len()).max(3), width(|r| r.2.len()).max(9), width(|r| r.3.chars().count()).max(7));
    let mut text = format!("{:<w1$}  {:<w2$}  {:<w3$}  {:<w4$}  EFEITO\n", "JOGO", "PID", "RESOLUÇÃO", "FORMATO");
    for (app, pid, size, format, effect) in &rows {
        text.push_str(&format!("{app:<w1$}  {pid:<w2$}  {size:<w3$}  {format:<w4$}  {effect}\n"));
    }
    // O que a layer fez com cada técnica da cadeia (só dos jogos que construíram uma).
    for l in layers.iter().filter(|l| !l.status.techniques.is_empty()) {
        let app = if l.status.app.is_empty() { l.id.clone() } else { l.status.app.clone() };
        text.push_str(&format!("\ncadeia de {app} (pid {}):\n", pid_of(&l.id).unwrap_or("?")));
        for (n, t) in l.status.techniques.iter().enumerate() {
            match &t.state {
                TechniqueState::Applied => text.push_str(&format!("  {}. {}  aplicada\n", n + 1, t.name)),
                TechniqueState::Skipped(why) => text.push_str(&format!("  {}. {}  pulada: {why}\n", n + 1, t.name)),
            }
        }
    }
    // As candidatas a profundidade da cena (só com a observação ligada na layer).
    for l in layers.iter().filter(|l| !l.status.depth.is_empty()) {
        let app = if l.status.app.is_empty() { l.id.clone() } else { l.status.app.clone() };
        text.push_str(&format!("\nprofundidade de {app} (pid {}):\n", pid_of(&l.id).unwrap_or("?")));
        if !l.status.depth_passes.is_empty() {
            text.push_str(&format!("  passes da escolhida (posição:desenhos/indiretos tamanho começa->termina): {}\n", l.status.depth_passes));
        }
        for (n, d) in l.status.depth.iter().enumerate() {
            let reversed = match d.reversed {
                Some(true) => "invertida",
                Some(false) => "normal",
                None => "?",
            };
            let why = d.unusable.as_deref().map(|u| format!("  [não serve: {u}]")).unwrap_or_default();
            let sampling = if d.sampled { "amostrável" } else { "sem SAMPLED" };
            text.push_str(&format!(
                "  {} [{}] {}x{} {}  {} desenhos, {} vértices, {} passe(s)  {} -> {}  {reversed}  {sampling}, layout {}{why}\n",
                if d.chosen { "*" } else { " " },
                n + 1,
                d.width,
                d.height,
                depth_format_name(d.format),
                d.draws,
                d.vertices,
                d.passes,
                d.load,
                d.store,
                layout_name(d.layout),
            ));
        }
    }
    text
}

fn layout_name(layout: i32) -> String {
    match layout {
        0 => "?".into(),
        1 => "GENERAL".into(),
        3 => "DEPTH_STENCIL_ATTACHMENT".into(),
        4 => "DEPTH_STENCIL_READ_ONLY".into(),
        5 => "SHADER_READ_ONLY".into(),
        6 => "TRANSFER_SRC".into(),
        7 => "TRANSFER_DST".into(),
        1_000_241_000 => "DEPTH_ATTACHMENT".into(),
        1_000_241_001 => "DEPTH_READ_ONLY".into(),
        other => format!("{other}"),
    }
}

fn depth_format_name(format: i32) -> String {
    match format {
        124 => "D16_UNORM".into(),
        125 => "X8_D24_UNORM".into(),
        126 => "D32_SFLOAT".into(),
        128 => "D16_UNORM_S8".into(),
        129 => "D24_UNORM_S8".into(),
        130 => "D32_SFLOAT_S8".into(),
        other => format!("formato {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{LayerStatus, SwapchainStatus};

    #[test]
    fn the_chain_of_a_game_is_listed_below_the_table() {
        let mut game = live("Haste", vec![sc(1920, 1080, 44, EffectState::Ready)]);
        game.status.techniques = vec![
            ipc::TechniqueStatus { name: "Sepia".into(), state: TechniqueState::Applied },
            ipc::TechniqueStatus { name: "CAS".into(), state: TechniqueState::Skipped("escrita sRGB numa textura".into()) },
        ];
        let text = render(&[game]);
        assert!(text.contains("cadeia de Haste (pid 1):"), "{text}");
        assert!(text.contains("1. Sepia  aplicada") && text.contains("2. CAS  pulada: escrita sRGB numa textura"), "{text}");
        assert!(!render(&[live("Outro", vec![sc(10, 10, 44, EffectState::Idle)])]).contains("cadeia de"));
    }

    fn live(app: &str, swapchains: Vec<SwapchainStatus>) -> LiveLayer {
        live_with(app, "layer-1-2", swapchains)
    }

    fn live_with(app: &str, id: &str, swapchains: Vec<SwapchainStatus>) -> LiveLayer {
        LiveLayer { id: id.into(), status: LayerStatus { depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: app.into(), swapchains } }
    }

    fn sc(w: u32, h: u32, format: i32, effect: EffectState) -> SwapchainStatus {
        SwapchainStatus { width: w, height: h, format, images: 3, effect }
    }

    #[test]
    fn known_formats_have_names_and_unknown_ones_show_the_number() {
        assert_eq!(format_name(44), "B8G8R8A8_UNORM");
        assert_eq!(format_name(50), "B8G8R8A8_SRGB");
        assert_eq!(format_name(999), "formato 999");
    }

    #[test]
    fn the_table_lines_up_and_names_each_state() {
        let text = render(&[
            live("Idols of Ash", vec![sc(1920, 1080, 44, EffectState::Ready), sc(1920, 1011, 44, EffectState::Waiting)]),
            live("RocketLeague.exe", vec![sc(1914, 1048, 50, EffectState::Unavailable("swapchain em formato sRGB".into()))]),
        ]);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].starts_with("JOGO") && lines[0].ends_with("EFEITO"));
        assert!(lines[1].contains("Idols of Ash") && lines[1].contains("1920x1080") && lines[1].ends_with("pronto"));
        assert!(lines[2].contains("1920x1011") && lines[2].ends_with("aguardando o bundle desta resolução"));
        assert!(lines[3].contains("B8G8R8A8_SRGB") && lines[3].ends_with("indisponível: swapchain em formato sRGB"));
        // As colunas começam no mesmo ponto (em caracteres, não bytes: o cabeçalho tem acentos) em todas as linhas.
        let col = |l: &str| {
            ["EFEITO", "pronto", "aguardando", "indisponível"].iter().find_map(|w| l.find(w)).map(|byte| l[..byte].chars().count())
        };
        assert!(lines[1].contains("  1  "), "mostra o pid: {text}");
        assert_eq!(col(lines[0]), col(lines[1]));
        assert_eq!(col(lines[0]), col(lines[3]));
    }

    #[test]
    fn an_empty_list_explains_how_to_turn_the_layer_on() {
        let text = render(&[]);
        assert!(text.contains("nenhum jogo") && text.contains("LINUX_RESHADE=1"), "{text}");
    }

    #[test]
    fn a_layer_without_a_name_falls_back_to_its_id() {
        assert!(render(&[live("", vec![sc(10, 10, 44, EffectState::Idle)])]).contains("layer-1-2"));
    }

    #[test]
    fn the_pid_is_read_from_the_id() {
        assert_eq!(pid_of("layer-4321-998"), Some("4321"));
        assert_eq!(pid_of("layer-x-1"), None);
        assert_eq!(pid_of("ctl-1-2"), None);
    }

    #[test]
    fn find_accepts_id_pid_or_name_and_explains_failures() {
        let layers = [
            live_with("Idols of Ash", "layer-100-1", vec![]),
            live_with("RocketLeague", "layer-200-2", vec![]),
            live_with("vkcube", "layer-300-3", vec![]),
            live_with("vkcube", "layer-400-4", vec![]),
        ];
        assert_eq!(find(&layers, "layer-100-1").unwrap().id, "layer-100-1");
        assert_eq!(find(&layers, "200").unwrap().id, "layer-200-2");
        assert_eq!(find(&layers, "rocketleague").unwrap().id, "layer-200-2", "nome sem diferenciar maiúsculas");
        assert_eq!(find(&layers, " 300 ").unwrap().id, "layer-300-3");
        let ambiguous = find(&layers, "vkcube").unwrap_err();
        assert!(ambiguous.contains("pid 300") && ambiguous.contains("pid 400"), "{ambiguous}");
        let missing = find(&layers, "nada").unwrap_err();
        assert!(missing.contains("Idols of Ash") && missing.contains("RocketLeague"), "{missing}");
        assert!(find(&[], "1").unwrap_err().contains("nenhum jogo"));
    }
}
