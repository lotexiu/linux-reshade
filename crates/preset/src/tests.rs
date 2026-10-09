use super::*;

fn id(s: &str) -> TechId {
    TechId::parse(s)
}

fn ids(v: &[&str]) -> Vec<TechId> {
    v.iter().map(|s| id(s)).collect()
}

const PRESET: &str = "PreprocessorDefinitions=DEPTH_REVERSED=1,FLAG\r\n\
Techniques=Sepia@Sepia.fx,Bloom@Bloom.fx\r\n\
TechniqueSorting=Sepia@Sepia.fx,Vinheta@Vinheta.fx,Bloom@Bloom.fx\r\n\
KeySepia@Sepia.fx=122,1,0,0\r\n\
\r\n\
; ajustei à mão\r\n\
[Sepia.fx]\r\n\
Intensity=0.8\r\n\
Cor=1.000000,0.500000,0.200000\r\n\
KeyToggle=70,0,0,0\r\n\
PreprocessorDefinitions=QUALITY=2,FAST=\r\n\
\r\n\
[Bloom.fx]\r\n\
Force=0.3\r\n";

#[test]
fn unchanged_preset_round_trips_byte_for_byte() {
    assert_eq!(Preset::parse(PRESET).to_string(), PRESET);
}

#[test]
fn reads_techniques_and_order() {
    let p = Preset::parse(PRESET);
    assert_eq!(p.techniques(), ids(&["Sepia@Sepia.fx", "Bloom@Bloom.fx"]));
    assert_eq!(p.sorting(), ids(&["Sepia@Sepia.fx", "Vinheta@Vinheta.fx", "Bloom@Bloom.fx"]));
    assert!(p.is_enabled(&id("Sepia@Sepia.fx")));
    assert!(!p.is_enabled(&id("Vinheta@Vinheta.fx")));
    assert_eq!(id("Sepia@Sepia.fx").to_string(), "Sepia@Sepia.fx");
    assert_eq!(id("Sepia").to_string(), "Sepia");
}

#[test]
fn a_bare_name_matches_any_file() {
    assert!(id("Sepia").matches(&id("Sepia@Sepia.fx")));
    assert!(!id("Sepia@A.fx").matches(&id("Sepia@B.fx")));
    let p = Preset::parse("Techniques=Sepia\n");
    assert!(p.is_enabled(&id("Sepia@Sepia.fx")));
}

#[test]
fn without_sorting_the_techniques_list_gives_the_order() {
    let p = Preset::parse("Techniques=B@b.fx,A@a.fx\n");
    assert_eq!(p.sorting(), ids(&["B@b.fx", "A@a.fx"]));
}

#[test]
fn resolve_orders_by_the_list_and_appends_the_rest_in_fallback_order() {
    let p = Preset::parse(PRESET);
    // Ordem de reserva: alfabética por arquivo.
    let avail = ids(&["Bloom@Bloom.fx", "Sepia@Sepia.fx", "Vinheta@Vinheta.fx", "Zoom@Zoom.fx"]);
    let r = p.resolve(&avail);
    let order: Vec<_> = r.order.iter().map(|o| (o.id.to_string(), o.enabled)).collect();
    assert_eq!(
        order,
        [
            ("Sepia@Sepia.fx".into(), true),
            ("Vinheta@Vinheta.fx".into(), false),
            ("Bloom@Bloom.fx".into(), true),
            ("Zoom@Zoom.fx".into(), false),
        ]
    );
    assert!(r.unknown.is_empty());
    assert_eq!(r.enabled().map(TechId::to_string).collect::<Vec<_>>(), ["Sepia@Sepia.fx", "Bloom@Bloom.fx"]);
}

#[test]
fn an_unknown_enabled_technique_is_reported_not_fatal() {
    let p = Preset::parse("Techniques=Sepia@Sepia.fx,Fantasma@Fantasma.fx\nTechniqueSorting=Fantasma@Fantasma.fx,Sepia@Sepia.fx\n");
    let r = p.resolve(&ids(&["Sepia@Sepia.fx"]));
    assert_eq!(r.unknown, ids(&["Fantasma@Fantasma.fx"]));
    assert_eq!(r.enabled().count(), 1);
}

#[test]
fn an_unknown_disabled_technique_is_ignored() {
    let p = Preset::parse("Techniques=Sepia@Sepia.fx\nTechniqueSorting=Velha@Velha.fx,Sepia@Sepia.fx\n");
    let r = p.resolve(&ids(&["Sepia@Sepia.fx"]));
    assert!(r.unknown.is_empty());
}

#[test]
fn an_enabled_technique_missing_from_sorting_is_still_resolved() {
    let p = Preset::parse("Techniques=A@a.fx,B@b.fx\nTechniqueSorting=A@a.fx\n");
    let r = p.resolve(&ids(&["A@a.fx", "B@b.fx"]));
    assert_eq!(r.enabled().count(), 2);
}

#[test]
fn two_techniques_from_the_same_file_are_kept_apart() {
    let p = Preset::parse("Techniques=S2@Sharp.fx,S1@Sharp.fx\n");
    let r = p.resolve(&ids(&["S1@Sharp.fx", "S2@Sharp.fx"]));
    assert_eq!(r.enabled().map(TechId::to_string).collect::<Vec<_>>(), ["S2@Sharp.fx", "S1@Sharp.fx"]);
}

#[test]
fn disabling_keeps_the_order_and_everything_else() {
    let mut p = Preset::parse(PRESET);
    p.set_enabled(&id("Bloom@Bloom.fx"), false);
    assert_eq!(p.techniques(), ids(&["Sepia@Sepia.fx"]));
    assert_eq!(p.sorting(), ids(&["Sepia@Sepia.fx", "Vinheta@Vinheta.fx", "Bloom@Bloom.fx"]));
    let want = PRESET.replace("Techniques=Sepia@Sepia.fx,Bloom@Bloom.fx", "Techniques=Sepia@Sepia.fx");
    assert_eq!(p.to_string(), want);
}

#[test]
fn enabling_puts_the_technique_at_its_place_in_the_order() {
    let mut p = Preset::parse(PRESET);
    p.set_enabled(&id("Vinheta@Vinheta.fx"), true);
    assert_eq!(p.techniques(), ids(&["Sepia@Sepia.fx", "Vinheta@Vinheta.fx", "Bloom@Bloom.fx"]));
}

#[test]
fn setting_the_state_it_already_has_changes_nothing() {
    let mut p = Preset::parse(PRESET);
    p.set_enabled(&id("Sepia@Sepia.fx"), true);
    p.set_enabled(&id("Vinheta@Vinheta.fx"), false);
    assert_eq!(p.to_string(), PRESET);
}

#[test]
fn a_new_technique_goes_to_the_end() {
    let mut p = Preset::parse("Techniques=A@a.fx\n");
    p.set_enabled(&id("B@b.fx"), true);
    assert_eq!(p.techniques(), ids(&["A@a.fx", "B@b.fx"]));
    // A ordem coincide com a das ligadas e não havia lista de ordenação: continua sem ela.
    assert_eq!(p.to_string(), "Techniques=A@a.fx,B@b.fx\n");
}

#[test]
fn moving_a_technique_writes_sorting_only_when_needed() {
    let mut p = Preset::parse("Techniques=A@a.fx,B@b.fx,C@c.fx\n");
    assert!(p.move_technique(&id("C@c.fx"), 0));
    assert_eq!(p.techniques(), ids(&["C@c.fx", "A@a.fx", "B@b.fx"]));
    assert!(!p.to_string().contains("TechniqueSorting"));

    // Com uma desligada no meio, a ordem de todas difere da das ligadas: grava a lista.
    let mut p = Preset::parse("Techniques=A@a.fx,C@c.fx\nTechniqueSorting=A@a.fx,B@b.fx,C@c.fx\n");
    assert!(p.move_technique(&id("C@c.fx"), 1));
    assert_eq!(p.sorting(), ids(&["A@a.fx", "C@c.fx", "B@b.fx"]));
    assert_eq!(p.techniques(), ids(&["A@a.fx", "C@c.fx"]));
}

#[test]
fn moving_clamps_and_rejects_unknown() {
    let mut p = Preset::parse("Techniques=A@a.fx,B@b.fx\n");
    assert!(p.move_technique(&id("A@a.fx"), 99));
    assert_eq!(p.techniques(), ids(&["B@b.fx", "A@a.fx"]));
    assert!(!p.move_technique(&id("X@x.fx"), 0));
}

#[test]
fn an_empty_technique_list_is_written_empty() {
    let mut p = Preset::parse("Techniques=A@a.fx\n");
    p.set_enabled(&id("A@a.fx"), false);
    assert_eq!(p.to_string(), "Techniques=\nTechniqueSorting=A@a.fx\n");
}

#[test]
fn technique_keys() {
    let mut p = Preset::parse(PRESET);
    let k = p.technique_key(&id("Sepia@Sepia.fx")).unwrap();
    assert_eq!(k, KeyCombo { code: 122, ctrl: true, shift: false, alt: false });
    assert_eq!(p.technique_key(&id("Bloom@Bloom.fx")), None);
    // O mesmo atalho achado por nome sem arquivo.
    assert!(p.technique_key(&id("Sepia")).is_some());

    p.set_technique_key(&id("Bloom@Bloom.fx"), Some(KeyCombo { code: 123, ctrl: false, shift: true, alt: true }));
    assert!(p.to_string().contains("KeyBloom@Bloom.fx=123,0,1,1"));
    p.set_technique_key(&id("Bloom@Bloom.fx"), None);
    assert_eq!(p.technique_key(&id("Bloom@Bloom.fx")), None);
    assert_eq!(p.to_string(), PRESET);
}

#[test]
fn key_zero_means_no_shortcut() {
    assert_eq!(KeyCombo::parse("0,0,0,0"), None);
    assert_eq!(KeyCombo::parse("lixo"), None);
    assert_eq!(KeyCombo::parse("70"), Some(KeyCombo { code: 70, ctrl: false, shift: false, alt: false }));
}

#[test]
fn macros_empty_value_means_one() {
    assert_eq!(parse_macros("A=2, B ,C=,A=3"), [("A".into(), "3".into()), ("B".into(), "1".into()), ("C".into(), "1".into())]);
    assert!(parse_macros("").is_empty());
}

#[test]
fn macros_effect_beats_preset_beats_global() {
    let p = Preset::parse(PRESET);
    assert_eq!(p.macros(), [("DEPTH_REVERSED".into(), "1".into()), ("FLAG".into(), "1".into())]);
    let global: Macros = vec![("QUALITY".into(), "0".into()), ("FLAG".into(), "0".into()), ("G".into(), "9".into())];
    let m = p.effective_macros(&global, "Sepia.fx");
    let get = |n: &str| m.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    assert_eq!(get("QUALITY"), Some("2")); // efeito
    assert_eq!(get("FLAG"), Some("1")); // preset vence global
    assert_eq!(get("FAST"), Some("1")); // vazio vale 1
    assert_eq!(get("G"), Some("9")); // só global
    // Outro efeito não herda as macros do Sepia.
    assert_eq!(get_in(&p.effective_macros(&global, "Bloom.fx"), "QUALITY"), Some("0".to_string()));
}

fn get_in(m: &Macros, n: &str) -> Option<String> {
    m.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone())
}

#[test]
fn setting_macros_and_clearing_them() {
    let mut p = Preset::parse("Techniques=A@a.fx\n");
    p.set_macros(&[("X".into(), "1".into()), ("Y".into(), "2".into())]);
    assert_eq!(p.to_string(), "Techniques=A@a.fx\nPreprocessorDefinitions=X=1,Y=2\n");
    p.set_effect_macros("a.fx", &[("Q".into(), "3".into())]);
    assert_eq!(p.effect_macros("a.fx"), [("Q".into(), "3".into())]);
    p.set_macros(&[]);
    assert!(p.macros().is_empty());
    assert!(!p.to_string().contains("X=1"));
}

#[test]
fn uniform_values() {
    let mut p = Preset::parse(PRESET);
    assert_eq!(p.value("Sepia.fx", "Intensity"), Some("0.8"));
    assert_eq!(p.value("Sepia.fx", "Cor"), Some("1.000000,0.500000,0.200000"));
    // `values` não mistura macros nem atalhos com os uniforms.
    assert_eq!(p.values("Sepia.fx"), [("Intensity", "0.8"), ("Cor", "1.000000,0.500000,0.200000")]);
    assert_eq!(p.effect_files(), ["Sepia.fx", "Bloom.fx"]);

    p.set_value("Sepia.fx", "Intensity", "0.5");
    p.set_value("Novo.fx", "X", "1");
    assert_eq!(p.value("Sepia.fx", "Intensity"), Some("0.5"));
    assert_eq!(p.value("Novo.fx", "X"), Some("1"));
    assert!(p.remove_value("Novo.fx", "X"));
    assert_eq!(p.value("Novo.fx", "X"), None);
}

#[test]
fn only_the_changed_value_is_rewritten() {
    let mut p = Preset::parse(PRESET);
    p.set_value("Sepia.fx", "Intensity", "0.5");
    assert_eq!(p.to_string(), PRESET.replace("Intensity=0.8", "Intensity=0.5"));
}

#[test]
fn save_and_load_round_trip_on_disk() {
    let dir = std::env::temp_dir().join(format!("preset-test-{}", std::process::id()));
    let path = dir.join("sub").join("p.ini");
    let mut p = Preset::parse(PRESET);
    p.set_enabled(&id("Bloom@Bloom.fx"), false);
    p.save(&path).unwrap();
    assert_eq!(Preset::load(&path).unwrap(), p);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Ida e volta com presets reais da comunidade. Os arquivos não vão no repositório (licenças variadas):
/// aponte `LR_PRESET_CORPUS` para uma pasta com `.ini` de presets e rode os testes.
#[test]
fn community_presets_round_trip() {
    let Some(dir) = std::env::var_os("LR_PRESET_CORPUS") else {
        eprintln!("LR_PRESET_CORPUS não definida: sem presets reais para conferir");
        return;
    };
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "ini") {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        if bytes.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut p = Preset::parse(&text);
        // Só a falta de `\n` no fim pode mudar.
        let want = if text.ends_with('\n') { text.to_string() } else { format!("{text}\n") };
        let want = if text.contains("\r\n") && !text.ends_with('\n') { format!("{text}\r\n") } else { want };
        assert_eq!(p.to_string(), want, "{}", path.display());

        // Ligar o que já está ligado e desligar o que já está desligado não muda nada.
        for t in p.techniques() {
            p.set_enabled(&t, true);
        }
        assert_eq!(p.to_string(), want, "{}", path.display());

        // Desligar e religar todas devolve as mesmas técnicas ligadas, na mesma ordem relativa.
        let before = p.resolve(&p.sorting()).order;
        for t in p.techniques() {
            p.set_enabled(&t, false);
        }
        assert_eq!(p.techniques(), []);
        for t in before.iter().filter(|o| o.enabled) {
            p.set_enabled(&t.id, true);
        }
        assert_eq!(p.resolve(&p.sorting()).order, before, "{}", path.display());
        seen += 1;
    }
    eprintln!("{seen} presets reais conferidos");
}
