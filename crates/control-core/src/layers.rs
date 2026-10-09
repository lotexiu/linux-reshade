//! A lista de jogos com a layer ativa, como `lrctl layers` a mostra.

use ipc::{EffectState, LiveLayer};

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

/// A tabela de jogos e swapchains. Vazia, explica como ligar a layer.
pub fn render(layers: &[LiveLayer]) -> String {
    let rows: Vec<(String, String, String, String)> = layers
        .iter()
        .flat_map(|l| {
            let app = if l.status.app.is_empty() { l.id.clone() } else { l.status.app.clone() };
            l.status.swapchains.iter().map(move |s| {
                (app.clone(), format!("{}x{}", s.width, s.height), format_name(s.format), effect_text(&s.effect))
            })
        })
        .collect();
    if rows.is_empty() {
        let mut text = String::from("nenhum jogo com a layer ativa\n");
        text.push_str("(abra um jogo com LINUX_RESHADE=1 nas opções de lançamento; a layer só informa depois de criar o swapchain)\n");
        return text;
    }
    let width = |f: fn(&(String, String, String, String)) -> usize| rows.iter().map(f).max().unwrap_or(0);
    let (w1, w2, w3) = (width(|r| r.0.chars().count()).max(4), width(|r| r.1.len()).max(9), width(|r| r.2.chars().count()).max(7));
    let mut text = format!("{:<w1$}  {:<w2$}  {:<w3$}  EFEITO\n", "JOGO", "RESOLUÇÃO", "FORMATO");
    for (app, size, format, effect) in &rows {
        text.push_str(&format!("{app:<w1$}  {size:<w2$}  {format:<w3$}  {effect}\n"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{LayerStatus, SwapchainStatus};

    fn live(app: &str, swapchains: Vec<SwapchainStatus>) -> LiveLayer {
        LiveLayer { id: "layer-1-2".into(), status: LayerStatus { app: app.into(), swapchains } }
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
}
