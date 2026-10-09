use super::*;
use std::collections::HashMap;
use std::fs;

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("config-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.path(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    /// Um ambiente com `HOME` e `XDG_*` dentro da caixa, mais o que o teste acrescentar.
    fn env(&self, extra: &[(&str, &str)]) -> HashMap<String, OsString> {
        let mut m: HashMap<String, OsString> = HashMap::new();
        m.insert("HOME".into(), self.path("home").into());
        for (k, v) in extra {
            m.insert((*k).into(), (*v).into());
        }
        m
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn resolve(env: &HashMap<String, OsString>) -> Config {
    Config::resolve(&|k| env.get(k).cloned())
}

#[test]
fn search_path_parsing() {
    let base = Path::new("/dados");
    let home = Some(Path::new("/home/u"));
    let p = |s: &str| SearchPath::parse(s, base, home);
    assert_eq!(p("./Shaders/**"), SearchPath { dir: "/dados/Shaders".into(), recursive: true });
    assert_eq!(p("Shaders"), SearchPath { dir: "/dados/Shaders".into(), recursive: false });
    assert_eq!(p("/mnt/D/shaders"), SearchPath { dir: "/mnt/D/shaders".into(), recursive: false });
    assert_eq!(p("~/fx/**"), SearchPath { dir: "/home/u/fx".into(), recursive: true });
    assert_eq!(p("../outro"), SearchPath { dir: "/outro".into(), recursive: false });
    assert_eq!(p("**"), SearchPath { dir: "/dados".into(), recursive: true });
    assert!(p(".\\Shaders\\**").recursive, "barra invertida do ReShade também vale");
}

#[test]
fn recursive_expansion_is_sorted_and_skips_hidden() {
    let sb = Sandbox::new("expand");
    for d in ["S/b/deep", "S/a", "S/.git/x", "S/c"] {
        fs::create_dir_all(sb.path(d)).unwrap();
    }
    fs::write(sb.path("S/arquivo"), "").unwrap();
    let dirs = SearchPath { dir: sb.path("S"), recursive: true }.dirs();
    let rel: Vec<_> = dirs.iter().map(|d| d.strip_prefix(&sb.root).unwrap().to_string_lossy().into_owned()).collect();
    assert_eq!(rel, ["S", "S/a", "S/b", "S/b/deep", "S/c"]);
    let flat = SearchPath { dir: sb.path("S"), recursive: false }.dirs();
    assert_eq!(flat.len(), 1);
}

#[test]
fn nothing_configured() {
    let sb = Sandbox::new("none");
    let c = resolve(&sb.env(&[]));
    assert!(!c.is_configured());
    assert_eq!(c.data_dir, None);
    assert!(c.effect_paths.is_empty());
    assert_eq!(c.presets_dir, None);
    assert_eq!(c.cache_dir, sb.path("home/.cache/linux-reshade/bundles"));
}

#[test]
fn data_dir_gives_defaults_inside_it() {
    let sb = Sandbox::new("defaults");
    let data = sb.path("dados");
    fs::create_dir_all(&data).unwrap();
    let c = resolve(&sb.env(&[("LINUX_RESHADE_DATA", data.to_str().unwrap())]));
    assert!(c.is_configured());
    assert_eq!(c.effect_paths, [SearchPath { dir: data.join("Shaders"), recursive: true }]);
    assert_eq!(c.texture_paths, [SearchPath { dir: data.join("Textures"), recursive: true }]);
    assert_eq!(c.presets_dir, Some(data.join("Presets")));
}

#[test]
fn location_ini_points_to_the_data_dir() {
    let sb = Sandbox::new("location");
    let data = sb.path("meus dados");
    sb.write("home/.config/linux-reshade/location.ini", &format!("data_dir={}\n", data.display()));
    let c = resolve(&sb.env(&[]));
    assert_eq!(c.data_dir, Some(data.clone()));
    assert_eq!(c.presets_dir, Some(data.join("Presets")));
    // XDG_CONFIG_HOME tem prioridade sobre ~/.config.
    sb.write("xdg/linux-reshade/location.ini", "data_dir=/outra\n");
    let c = resolve(&sb.env(&[("XDG_CONFIG_HOME", sb.path("xdg").to_str().unwrap())]));
    assert_eq!(c.data_dir, Some(PathBuf::from("/outra")));
}

#[test]
fn env_beats_location_file() {
    let sb = Sandbox::new("envwins");
    sb.write("home/.config/linux-reshade/location.ini", "data_dir=/do/arquivo\n");
    let c = resolve(&sb.env(&[("LINUX_RESHADE_DATA", "/do/ambiente")]));
    assert_eq!(c.data_dir, Some(PathBuf::from("/do/ambiente")));
}

#[test]
fn config_ini_overrides_defaults_and_resolves_relative_to_itself() {
    let sb = Sandbox::new("configini");
    let data = sb.path("dados");
    sb.write(
        "dados/config.ini",
        "[GENERAL]\nEffectSearchPaths=./Meus/**, /mnt/D/shaders\nTextureSearchPaths=../Texturas\nPresetsPath=/mnt/D/presets\nIntermediateCachePath=./cache\n",
    );
    let c = resolve(&sb.env(&[("LINUX_RESHADE_DATA", data.to_str().unwrap())]));
    assert_eq!(
        c.effect_paths,
        [SearchPath { dir: data.join("Meus"), recursive: true }, SearchPath { dir: "/mnt/D/shaders".into(), recursive: false }]
    );
    assert_eq!(c.texture_paths, [SearchPath { dir: sb.path("Texturas"), recursive: false }]);
    assert_eq!(c.presets_dir, Some(PathBuf::from("/mnt/D/presets")));
    assert_eq!(c.cache_dir, data.join("cache"));
    assert_eq!(c.config_file, Some(data.join("config.ini")));
}

#[test]
fn env_lists_override_config_ini() {
    let sb = Sandbox::new("envlists");
    let data = sb.path("dados");
    sb.write("dados/config.ini", "[GENERAL]\nEffectSearchPaths=./Meus\n");
    let c = resolve(&sb.env(&[
        ("LINUX_RESHADE_DATA", data.to_str().unwrap()),
        ("LINUX_RESHADE_EFFECTS", "/a:/b/**"),
        ("LINUX_RESHADE_PRESETS", "/p"),
        ("LINUX_RESHADE_CACHE", "/c"),
    ]));
    assert_eq!(c.effect_paths, [SearchPath { dir: "/a".into(), recursive: false }, SearchPath { dir: "/b".into(), recursive: true }]);
    assert_eq!(c.presets_dir, Some(PathBuf::from("/p")));
    assert_eq!(c.cache_dir, PathBuf::from("/c"));
}

#[test]
fn per_game_folder_is_searched_before_the_global_one() {
    let sb = Sandbox::new("layers");
    let data = sb.path("dados");
    for d in ["dados/Shaders/cores", "dados/Presets/RocketLeague/Shaders", "dados/Presets/Outro"] {
        fs::create_dir_all(sb.path(d)).unwrap();
    }
    let c = resolve(&sb.env(&[("LINUX_RESHADE_DATA", data.to_str().unwrap())]));
    assert_eq!(
        c.effect_dirs(Some("RocketLeague")),
        [data.join("Presets/RocketLeague/Shaders"), data.join("Shaders"), data.join("Shaders/cores")]
    );
    // Jogo sem pasta própria de shaders, ou sem jogo: só as globais.
    assert_eq!(c.effect_dirs(Some("Outro")), [data.join("Shaders"), data.join("Shaders/cores")]);
    assert_eq!(c.effect_dirs(None), [data.join("Shaders"), data.join("Shaders/cores")]);
    // Texturas seguem a mesma regra (a pasta do jogo não tem `Textures`).
    assert_eq!(c.texture_dirs(Some("RocketLeague")), [data.join("Textures")]);
}

#[test]
fn init_creates_the_layout_and_the_pointer_without_overwriting() {
    let sb = Sandbox::new("init");
    let data = sb.path("dados");
    let env = sb.env(&[]);
    let lookup = |k: &str| env.get(k).cloned();
    let done = init(&data, &lookup).unwrap();
    for d in ["Shaders", "Textures", "Presets"] {
        assert!(data.join(d).is_dir(), "{d}");
    }
    assert!(data.join("config.ini").is_file());
    assert!(done.created.len() >= 5);
    assert_eq!(done.location, Some(sb.path("home/.config/linux-reshade/location.ini")));

    // Depois de `init`, `resolve` acha tudo sozinho, sem variáveis.
    let c = resolve(&env);
    assert_eq!(c.data_dir, Some(data.clone()));
    assert_eq!(c.effect_paths, [SearchPath { dir: data.join("Shaders"), recursive: true }]);

    // Rodar de novo não cria nada e não mexe no `config.ini` editado pelo usuário.
    sb.write("dados/config.ini", "[GENERAL]\nPresetsPath=/meu\n; nota\n");
    let again = init(&data, &lookup).unwrap();
    assert!(again.created.is_empty(), "{:?}", again.created);
    assert_eq!(fs::read_to_string(data.join("config.ini")).unwrap(), "[GENERAL]\nPresetsPath=/meu\n; nota\n");
}

#[test]
fn the_default_data_dir_counts_only_once_it_exists() {
    let sb = Sandbox::new("default-dir");
    let env = sb.env(&[]);
    assert_eq!(default_data_dir(&|k| env.get(k).cloned()), Some(sb.path("home/.local/share/linux-reshade")));
    assert_eq!(resolve(&env).data_dir, None);

    fs::create_dir_all(sb.path("home/.local/share/linux-reshade")).unwrap();
    let c = resolve(&env);
    assert_eq!(c.data_dir, Some(sb.path("home/.local/share/linux-reshade")));
    assert!(c.is_configured());
    assert_eq!(c.presets_dir, Some(sb.path("home/.local/share/linux-reshade/Presets")));

    // XDG_DATA_HOME manda; e uma escolha explícita vale mais que a padrão.
    let env = sb.env(&[("XDG_DATA_HOME", sb.path("xdg").to_str().unwrap())]);
    assert_eq!(default_data_dir(&|k| env.get(k).cloned()), Some(sb.path("xdg/linux-reshade")));
    sb.write("home/.config/linux-reshade/location.ini", "data_dir=/outra\n");
    assert_eq!(resolve(&sb.env(&[])).data_dir, Some(PathBuf::from("/outra")));
}
