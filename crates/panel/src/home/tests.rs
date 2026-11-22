use super::*;
use ipc::{EffectState, LayerStatus, SwapchainStatus};
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fx-bundle/tests/data/Sepia")
}

struct Env {
    root: PathBuf,
    compiler: Compiler,
}

/// Um ambiente com um efeito `Sepia.fx` e um "fx2spv" que apenas copia o bundle de referência.
fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("lr-home-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("effects")).unwrap();
    fs::create_dir_all(root.join("Presets")).unwrap();
    fs::write(root.join("effects/Sepia.fx"), "// de mentira").unwrap();
    let tool = root.join("fake-fx2spv");
    let script = format!(
        "#!/bin/sh\nout=\"\"; prev=\"\"\nfor a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; done\nrm -rf \"$out\"; cp -r '{}' \"$out\"\n",
        golden().display()
    );
    fs::write(&tool, script).unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    let compiler = Compiler { fx2spv: tool, effects: vec![config::SearchPath::flat(root.join("effects"))], includes: vec![], textures: vec![], cache: root.join("cache") };
    Env { root, compiler }
}

fn open_game(id: &str) -> Vec<LiveLayer> {
    vec![LiveLayer {
        id: "layer-1-1".into(),
        status: LayerStatus { perf: None,
            depth: Vec::new(), depth_passes: String::new(),
            techniques: Vec::new(),
            exe: id.into(),
            app: "App".into(),
            swapchains: vec![SwapchainStatus { width: 800, height: 600, format: 44, images: 3, effect: EffectState::Idle }],
        },
    }]
}

fn home(e: Env, layers: Vec<LiveLayer>) -> (Home, PathBuf) {
    let state = ipc::Writer::create_at(&e.root.join("state-dir/state")).unwrap();
    let presets = e.root.join("Presets");
    (Home::with_layers(Ok(e.compiler), Some(presets.clone()), state, Box::new(move || layers.clone())), presets)
}

fn sepia() -> TechId {
    TechId::new("Sepia", "Sepia.fx")
}

#[test]
fn it_starts_on_the_open_game_with_nothing_to_show() {
    let (h, _) = home(env("start"), open_game("vkcube"));
    let s = h.snapshot();
    assert_eq!(s.game.as_deref(), Some("vkcube"));
    assert_eq!(s.games, [GameEntry { id: "vkcube".into(), open: true }]);
    assert!(s.preset.is_none() && s.techniques.is_empty());
    assert_eq!(s.effects, ["Sepia"]);
}

#[test]
fn adding_an_effect_to_a_game_without_preset_creates_its_default_preset() {
    let (mut h, presets) = home(env("add"), open_game("vkcube"));
    h.add_effect("Sepia");
    assert!(h.snapshot().notice.is_none(), "{:?}", h.snapshot().notice);
    assert_eq!(fs::read_to_string(presets.join("vkcube/game.ini")).unwrap(), "preset=Padrão.ini\n");
    assert!(fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap().contains("Techniques=Sepia@Sepia.fx"));
    let s = h.snapshot();
    assert_eq!(s.active_preset.as_deref(), Some("Padrão"));
    assert_eq!(s.presets, ["Padrão"]);
    assert_eq!(s.techniques.len(), 1);
    let t = &s.techniques[0];
    assert_eq!((t.label.as_str(), t.file.as_str(), t.enabled), ("Sépia", "Sepia.fx", true));
    assert_eq!(t.controls.len(), 1);
    assert_eq!(t.controls[0].value, vec![1.0]);
}

#[test]
fn editing_a_game_that_uses_the_global_preset_copies_it_and_leaves_the_global_alone() {
    let e = env("inherit");
    fs::write(e.root.join("Presets/preset.ini"), "; global\nTechniques=Sepia@Sepia.fx\n").unwrap();
    let (mut h, presets) = home(e, open_game("vkcube"));
    assert!(h.snapshot().preset.as_ref().unwrap().inherited, "mostra o global que o jogo herda");
    assert!(h.snapshot().techniques[0].enabled);

    h.set_technique_enabled(&sepia(), false);
    assert!(!h.snapshot().techniques[0].enabled);
    assert!(!h.snapshot().preset.as_ref().unwrap().inherited, "agora o jogo tem o preset dele");
    assert_eq!(fs::read_to_string(presets.join("preset.ini")).unwrap(), "; global\nTechniques=Sepia@Sepia.fx\n", "o global não mudou");
}

#[test]
fn controls_are_written_to_the_preset_and_reset_removes_them() {
    let (mut h, presets) = home(env("controls"), open_game("vkcube"));
    h.add_effect("Sepia");
    h.set_control("Sepia.fx", "Strength", &[0.25]);
    assert!(h.snapshot().notice.is_none(), "{:?}", h.snapshot().notice);
    assert_eq!(h.snapshot().techniques[0].controls[0].value, vec![0.25]);
    assert!(fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap().contains("[Sepia.fx]\nStrength=0.25"));

    h.set_control("Sepia.fx", "Strength", &[9.0]);
    assert!(h.snapshot().notice.as_deref().is_some_and(|n| n.contains("outside the range")));

    h.reset_effect("Sepia.fx");
    assert_eq!(h.snapshot().techniques[0].controls[0].value, vec![1.0]);
    assert!(h.snapshot().techniques[0].controls[0].is_default());
}

#[test]
fn techniques_can_be_reordered_and_removed() {
    let e = env("order");
    fs::write(e.root.join("Presets/preset.ini"), "Techniques=Sepia@Sepia.fx,Outra@Outra.fx\n").unwrap();
    let (mut h, _) = home(e, vec![]);
    assert_eq!(h.snapshot().game, None, "sem jogo aberto mostra o preset global");
    h.move_technique(&TechId::new("Outra", "Outra.fx"), -1);
    let names: Vec<_> = h.snapshot().techniques.iter().map(|t| t.id.name.clone()).collect();
    assert_eq!(names, ["Outra", "Sepia"]);
    h.move_technique(&TechId::new("Outra", "Outra.fx"), -5);
    assert_eq!(h.snapshot().techniques[0].id.name, "Outra", "limitado ao começo");
    h.remove_technique(&TechId::new("Outra", "Outra.fx"));
    assert_eq!(h.snapshot().techniques.len(), 1);
    assert_eq!(h.snapshot().techniques[0].id.name, "Sepia");
}

#[test]
fn what_the_layer_did_with_each_technique_is_shown_for_the_open_game() {
    let e = env("runtime");
    fs::write(e.root.join("Presets/preset.ini"), "Techniques=Sepia@Sepia.fx,CAS@CAS.fx,Outra@Outra.fx\nTechniqueSorting=Sepia@Sepia.fx,CAS@CAS.fx,Outra@Outra.fx,Off@Off.fx\n").unwrap();
    let mut layers = open_game("vkcube");
    layers[0].status.techniques = vec![
        ipc::TechniqueStatus { name: "Sepia".into(), state: TechniqueState::Applied },
        ipc::TechniqueStatus { name: "CAS".into(), state: TechniqueState::Skipped("passe 0: escrita sRGB numa textura".into()) },
    ];
    let (h, _) = home(e, layers);
    let rt: Vec<_> = h.snapshot().techniques.iter().map(|t| (t.id.name.as_str(), t.runtime.clone())).collect();
    assert_eq!(
        rt,
        [
            ("Sepia", Some(TechniqueState::Applied)),
            ("CAS", Some(TechniqueState::Skipped("passe 0: escrita sRGB numa textura".into()))),
            ("Outra", None), // a layer não a conhece (ainda não construiu)
            ("Off", None),   // desligada
        ]
    );
}

#[test]
fn the_effect_analysis_shows_up_in_the_snapshot_as_the_worker_saves_it() {
    let e = env("analysis");
    let compiler_root = e.root.clone();
    let (mut h, _) = home(e, vec![]);
    assert_eq!(h.snapshot().analysis, (0, 1));
    assert!(h.snapshot().infos.is_empty());
    // O worker (aqui, à mão) analisa o efeito e salva.
    let c = Compiler { fx2spv: compiler_root.join("fake-fx2spv"), effects: vec![config::SearchPath::flat(compiler_root.join("effects"))], includes: vec![], textures: vec![], cache: compiler_root.join("cache") };
    analysis::ensure(&c, "Sepia");
    h.refresh();
    assert_eq!(h.snapshot().analysis, (1, 1));
    let info = &h.snapshot().infos["Sepia"];
    assert!(info.compatible());
    assert_eq!(info.techniques, ["Sepia"]);
}

#[test]
fn removing_the_last_technique_of_an_effect_deletes_its_settings() {
    let (mut h, presets) = home(env("forget"), open_game("vkcube"));
    h.add_effect("Sepia");
    h.set_control("Sepia.fx", "Strength", &[0.25]);
    let file = presets.join("vkcube/Padrão.ini");
    assert!(fs::read_to_string(&file).unwrap().contains("[Sepia.fx]"));
    // Desligar não apaga nada: as configurações ficam.
    h.set_technique_enabled(&sepia(), false);
    assert!(fs::read_to_string(&file).unwrap().contains("Strength=0.25"));
    // Remover esquece o efeito e as configurações; adicionar de novo começa dos padrões.
    h.remove_technique(&sepia());
    assert!(!fs::read_to_string(&file).unwrap().contains("Sepia"), "{}", fs::read_to_string(&file).unwrap());
    h.add_effect("Sepia");
    assert_eq!(h.snapshot().techniques[0].controls[0].value, vec![1.0]);
}

fn blur_techniques(h: &mut Home) {
    // Um efeito com duas técnicas (como o desfoque gaussiano horizontal e vertical), no mesmo arquivo.
    h.edit(|p, _| {
        p.set_enabled(&TechId::new("Horizontal", "Blur.fx"), true);
        p.set_enabled(&TechId::new("Vertical", "Blur.fx"), true);
        Ok(None)
    });
}

#[test]
fn techniques_of_one_effect_are_one_group_with_one_set_of_controls() {
    let (mut h, _) = home(env("groups"), open_game("vkcube"));
    h.add_effect("Sepia");
    blur_techniques(&mut h);
    let g = &h.snapshot().groups;
    assert_eq!(g.iter().map(|g| (g.file.as_str(), g.techniques.len())).collect::<Vec<_>>(), [("Sepia.fx", 1), ("Blur.fx", 2)]);
    assert_eq!(g[0].name, "Sepia");
    assert_eq!(g[0].controls.len(), 1, "os parâmetros são do efeito");
    assert!(g[1].techniques.iter().all(|t| t.controls.is_empty()), "e não se repetem em cada técnica");
}

#[test]
fn an_effect_moves_removes_and_toggles_as_a_whole() {
    let (mut h, presets) = home(env("group-actions"), open_game("vkcube"));
    h.add_effect("Sepia");
    blur_techniques(&mut h);
    h.set_control("Sepia.fx", "Strength", &[0.5]);
    let order = |h: &Home| h.snapshot().techniques.iter().map(|t| t.id.name.clone()).collect::<Vec<_>>();
    assert_eq!(order(&h), ["Sepia", "Horizontal", "Vertical"]);

    h.move_effect("Blur.fx", -1); // as duas técnicas sobem juntas, na mesma ordem
    assert_eq!(order(&h), ["Horizontal", "Vertical", "Sepia"]);
    h.move_effect("Blur.fx", 1);
    assert_eq!(order(&h), ["Sepia", "Horizontal", "Vertical"]);
    h.move_effect("Sepia.fx", -5);
    assert_eq!(order(&h), ["Sepia", "Horizontal", "Vertical"], "já é o primeiro");

    h.set_effect_enabled("Blur.fx", false);
    assert!(h.snapshot().groups[1].techniques.iter().all(|t| !t.enabled));
    assert!(h.snapshot().groups[0].all_enabled());
    h.set_technique_enabled(&TechId::new("Horizontal", "Blur.fx"), true);
    assert!(h.snapshot().groups[1].any_enabled() && !h.snapshot().groups[1].all_enabled());

    h.remove_effect("Sepia.fx");
    assert_eq!(order(&h), ["Horizontal", "Vertical"]);
    assert!(!fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap().contains("Sepia"), "as configurações vão junto");
}

#[test]
fn macros_of_the_preset_and_of_an_effect_are_edited_and_validated() {
    let (mut h, presets) = home(env("macros"), open_game("vkcube"));
    h.add_effect("Sepia");
    h.set_preset_macros("QUALITY=2, FLAG");
    assert_eq!(h.snapshot().preset_macros, "QUALITY=2,FLAG=1", "valor vazio vale 1");
    h.set_effect_macros("Sepia.fx", "FAST=3");
    assert_eq!(h.snapshot().groups[0].macros, "FAST=3");
    let text = fs::read_to_string(presets.join("vkcube/Padrão.ini")).unwrap();
    assert!(text.contains("PreprocessorDefinitions=QUALITY=2,FLAG=1") && text.contains("PreprocessorDefinitions=FAST=3"), "{text}");

    h.set_preset_macros("1ruim=2");
    assert!(h.snapshot().notice.as_deref().is_some_and(|n| n.contains("invalid definition")), "{:?}", h.snapshot().notice);
    assert_eq!(h.snapshot().preset_macros, "QUALITY=2,FLAG=1", "a inválida não foi gravada");
    h.set_preset_macros("");
    assert_eq!(h.snapshot().preset_macros, "");
}

#[test]
fn deleting_a_preset_from_the_screen() {
    let (mut h, presets) = home(env("delete"), open_game("vkcube"));
    h.add_effect("Sepia");
    h.new_preset("Noturno");
    h.delete_preset("Noturno");
    assert!(!presets.join("vkcube/Noturno.ini").exists());
    assert_eq!(h.snapshot().presets, ["Padrão"]);
    assert_eq!(h.snapshot().active_preset, None, "o jogo ficou sem o preset ativo: volta ao global (que não existe)");
    h.delete_preset("Nada");
    assert!(h.snapshot().notice.is_some());
}

#[test]
fn adding_an_effect_that_does_not_exist_says_why() {
    let (mut h, _) = home(env("whynot"), open_game("vkcube"));
    h.add_effect("Inexistente");
    let notice = h.snapshot().notice.clone().unwrap();
    assert!(notice.starts_with("Inexistente:") && notice.contains("could not find"), "{notice}");
}

#[test]
fn an_effect_that_was_never_compiled_shows_a_problem_not_a_crash() {
    let e = env("problem");
    fs::write(e.root.join("Presets/preset.ini"), "Techniques=Fantasma@Fantasma.fx\n").unwrap();
    let (h, _) = home(e, vec![]);
    let t = &h.snapshot().techniques[0];
    assert_eq!(t.label, "Fantasma");
    assert!(t.problem.as_deref().is_some_and(|p| p.contains("not compiled")), "{:?}", t.problem);
    assert!(t.controls.is_empty());
}

#[test]
fn presets_of_a_game_can_be_created_and_switched() {
    let (mut h, _) = home(env("presets"), open_game("vkcube"));
    h.add_effect("Sepia");
    h.new_preset("Noturno");
    assert_eq!(h.snapshot().active_preset.as_deref(), Some("Noturno"));
    assert_eq!(h.snapshot().presets, ["Noturno", "Padrão"]);
    h.set_technique_enabled(&sepia(), false);
    h.select_preset(Some("Padrão"));
    assert!(h.snapshot().techniques[0].enabled, "o Padrão não foi tocado");
    h.select_preset(Some("Noturno"));
    assert!(!h.snapshot().techniques[0].enabled);
    h.select_preset(None);
    assert_eq!(h.snapshot().active_preset, None, "sem preset global, o jogo fica sem nada");
    assert!(h.snapshot().techniques.is_empty());
}

#[test]
fn a_file_edited_from_outside_shows_up_on_the_next_refresh() {
    let e = env("outside");
    fs::write(e.root.join("Presets/preset.ini"), "Techniques=Sepia@Sepia.fx\n").unwrap();
    let (mut h, presets) = home(e, vec![]);
    assert!(h.snapshot().techniques[0].enabled);
    fs::write(presets.join("preset.ini"), "Techniques=\nTechniqueSorting=Sepia@Sepia.fx\n").unwrap();
    fs::File::options().write(true).open(presets.join("preset.ini")).unwrap().set_modified(SystemTime::now() + std::time::Duration::from_secs(5)).unwrap();
    h.refresh();
    assert!(!h.snapshot().techniques[0].enabled);
}

#[test]
fn choosing_the_game_changes_what_is_shown_and_an_unknown_id_is_ignored() {
    let e = env("games");
    fs::create_dir_all(e.root.join("Presets/Antigo")).unwrap();
    fs::write(e.root.join("Presets/Antigo/game.ini"), "preset=A.ini\n").unwrap();
    fs::write(e.root.join("Presets/Antigo/A.ini"), "Techniques=Sepia@Sepia.fx\n").unwrap();
    let (mut h, _) = home(e, open_game("vkcube"));
    let ids: Vec<_> = h.snapshot().games.iter().map(|g| (g.id.as_str(), g.open)).collect();
    assert_eq!(ids, [("vkcube", true), ("Antigo", false)], "os abertos primeiro");
    h.select_game(Some("Antigo"));
    assert_eq!(h.snapshot().techniques.len(), 1);
    h.select_game(Some("../x"));
    assert_eq!(h.snapshot().game, None, "id inválido vira o preset global");
}

#[test]
fn the_global_enable_switch_goes_through_the_shared_state() {
    let (mut h, _) = home(env("switch"), vec![]);
    assert!(!h.snapshot().enabled);
    h.set_enabled(true);
    assert!(h.snapshot().enabled);
}
