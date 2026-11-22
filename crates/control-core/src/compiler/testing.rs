use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

pub struct Fake {
    pub root: PathBuf,
    pub compiler: Compiler,
}

/// Escreve `manifest.json` no diretório dado por `-o` e anota os argumentos em `calls.log`.
pub const OK_TOOL: &str = r#"#!/bin/sh
out=""; prev=""
for a in "$@"; do [ "$prev" = "-o" ] && out="$a"; prev="$a"; done
echo "$@" >> "$(dirname "$0")/calls.log"
mkdir -p "$out" && echo '{}' > "$out/manifest.json"
"#;

/// Um `fx2spv` de mentira que entrega os bundles reais de teste do postfx (Sepia, Invert, ...) no lugar do
/// que compilaria o `.fx` de mesmo nome; um efeito sem bundle de teste falha. Também anota `calls.log`.
pub fn bundles_tool() -> String {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../postfx/tests/data");
    format!(
        "#!/bin/sh\nout=\"\"; prev=\"\"; for a in \"$@\"; do [ \"$prev\" = \"-o\" ] && out=\"$a\"; prev=\"$a\"; src=\"$a\"; done\n\
         echo \"$@\" >> \"$(dirname \"$0\")/calls.log\"\n\
         name=$(basename \"$src\" .fx)\n[ -d \"{d}/$name\" ] || {{ echo \"sem bundle de teste para $name\" >&2; exit 1; }}\n\
         mkdir -p \"$out\" && cp -r \"{d}/$name/.\" \"$out/\"\n",
        d = data.display()
    )
}

/// Como [`fake`], com `bundles_tool` e os efeitos `Invert` e `Quebrado` (este não tem bundle de teste).
pub fn fake_with_bundles(name: &str) -> Fake {
    let f = fake(name, &bundles_tool());
    for fx in ["Invert", "Quebrado", "Depth"] {
        fs::write(f.root.join(format!("effects/{fx}.fx")), "// de mentira").unwrap();
    }
    f
}

pub fn calls(f: &Fake) -> Vec<String> {
    fs::read_to_string(f.root.join("calls.log")).map(|t| t.lines().map(String::from).collect()).unwrap_or_default()
}

pub fn fake(name: &str, tool_script: &str) -> Fake {
    let root = std::env::temp_dir().join(format!("lr-compiler-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("effects")).unwrap();
    fs::write(root.join("effects/Sepia.fx"), "// fx de mentira").unwrap();
    let tool = root.join("fake-fx2spv");
    fs::write(&tool, tool_script).unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    let compiler = Compiler {
        fx2spv: tool,
        effects: vec![SearchPath::flat(root.join("effects"))],
        includes: vec![SearchPath::flat(root.join("inc"))],
        textures: vec![SearchPath::flat(root.join("textures"))],
        cache: root.join("cache"),
    };
    Fake { root, compiler }
}
