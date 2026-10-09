//! Testes com o bundle real gerado pelo fx2spv (`tests/data/Sepia`, de `effects/Sepia.fx`) e com
//! variações inválidas dele. Para regenerar o golden: `scripts/dev.sh fx-golden`.

use fx_bundle::manifest::{Scalar, Stage, Value};
use fx_bundle::{Bundle, Error, Manifest};
use serde_json::{Value as Json, json};
use std::fs;
use std::path::{Path, PathBuf};

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/Sepia")
}

fn golden_json() -> Json {
    serde_json::from_str(&fs::read_to_string(golden().join("manifest.json")).unwrap()).unwrap()
}

/// Aplica `edit` ao manifesto do golden e devolve o resultado de parse + validação.
fn mutated(edit: impl FnOnce(&mut Json)) -> Result<Manifest, Error> {
    let mut v = golden_json();
    edit(&mut v);
    let m = Manifest::parse(&v.to_string())?;
    m.validate()?;
    Ok(m)
}

#[test]
fn loads_the_golden_bundle() {
    let bundle = Bundle::load(&golden()).expect("carregar");
    let m = &bundle.manifest;
    assert_eq!((m.target.width, m.target.height), (500, 500));
    assert_eq!(m.spirv_version, "1.0");

    assert_eq!(m.techniques.len(), 1);
    assert_eq!(m.techniques[0].name, "Sepia");
    let pass = &m.techniques[0].passes[0];
    assert_eq!(pass.vertex_entry, "E__VS_Fullscreen");
    assert_eq!(pass.pixel_entry, "E__PS_Sepia");
    assert_eq!((pass.samplers[0].sampler, pass.samplers[0].binding), (0, 0));
    assert!(pass.render_targets.is_empty(), "escreve direto no frame do jogo");

    // O uniform da interface: nome, deslocamento e as anotações que a UI usa.
    let strength = &m.uniforms[0];
    assert_eq!(strength.name, "Strength");
    assert_eq!((strength.offset, strength.size), (0, 4));
    assert_eq!(strength.default, Some(Value::Scalar(Scalar::Number(1.0))));
    assert_eq!(strength.annotations["ui_type"], Value::Scalar(Scalar::Text("slider".into())));
    assert_eq!(strength.annotations["ui_max"], Value::Scalar(Scalar::Number(1.0)));

    // Textura com a semântica COLOR e o sampler que a lê.
    assert_eq!(m.textures[0].semantic.as_deref(), Some("COLOR"));
    assert_eq!(m.samplers[0].texture, m.textures[0].unique_name);

    for e in &m.entry_points {
        assert!(bundle.shader(&e.name).is_some_and(|w| w[0] == 0x0723_0203), "SPIR-V de {}", e.name);
    }
    assert_eq!(m.entry_points.iter().filter(|e| e.stage == Stage::Pixel).count(), 1);
}

#[test]
fn ignores_unknown_fields() {
    assert!(mutated(|v| {
        v["campo_novo"] = json!({"a": 1});
        v["techniques"][0]["passes"][0]["outro_campo"] = json!(true);
    })
    .is_ok());
}

#[test]
fn accepts_a_declared_but_unused_depth_texture() {
    // É o caso do ReShade.fxh: declara DepthBufferTex : DEPTH, mas o efeito não a amostra.
    let m = mutated(|v| {
        let mut depth = v["textures"][0].clone();
        depth["name"] = json!("DepthBufferTex");
        depth["unique_name"] = json!("V__DepthBufferTex");
        depth["semantic"] = json!("DEPTH");
        v["textures"].as_array_mut().unwrap().push(depth);
    });
    assert!(m.is_ok(), "{m:?}");
}

#[test]
fn rejects_unknown_enum_values() {
    let r = mutated(|v| v["samplers"][0]["min_filter"] = json!("cubic"));
    assert!(matches!(r, Err(Error::Parse(_))), "{r:?}");
}

#[test]
fn rejects_what_v1_does_not_run() {
    let depth = mutated(|v| v["textures"][0]["semantic"] = json!("DEPTH"));
    assert!(matches!(depth, Err(Error::Unsupported(_))), "{depth:?}");

    let compute = mutated(|v| v["techniques"][0]["passes"][0]["compute_entry"] = json!("CS"));
    assert!(matches!(compute, Err(Error::Unsupported(_))), "{compute:?}");

    let texture_3d = mutated(|v| v["textures"][0]["type"] = json!("3d"));
    assert!(matches!(texture_3d, Err(Error::Unsupported(_))), "{texture_3d:?}");

    let future = mutated(|v| v["version"] = json!(2));
    assert!(matches!(future, Err(Error::Unsupported(_))), "{future:?}");

    let huge = mutated(|v| v["total_uniform_size"] = json!(1_000_000));
    assert!(matches!(huge, Err(Error::Unsupported(_))), "{huge:?}");
}

#[test]
fn rejects_inconsistent_references() {
    let cases: Vec<(&str, Box<dyn FnOnce(&mut Json)>)> = vec![
        ("formato errado", Box::new(|v| v["format"] = json!("outra-coisa"))),
        ("entry point inexistente", Box::new(|v| v["techniques"][0]["passes"][0]["pixel_entry"] = json!("nada"))),
        ("estágio trocado", Box::new(|v| v["techniques"][0]["passes"][0]["pixel_entry"] = json!("E__VS_Fullscreen"))),
        ("sampler inexistente no passe", Box::new(|v| v["techniques"][0]["passes"][0]["samplers"][0]["sampler"] = json!(7))),
        ("textura inexistente no sampler", Box::new(|v| v["samplers"][0]["texture"] = json!("nada"))),
        ("uniform fora do buffer", Box::new(|v| v["uniforms"][0]["offset"] = json!(4096))),
        ("anexo que não é textura", Box::new(|v| v["techniques"][0]["passes"][0]["render_targets"] = json!(["nada"]))),
        ("anexo que não é alvo", Box::new(|v| v["techniques"][0]["passes"][0]["render_targets"] = json!([v["textures"][0]["unique_name"].clone()]))),
        ("sem técnicas", Box::new(|v| v["techniques"] = json!([]))),
        ("resolução zero", Box::new(|v| v["target"]["width"] = json!(0))),
        ("0 vértices", Box::new(|v| v["techniques"][0]["passes"][0]["num_vertices"] = json!(0))),
    ];
    for (what, edit) in cases {
        let r = mutated(edit);
        assert!(matches!(r, Err(Error::Invalid(_)) | Err(Error::Parse(_))), "{what}: {r:?}");
    }
}

#[test]
fn rejects_file_names_that_escape_the_directory() {
    for bad in ["../x.spv", "/etc/passwd.spv", "a/b.spv", "..", "sem-extensao", ".oculto.spv", ""] {
        let r = mutated(|v| v["entry_points"][0]["file"] = json!(bad));
        assert!(matches!(r, Err(Error::Invalid(_))), "{bad:?}: {r:?}");
    }
}

/// Copia o golden para um diretório temporário para poder estragá-lo.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fx-bundle-test-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for entry in fs::read_dir(golden()).unwrap() {
        let entry = entry.unwrap();
        fs::copy(entry.path(), dir.join(entry.file_name())).unwrap();
    }
    dir
}

#[test]
fn rejects_broken_files_on_disk() {
    let missing = scratch("missing");
    fs::remove_file(missing.join("E__PS_Sepia.spv")).unwrap();
    assert!(matches!(Bundle::load(&missing), Err(Error::Io { .. })));

    let truncated = scratch("truncated");
    let spv = truncated.join("E__PS_Sepia.spv");
    let bytes = fs::read(&spv).unwrap();
    fs::write(&spv, &bytes[..bytes.len() - 3]).unwrap();
    assert!(matches!(Bundle::load(&truncated), Err(Error::Invalid(_))));

    let bad_magic = scratch("magic");
    let spv = bad_magic.join("E__VS_Fullscreen.spv");
    let mut bytes = fs::read(&spv).unwrap();
    bytes[0] ^= 0xFF;
    fs::write(&spv, &bytes).unwrap();
    assert!(matches!(Bundle::load(&bad_magic), Err(Error::Invalid(_))));

    let not_json = scratch("json");
    fs::write(not_json.join("manifest.json"), "isto não é json").unwrap();
    assert!(matches!(Bundle::load(&not_json), Err(Error::Parse(_))));

    assert!(matches!(Bundle::load(Path::new("/nao/existe")), Err(Error::Io { .. })));
}

/// Carrega todos os bundles que `scripts/dev.sh fx-test` gerou em `build/bundles` (efeitos próprios e
/// da comunidade). O leitor nunca pode falhar com `Parse` ou `Invalid` num bundle que o fx2spv gerou;
/// `Unsupported` é aceitável (por exemplo, efeitos que usam depth). Se não houver bundles, avisa e passa.
#[test]
fn reads_every_bundle_the_compiler_produced() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/bundles");
    let Ok(entries) = fs::read_dir(&dir) else {
        eprintln!("sem {}; rode scripts/dev.sh fx-test para exercitar este teste", dir.display());
        return;
    };
    let (mut loaded, mut unsupported) = (0, Vec::new());
    for entry in entries.flatten().filter(|e| e.path().join("manifest.json").is_file()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        match Bundle::load(&entry.path()) {
            Ok(_) => loaded += 1,
            Err(Error::Unsupported(why)) => unsupported.push(format!("{name}: {why}")),
            Err(e) => panic!("{name}: o leitor recusou um bundle do próprio compilador: {e}"),
        }
    }
    eprintln!("{loaded} bundles carregados; {} não suportados na v1:", unsupported.len());
    for u in &unsupported {
        eprintln!("  {u}");
    }
}
