//! Descobre os idiomas: cada `locales/<código>.tsv` vira um idioma embutido no programa. Acrescentar um idioma é acrescentar o arquivo.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut codes: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| {
                    let path = e.path();
                    (path.extension()? == "tsv").then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned())).flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    codes.sort();
    let mut out = String::from("/// Os catálogos embutidos (um por arquivo de `locales/`).\npub const BUILTIN: &[(&str, &str)] = &[\n");
    for code in &codes {
        let path = dir.join(format!("{code}.tsv"));
        let _ = writeln!(out, "    ({code:?}, include_str!({:?})),", path.display().to_string());
        println!("cargo:rerun-if-changed={}", path.display());
    }
    out.push_str("];\n");
    let target = Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("builtin_locales.rs");
    std::fs::write(target, out).expect("gravar builtin_locales.rs");
}
