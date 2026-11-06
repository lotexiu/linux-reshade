//! Que controle mostrar para cada parâmetro de um efeito, derivado das anotações `ui_*` que o próprio
//! autor do efeito escreveu (a mesma convenção do ReShade). Sem nada de toolkit: a interface só desenha.

use fx_bundle::manifest::{Annotations, Scalar, ScalarBase, Uniform, Value};
use fx_bundle::{Bundle, uniforms};

/// O tipo de controle.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `bool`: um interruptor.
    Switch,
    /// Número com faixa conhecida: um slider.
    Slider { min: f64, max: f64, step: f64 },
    /// Escolha de uma opção (`ui_type = "combo"`); o valor é o índice.
    Combo { items: Vec<String> },
    /// Cor (`ui_type = "color"`, `float3`/`float4`), componentes de 0 a 1.
    Color { alpha: bool },
    /// Número sem faixa: um campo de entrada.
    Number { min: Option<f64>, max: Option<f64>, step: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    /// O nome do uniform no efeito (é o que `lrctl set` usa).
    pub name: String,
    /// O texto mostrado: `ui_label`, ou o nome.
    pub label: String,
    pub tooltip: Option<String>,
    pub category: Option<String>,
    /// `float`, `int`, `float3`...
    pub type_name: String,
    pub kind: Kind,
    /// Quantos componentes tem o valor (1 para escalares).
    pub components: usize,
    pub value: Vec<f64>,
    pub default: Vec<f64>,
}

impl Control {
    pub fn is_default(&self) -> bool {
        self.value == self.default
    }
}

fn text<'a>(a: &'a Annotations, key: &str) -> Option<&'a str> {
    match a.get(key) {
        Some(Value::Scalar(Scalar::Text(t))) => Some(t),
        _ => None,
    }
}

/// O manifesto guarda os números como `f32` (`0.01` vira `0.00999999978`); aqui voltam ao que o autor escreveu.
fn clean(n: f64) -> f64 {
    (n as f32).to_string().parse().unwrap_or(n)
}

fn number(a: &Annotations, key: &str) -> Option<f64> {
    match a.get(key) {
        Some(Value::Scalar(Scalar::Number(n))) => Some(clean(*n)),
        Some(Value::List(l)) => l.iter().find_map(|s| if let Scalar::Number(n) = s { Some(clean(*n)) } else { None }),
        _ => None,
    }
}

/// Texto de efeito para a tela: um NUL no meio derruba o GTK (que usa strings C), então vira espaço.
fn no_nul(s: &str) -> String {
    s.replace('\0', " ")
}

fn flag(a: &Annotations, key: &str) -> bool {
    matches!(a.get(key), Some(Value::Scalar(Scalar::Bool(true)))) || number(a, key).is_some_and(|n| n != 0.0)
}

/// `ui_items`: as opções, separadas por NUL (`"Baixo\0Médio\0Alto\0"`).
fn items(a: &Annotations) -> Vec<String> {
    let Some(raw) = text(a, "ui_items") else { return Vec::new() };
    raw.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn type_name(u: &Uniform) -> String {
    let base = match u.ty.base {
        ScalarBase::Bool => "bool",
        ScalarBase::Int => "int",
        ScalarBase::Uint => "uint",
        ScalarBase::Float => "float",
        _ => "?",
    };
    if u.ty.rows > 1 { format!("{base}{}", u.ty.rows) } else { base.to_string() }
}

/// O tipo de controle de um uniform, ou `None` se ele não é um controle (preenchido pelo runtime,
/// escondido pelo autor, ou de um tipo que não sabemos editar).
pub fn kind_of(u: &Uniform) -> Option<Kind> {
    let a = &u.annotations;
    if uniforms::is_runtime_source(u) || flag(a, "hidden") || u.ty.cols != 1 || u.ty.array != 0 {
        return None;
    }
    let ui_type = text(a, "ui_type").unwrap_or("");
    let (min, max) = (number(a, "ui_min"), number(a, "ui_max"));
    let integer = matches!(u.ty.base, ScalarBase::Int | ScalarBase::Uint);
    let step = number(a, "ui_step").filter(|s| *s > 0.0);
    match u.ty.base {
        ScalarBase::Bool => Some(Kind::Switch),
        ScalarBase::Float | ScalarBase::Int | ScalarBase::Uint => {
            if ui_type == "color" && u.ty.base == ScalarBase::Float && matches!(u.ty.rows, 3 | 4) {
                return Some(Kind::Color { alpha: u.ty.rows == 4 });
            }
            if ui_type == "combo" && integer && u.ty.rows == 1 {
                let items = items(a);
                if !items.is_empty() {
                    return Some(Kind::Combo { items });
                }
            }
            match (min, max) {
                // Com faixa, um slider; passo: o do autor, ou 1 para inteiros, ou 1/100 da faixa.
                (Some(min), Some(max)) if max > min => {
                    Some(Kind::Slider { min, max, step: if integer { step.unwrap_or(1.0) } else { fine_step(min, max, step) } })
                }
                _ => Some(Kind::Number { min, max, step: step.unwrap_or(if integer { 1.0 } else { 0.01 }) }),
            }
        }
        _ => None,
    }
}

/// Os controles de um efeito, na ordem do manifesto, com os valores de `block` (a imagem do buffer de uniforms).
pub fn controls_of(bundle: &Bundle, block: &[u8]) -> Vec<Control> {
    let defaults = uniforms::default_block(&bundle.manifest);
    bundle
        .manifest
        .uniforms
        .iter()
        .filter_map(|u| {
            let kind = kind_of(u)?;
            let value = uniforms::decode(u, block)?;
            Some(Control {
                name: u.name.clone(),
                label: no_nul(text(&u.annotations, "ui_label").unwrap_or(&u.name)),
                tooltip: text(&u.annotations, "ui_tooltip").map(no_nul),
                category: text(&u.annotations, "ui_category").map(no_nul),
                type_name: type_name(u),
                kind,
                components: value.len(),
                default: uniforms::decode(u, &defaults).unwrap_or_else(|| value.clone()),
                value,
            })
        })
        .collect()
}

/// O passo de um slider decimal: o do autor do efeito costuma ser grosso demais para se ajustar com a mão (o ReShade
/// arrasta de forma contínua). Vale o menor entre o dele e 1/1000 da faixa, arredondado para uma potência de dez.
fn fine_step(min: f64, max: f64, author: Option<f64>) -> f64 {
    let range = (max - min).abs().max(1e-9);
    let wanted = author.unwrap_or(range / 100.0).min(range / 1000.0);
    10f64.powf(wanted.log10().floor())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fx_bundle::manifest::Manifest;
    use std::path::Path;

    /// Um uniform de teste, com as anotações dadas (em JSON).
    fn uniform(base: &str, rows: u32, annotations: &str) -> Uniform {
        let manifest = Manifest::parse(&format!(
            r#"{{"format":"linux-reshade-bundle","version":1,"source":"t.fx",
            "compiler":{{"name":"x","reshade_commit":"x","spirv_headers_commit":"x"}},
            "target":{{"width":8,"height":8,"color_space":"srgb"}},"spirv_version":"1.0",
            "entry_points":[],"total_uniform_size":16,"techniques":[],
            "uniforms":[{{"name":"u","unique_name":"u","type":{{"base":"{base}","rows":{rows},"cols":1,"array":0}},
            "offset":0,"size":{},"default":null,"annotations":{annotations}}}]}}"#,
            rows * 4
        ))
        .unwrap();
        manifest.uniforms[0].clone()
    }

    #[test]
    fn a_bool_is_a_switch() {
        assert_eq!(kind_of(&uniform("bool", 1, "{}")), Some(Kind::Switch));
    }

    #[test]
    fn a_number_with_a_range_is_a_slider_with_a_sensible_step() {
        let float = kind_of(&uniform("float", 1, r#"{"ui_type":"slider","ui_min":0.0,"ui_max":2.0}"#));
        assert_eq!(float, Some(Kind::Slider { min: 0.0, max: 2.0, step: 0.001 }));
        let int = kind_of(&uniform("int", 1, r#"{"ui_min":1,"ui_max":10}"#));
        assert_eq!(int, Some(Kind::Slider { min: 1.0, max: 10.0, step: 1.0 }), "sem ui_type, mas com faixa");
        let custom = kind_of(&uniform("float", 1, r#"{"ui_min":0,"ui_max":1,"ui_step":0.25}"#));
        assert_eq!(custom, Some(Kind::Slider { min: 0.0, max: 1.0, step: 0.001 }), "o passo do autor, grosso demais, é afinado");
    }

    #[test]
    fn numbers_written_by_the_author_come_back_as_written() {
        // Como o fx2spv os grava: floats de 32 bits em 9 dígitos.
        let k = kind_of(&uniform("float", 1, r#"{"ui_min":0.100000001,"ui_max":1,"ui_step":0.00999999978}"#));
        assert_eq!(k, Some(Kind::Slider { min: 0.1, max: 1.0, step: 0.0001 }));
        assert_eq!(clean(0.699999988), 0.7);
        assert_eq!(clean(8.0), 8.0);
        assert_eq!(clean(-1.0), -1.0);
    }

    #[test]
    fn a_number_without_a_range_is_a_free_input() {
        assert_eq!(kind_of(&uniform("float", 1, "{}")), Some(Kind::Number { min: None, max: None, step: 0.01 }));
        assert_eq!(kind_of(&uniform("uint", 1, r#"{"ui_min":0}"#)), Some(Kind::Number { min: Some(0.0), max: None, step: 1.0 }));
        // faixa degenerada (max <= min) não vira slider
        assert!(matches!(kind_of(&uniform("float", 1, r#"{"ui_min":1,"ui_max":1}"#)), Some(Kind::Number { .. })));
    }

    #[test]
    fn combo_splits_the_nul_separated_items() {
        let k = kind_of(&uniform("int", 1, r#"{"ui_type":"combo","ui_items":"Baixo\u0000Médio\u0000Alto\u0000"}"#));
        assert_eq!(k, Some(Kind::Combo { items: vec!["Baixo".into(), "Médio".into(), "Alto".into()] }));
        // sem opções declaradas, volta a ser um número
        assert!(matches!(kind_of(&uniform("int", 1, r#"{"ui_type":"combo"}"#)), Some(Kind::Number { .. })));
    }

    #[test]
    fn colors_need_three_or_four_floats() {
        assert_eq!(kind_of(&uniform("float", 3, r#"{"ui_type":"color"}"#)), Some(Kind::Color { alpha: false }));
        assert_eq!(kind_of(&uniform("float", 4, r#"{"ui_type":"color"}"#)), Some(Kind::Color { alpha: true }));
        assert!(matches!(kind_of(&uniform("float", 2, r#"{"ui_type":"color"}"#)), Some(Kind::Number { .. })), "float2 não é cor");
    }

    #[test]
    fn what_is_not_a_control_is_left_out() {
        assert_eq!(kind_of(&uniform("float", 1, r#"{"source":"timer"}"#)), None, "preenchido pelo runtime");
        assert_eq!(kind_of(&uniform("float", 1, r#"{"hidden":true}"#)), None, "escondido pelo autor");
        assert!(kind_of(&uniform("float", 1, r#"{"hidden":false}"#)).is_some());
    }

    /// Com um bundle de verdade: rótulo, faixa e valores atuais vindos do bloco.
    #[test]
    fn the_sepia_bundle_gives_a_labelled_slider() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fx-bundle/tests/data/Sepia");
        let bundle = Bundle::load(&dir).unwrap();
        let mut block = uniforms::default_block(&bundle.manifest);
        let controls = controls_of(&bundle, &block);
        assert_eq!(controls.len(), 1);
        let c = &controls[0];
        assert_eq!((c.name.as_str(), c.label.as_str(), c.type_name.as_str()), ("Strength", "Intensidade", "float"));
        assert_eq!(c.kind, Kind::Slider { min: 0.0, max: 1.0, step: 0.001 });
        assert_eq!((c.value.clone(), c.default.clone(), c.is_default()), (vec![1.0], vec![1.0], true));

        // O valor atual vem do bloco, e o padrão continua o do manifesto.
        block[..4].copy_from_slice(&0.25f32.to_le_bytes());
        let c = &controls_of(&bundle, &block)[0];
        assert_eq!((c.value.clone(), c.default.clone(), c.is_default()), (vec![0.25], vec![1.0], false));
    }
}
