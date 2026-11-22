use super::testing::*;
use super::*;
use std::fs;
use std::time::Duration;

/// 1x1 RGBA transparente.
const PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08,
    0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x60, 0x60, 0x60, 0x60, 0x00, 0x00,
    0x00, 0x05, 0x00, 0x01, 0xA5, 0xF6, 0x45, 0x40, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82
];

#[test]
fn the_image_of_a_file_texture_is_decoded_into_the_bundle() {
    let f = fake_with_bundles("texture-file");
    fs::write(f.root.join("effects/Tex.fx"), "// de mentira").unwrap();
    // O nome pedido pelo efeito (img.png) acha o arquivo mesmo com outras maiúsculas.
    fs::create_dir_all(f.root.join("textures")).unwrap();
    fs::write(f.root.join("textures/IMG.PNG"), PNG_1X1).unwrap();
    let ensured = f.compiler.ensure("Tex", 8, 4).unwrap();
    let image = fs::read(fx_bundle::images::path(ensured.dir(), 0)).unwrap();
    let image = fx_bundle::images::decode(&image).unwrap();
    assert_eq!((image.width, image.height, image.rgba.len()), (4, 2, 32), "esticada ao tamanho que o manifesto declara para a textura");
    assert!(f.compiler.is_current("Tex", 8, 4));

    // O PNG foi editado depois: o bundle deixa de estar atual e é refeito.
    let later = SystemTime::now() + Duration::from_secs(120);
    fs::File::options().write(true).open(f.root.join("textures/IMG.PNG")).unwrap().set_modified(later).unwrap();
    assert!(!f.compiler.is_current("Tex", 8, 4));
}

#[test]
fn a_missing_texture_image_fails_the_compile_and_leaves_no_bundle() {
    let f = fake_with_bundles("texture-missing");
    fs::write(f.root.join("effects/Tex.fx"), "// de mentira").unwrap();
    let err = f.compiler.ensure("Tex", 8, 4).unwrap_err();
    assert!(matches!(err, CompileError::Texture(_)), "{err}");
    assert!(err.to_string().contains("img.png"), "{err}");
    assert!(!f.compiler.bundle_dir("Tex", 8, 4).exists(), "não fica um bundle sem a imagem");
    assert!(!f.compiler.is_current("Tex", 8, 4));
}

#[test]
fn a_texture_that_is_not_an_image_fails_the_compile() {
    let f = fake_with_bundles("texture-bad");
    fs::write(f.root.join("effects/Tex.fx"), "// de mentira").unwrap();
    fs::create_dir_all(f.root.join("textures")).unwrap();
    fs::write(f.root.join("textures/img.png"), b"nao sou um png").unwrap();
    let err = f.compiler.ensure("Tex", 8, 4).unwrap_err();
    assert!(matches!(err, CompileError::Texture(_)), "{err}");
}

#[test]
fn definitions_make_a_separate_bundle_with_a_stable_key() {
    assert_eq!(variant_key("Sepia", &[]), "Sepia");
    let a = variant_key("Sepia", &[("A".into(), "1".into()), ("B".into(), "2".into())]);
    let b = variant_key("Sepia", &[("B".into(), "2".into()), ("A".into(), "1".into())]);
    assert_eq!(a, b, "a ordem não importa");
    assert_ne!(a, variant_key("Sepia", &[("A".into(), "1".into())]));
    assert!(a.starts_with("Sepia.m") && valid_name(&a), "{a}");
    assert_eq!(base_name(&a), "Sepia");
    assert_eq!(base_name("Sepia"), "Sepia");
    assert_eq!(base_name("a.b"), "a.b", "um ponto no nome não é uma definição");
    assert_eq!(base_name("Film.mcafebabe"), "Film");
}

#[test]
fn compiling_with_definitions_passes_them_and_keeps_a_separate_bundle() {
    let f = fake("defines", OK_TOOL);
    let defs = vec![("QUALITY".to_string(), "2".to_string()), ("FAST".to_string(), "1".to_string())];
    let with = f.compiler.ensure_with("Sepia", 800, 600, &defs).unwrap();
    let plain = f.compiler.ensure("Sepia", 800, 600).unwrap();
    assert_ne!(with.dir(), plain.dir());
    let calls = calls(&f);
    assert!(calls[0].contains("-D QUALITY=2") && calls[0].contains("-D FAST=1"), "{}", calls[0]);
    assert!(!calls[1].contains("-D"), "{}", calls[1]);
    // O mesmo pedido de novo usa o cache, e a data do .fx vale para as duas variantes.
    assert!(matches!(f.compiler.ensure_with("Sepia", 800, 600, &defs).unwrap(), Ensured::Cached(_)));
    assert!(f.compiler.is_current(&variant_key("Sepia", &defs), 800, 600));
    // Definição que não é um identificador não chega à linha de comando.
    assert!(f.compiler.ensure_with("Sepia", 1, 1, &[("A B".into(), "1".into())]).is_err());
    assert!(f.compiler.ensure_with("Sepia", 1, 1, &[("A".into(), "x\ny".into())]).is_err());
}

#[test]
fn names_cannot_escape_the_cache() {
    for ok in ["Sepia", "Film_Grain-2", "a.b", "Adventurous MLUT", "Look (2)"] {
        assert!(valid_name(ok), "{ok}");
    }
    for bad in ["", "../x", "a/b", "/etc/passwd", ".oculto", " a", "a ", "a;b", "a&b", "é", "a\\b"] {
        assert!(!valid_name(bad), "{bad}");
    }
    let f = fake("names", OK_TOOL);
    assert!(matches!(f.compiler.ensure("../Sepia", 10, 10), Err(CompileError::BadName(_))));
    assert!(calls(&f).is_empty(), "nem chegou a chamar a ferramenta");
}

#[test]
fn the_template_roundtrips_to_the_effect_name() {
    let f = fake("template", OK_TOOL);
    let t = f.compiler.template("Sepia");
    assert!(t.ends_with("/Sepia-{W}x{H}"), "{t}");
    assert_eq!(f.compiler.name_from_template(&t), Some("Sepia".into()));
    assert_eq!(f.compiler.name_from_template("/outro/lugar/Sepia-{W}x{H}"), None, "fora do cache");
    assert_eq!(f.compiler.name_from_template(&f.compiler.bundle_dir("Sepia", 10, 10).to_string_lossy()), None, "diretório literal");
}

#[test]
fn compiles_once_then_uses_the_cache() {
    let f = fake("cache", OK_TOOL);
    let first = f.compiler.ensure("Sepia", 1920, 1080).unwrap();
    assert_eq!(first, Ensured::Compiled(f.compiler.bundle_dir("Sepia", 1920, 1080)));
    assert!(first.dir().join("manifest.json").is_file());

    let second = f.compiler.ensure("Sepia", 1920, 1080).unwrap();
    assert_eq!(second, Ensured::Cached(f.compiler.bundle_dir("Sepia", 1920, 1080)));
    assert_eq!(calls(&f).len(), 1, "a segunda vez não chamou a ferramenta");

    // Outra resolução é outro bundle.
    assert!(matches!(f.compiler.ensure("Sepia", 1914, 1048).unwrap(), Ensured::Compiled(_)));
    assert_eq!(calls(&f).len(), 2);
}

#[test]
fn the_tool_receives_the_size_includes_output_and_source() {
    let f = fake("args", OK_TOOL);
    f.compiler.ensure("Sepia", 800, 600).unwrap();
    let call = &calls(&f)[0];
    assert!(call.starts_with("--width 800 --height 600 -I "), "{call}");
    assert!(call.contains("/inc -o "), "{call}");
    assert!(call.ends_with("/effects/Sepia.fx"), "{call}");
    assert!(call.contains("/cache/building/Sepia-800x600."), "o bundle é montado numa pasta de trabalho: {call}");
}

#[test]
fn the_bundle_only_appears_in_place_once_it_is_complete() {
    // Quem olha o cache enquanto se compila (a layer de um jogo que abre) não pode ver o bundle pela metade.
    let f = fake("atomic", OK_TOOL);
    f.compiler.ensure("Sepia", 800, 600).unwrap();
    let call = &calls(&f)[0];
    let out = call.split(" -o ").nth(1).unwrap().split(' ').next().unwrap();
    assert!(!out.ends_with("/cache/Sepia-800x600"), "o compilador escreveu direto no lugar final: {out}");
    assert!(f.compiler.bundle_dir("Sepia", 800, 600).join("manifest.json").is_file());
    assert!(!std::path::Path::new(out).exists(), "a pasta de trabalho some depois de publicar");
}

#[test]
fn a_known_source_defect_is_compiled_from_a_patched_copy_and_the_bundle_goes_stale_without_the_marker() {
    let f = fake("patched", OK_TOOL);
    let original = "for(uint b = 1; b <= AF_POINTS; b <<= 1, barrier())";
    fs::write(f.root.join("effects/MartysMods_DEPTHOFFIELD.fx"), original).unwrap();
    assert!(matches!(f.compiler.ensure("MartysMods_DEPTHOFFIELD", 64, 64).unwrap(), Ensured::Compiled(_)));
    let call = &calls(&f)[0];
    assert!(call.contains("/cache/patched/MartysMods_DEPTHOFFIELD-64x64/MartysMods_DEPTHOFFIELD.fx"), "{call}");
    assert!(f.root.join("cache/patched/MartysMods_DEPTHOFFIELD-64x64/Sepia.fx").exists(), "o que há ao lado do original aparece na pasta de trabalho");
    assert_eq!(fs::read_to_string(f.root.join("effects/MartysMods_DEPTHOFFIELD.fx")).unwrap(), original, "o original não muda");
    assert!(fs::read_to_string(f.root.join("cache/patched/MartysMods_DEPTHOFFIELD-64x64/MartysMods_DEPTHOFFIELD.fx")).unwrap().contains("b < AF_POINTS"));
    assert!(f.compiler.is_current("MartysMods_DEPTHOFFIELD", 64, 64));
    // Um bundle de antes das correções (sem a anotação) fica desatualizado.
    fs::remove_file(f.root.join("cache/MartysMods_DEPTHOFFIELD-64x64/patches")).unwrap();
    assert!(!f.compiler.is_current("MartysMods_DEPTHOFFIELD", 64, 64));
}

#[test]
fn the_spirv_optimizer_keeps_every_module_valid_and_the_bundle_goes_stale_without_the_marker() {
    let Some(tool) = optimizer() else {
        eprintln!("sem spirv-opt; teste ignorado");
        return;
    };
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../postfx/tests/data/Sepia");
    let dir = std::env::temp_dir().join(format!("lr-opt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(&data).unwrap().flatten() {
        std::fs::copy(e.path(), dir.join(e.file_name())).unwrap();
    }
    optimize_bundle(&dir, tool);
    let bundle = fx_bundle::Bundle::load(&dir).expect("o bundle otimizado carrega e valida");
    assert!(!bundle.manifest.techniques.is_empty());
    assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| e.path().extension().is_none_or(|x| x != "opt")), "sem sobras temporárias");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn editing_the_source_triggers_a_recompile() {
    let f = fake("stale", OK_TOOL);
    f.compiler.ensure("Sepia", 64, 64).unwrap();
    // O .fx fica mais novo que o bundle compilado.
    let later = SystemTime::now() + Duration::from_secs(60);
    fs::File::options().write(true).open(f.root.join("effects/Sepia.fx")).unwrap().set_modified(later).unwrap();
    assert!(matches!(f.compiler.ensure("Sepia", 64, 64).unwrap(), Ensured::Compiled(_)));
    assert_eq!(calls(&f).len(), 2);
}

#[test]
fn a_failing_tool_reports_what_it_printed() {
    let f = fake("fail", "#!/bin/sh\necho 'Sepia.fx(3, 1): erro de sintaxe' >&2\nexit 1\n");
    match f.compiler.ensure("Sepia", 10, 10) {
        Err(CompileError::Tool { output, .. }) => assert!(output.contains("erro de sintaxe"), "{output}"),
        other => panic!("{other:?}"),
    }
    assert!(!f.compiler.bundle_dir("Sepia", 10, 10).join("manifest.json").exists());
}

#[test]
fn available_lists_effects_once_and_prefers_the_first_directory() {
    let f = fake("available", OK_TOOL);
    let second = f.root.join("outros");
    fs::create_dir_all(&second).unwrap();
    fs::write(second.join("Sepia.fx"), "// repetido").unwrap();
    fs::write(second.join("Vibrance.fx"), "//").unwrap();
    fs::write(second.join("ReShade.fxh"), "// header, não é um efeito").unwrap();
    fs::write(second.join("notas.txt"), "x").unwrap();
    fs::write(second.join(".oculto.fx"), "x").unwrap();
    let mut compiler = f.compiler;
    compiler.effects.push(SearchPath::flat(second.clone()));
    let names: Vec<_> = compiler.available().into_iter().map(|(n, d)| (n, d == second)).collect();
    assert_eq!(names, vec![("Sepia".to_string(), false), ("Vibrance".to_string(), true)]);
}

#[test]
fn recursive_search_path_finds_effects_in_subfolders_and_sees_new_ones() {
    let f = fake("recursive", OK_TOOL);
    let pack = f.root.join("pacote");
    fs::create_dir_all(pack.join("cores")).unwrap();
    fs::create_dir_all(pack.join(".git")).unwrap();
    fs::write(pack.join("cores/Vibrance.fx"), "//").unwrap();
    fs::write(pack.join(".git/Oculto.fx"), "//").unwrap();
    let mut compiler = f.compiler;
    compiler.effects = vec![SearchPath { dir: pack.clone(), recursive: true }];
    assert_eq!(compiler.source("Vibrance").unwrap(), pack.join("cores/Vibrance.fx"));
    assert!(compiler.source("Oculto").is_err(), "pastas ocultas ficam de fora");
    // Um efeito colocado depois que o compilador existe é visto sem recriá-lo (o daemon vive muito tempo).
    assert!(compiler.source("Novo").is_err());
    fs::create_dir_all(pack.join("novos")).unwrap();
    fs::write(pack.join("novos/Novo.fx"), "//").unwrap();
    assert_eq!(compiler.source("Novo").unwrap(), pack.join("novos/Novo.fx"));
    let names: Vec<_> = compiler.available().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["Novo", "Vibrance"]);
}

fn entry(name: &str, age_secs: u64) -> CacheEntry {
    CacheEntry {
        path: PathBuf::from("/cache").join(name),
        file_name: name.to_string(),
        modified: SystemTime::now() - Duration::from_secs(age_secs),
    }
}

fn doomed(entries: &[CacheEntry], in_use: &[(&str, u32, u32)], keep: usize) -> Vec<String> {
    let in_use = in_use.iter().map(|(n, w, h)| (n.to_string(), *w, *h)).collect();
    let mut names: Vec<_> =
        prune_plan(entries, &in_use, keep).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn prune_keeps_the_newest_of_each_effect() {
    let entries = [
        entry("Sepia-100x100", 400),
        entry("Sepia-200x200", 300),
        entry("Sepia-300x300", 200),
        entry("Sepia-400x400", 100),
        entry("Invert-100x100", 900), // outro efeito: tem a própria cota
    ];
    assert_eq!(doomed(&entries, &[], 2), ["Sepia-100x100", "Sepia-200x200"]);
    assert!(doomed(&entries, &[], 4).is_empty(), "dentro da cota nada é apagado");
}

#[test]
fn prune_never_removes_what_an_open_game_uses() {
    let entries = [entry("Sepia-100x100", 400), entry("Sepia-200x200", 300), entry("Sepia-300x300", 200)];
    assert_eq!(doomed(&entries, &[("Sepia", 100, 100)], 1), ["Sepia-200x200"], "o mais velho está em uso e fica");
}

#[test]
fn prune_clears_interrupted_compilations_and_ignores_strangers() {
    let entries = [entry("Sepia-100x100.tmp", 10), entry("anotacoes", 999), entry("Sepia-1x", 999), entry("../Sepia-5x5", 999)];
    assert_eq!(doomed(&entries, &[], 0), ["Sepia-100x100.tmp"], "só o .tmp; o que não é bundle nosso não se toca");
}

#[test]
fn prune_removes_real_directories_and_reports_how_many() {
    let f = fake("prune-real", OK_TOOL);
    for (i, (w, h)) in [(100, 100), (200, 200), (300, 300)].into_iter().enumerate() {
        f.compiler.ensure("Sepia", w, h).unwrap();
        // Envelhece em ordem: o 100x100 é o mais antigo e o 300x300, o mais novo.
        let manifest = f.compiler.bundle_dir("Sepia", w, h).join("manifest.json");
        let at = SystemTime::now() - Duration::from_secs(1000 - i as u64 * 100);
        fs::File::options().write(true).open(&manifest).unwrap().set_modified(at).unwrap();
    }
    // Fica 1 por cota (o 300x300, o mais novo), e o 100x100, o mais velho, está em uso por um jogo.
    let in_use = std::collections::HashSet::from([("Sepia".to_string(), 100, 100)]);
    assert_eq!(f.compiler.prune(&in_use, 1), 1, "só o 200x200 é apagado");
    let exists = |w, h| f.compiler.bundle_dir("Sepia", w, h).exists();
    assert!(exists(100, 100), "em uso");
    assert!(!exists(200, 200), "velho e sem uso");
    assert!(exists(300, 300), "o mais novo");
    assert_eq!(f.compiler.prune(&in_use, 1), 0, "uma segunda passada não acha mais nada");
}

#[test]
fn the_schema_bundle_comes_from_an_open_game_or_falls_back_to_the_default_size() {
    use ipc::{EffectState, LayerStatus, LiveLayer, SwapchainStatus};
    let f = fake("schema", OK_TOOL);
    let template = f.compiler.template("Sepia");
    let sc = |w, h, e| SwapchainStatus { width: w, height: h, format: 44, images: 3, effect: e };
    let game = |sw: Vec<SwapchainStatus>| LiveLayer { id: "x".into(), status: LayerStatus { perf: None, depth: Vec::new(), depth_passes: String::new(), techniques: Vec::new(), exe: String::new(), app: "A".into(), swapchains: sw } };

    // Sem jogo aberto: o tamanho padrão.
    assert_eq!(f.compiler.schema_bundle(&template, &[]).unwrap(), f.compiler.bundle_dir("Sepia", 1920, 1080));
    // Com jogo: a resolução dele (ignorando swapchains que não aceitam efeito).
    let live = [game(vec![sc(640, 480, EffectState::Unavailable("sRGB".into())), sc(1280, 720, EffectState::Waiting)])];
    assert_eq!(f.compiler.schema_bundle(&template, &live).unwrap(), f.compiler.bundle_dir("Sepia", 1280, 720));
    // Uma escolha que não é por nome não tem o que resolver.
    assert!(f.compiler.schema_bundle("/qualquer/bundle", &[]).is_err());
}

#[test]
fn any_bundle_also_finds_a_variant_compiled_with_macros() {
    let f = fake("any-variant", OK_TOOL);
    let defines = vec![("QUALITY".to_string(), "2".to_string())];
    f.compiler.ensure_with("Sepia", 100, 100, &defines).unwrap();
    let key = variant_key("Sepia", &defines);
    assert_eq!(f.compiler.any_bundle("Sepia"), Some(f.compiler.bundle_dir(&key, 100, 100)));
}

#[test]
fn any_bundle_finds_the_most_recent_compiled_resolution() {
    let f = fake("any", OK_TOOL);
    assert_eq!(f.compiler.any_bundle("Sepia"), None, "nada compilado");
    for (i, (w, h)) in [(100, 100), (200, 200)].into_iter().enumerate() {
        f.compiler.ensure("Sepia", w, h).unwrap();
        let manifest = f.compiler.bundle_dir("Sepia", w, h).join("manifest.json");
        let at = SystemTime::now() - Duration::from_secs(500 - i as u64 * 100);
        fs::File::options().write(true).open(&manifest).unwrap().set_modified(at).unwrap();
    }
    f.compiler.ensure("Invert", 1, 1).ok(); // outro efeito (sem .fx: erro ignorado) não atrapalha
    assert_eq!(f.compiler.any_bundle("Sepia"), Some(f.compiler.bundle_dir("Sepia", 200, 200)));
    assert_eq!(f.compiler.any_bundle("Invert"), None);
}

#[test]
fn an_unknown_effect_lists_where_it_looked() {
    let f = fake("unknown", OK_TOOL);
    match f.compiler.ensure("Nada", 10, 10) {
        Err(CompileError::UnknownEffect { searched, .. }) => assert_eq!(searched, f.compiler.effect_dirs()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_missing_tool_is_an_io_error_not_a_panic() {
    let mut f = fake("missing", OK_TOOL);
    f.compiler.fx2spv = f.root.join("nao-existe");
    assert!(matches!(f.compiler.ensure("Sepia", 10, 10), Err(CompileError::Io(_))));
}

/// Com o fx2spv de verdade (se foi compilado), o resultado tem de ser um bundle que carrega.
#[test]
fn the_real_tool_produces_a_loadable_bundle() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tool = repo.join("build/fx2spv/fx2spv");
    if !tool.is_file() {
        eprintln!("sem {}; rode scripts/dev.sh fx-tool para exercitar este teste", tool.display());
        return;
    }
    let cache = std::env::temp_dir().join(format!("lr-compiler-{}-real", std::process::id()));
    let _ = fs::remove_dir_all(&cache);
    let compiler = Compiler { fx2spv: tool, effects: vec![SearchPath::flat(repo.join("effects"))], includes: vec![], textures: vec![], cache };
    let ensured = compiler.ensure("Sepia", 64, 48).expect("compilar o Sépia de verdade");
    let bundle = fx_bundle::Bundle::load(ensured.dir()).expect("o bundle compilado carrega");
    assert_eq!((bundle.manifest.target.width, bundle.manifest.target.height), (64, 48));
}
