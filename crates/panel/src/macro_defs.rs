//! As definições de pré-processador que a pessoa costuma mexer, descritas para a interface mostrar
//! "nome à esquerda, controle à direita" em vez de um campo de texto `NOME=VALOR,OUTRO=2`.
//!
//! O texto continua sendo a forma de guardar (o formato do `PreprocessorDefinitions` do ReShade); este módulo só
//! lê e escreve esse texto uma definição por vez.

use crate::macros_text;

/// Que controle serve a uma definição.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MacroKind {
    /// Liga/desliga (0 ou 1); `default` é o valor do ReShade quando a definição não existe.
    Switch { default: bool },
    /// Automático (a layer decide, se souber), Sim ou Não.
    Auto,
    /// Um número; sem a definição vale `default`.
    Number { default: f64, min: f64, max: f64, step: f64, digits: u32 },
}

#[derive(Debug, Clone, Copy)]
pub struct KnownMacro {
    pub name: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub kind: MacroKind,
}

const fn number(default: f64, min: f64, max: f64, step: f64, digits: u32) -> MacroKind {
    MacroKind::Number { default, min, max, step, digits }
}

/// As definições do `ReShade.fxh` que mexem na leitura da profundidade (as que a maioria precisa).
pub const KNOWN: &[KnownMacro] = &[
    KnownMacro { name: "RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN", label: "Upside-down depth", hint: "Turn on if the sky appears at the bottom and the ground on top in DisplayDepth (common in Unity games).", kind: MacroKind::Switch { default: false } },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_IS_REVERSED", label: "Reversed depth", hint: "Near is 1 and far is 0. Automatic: the layer detects it in the open game.", kind: MacroKind::Auto },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_IS_MIRRORED", label: "Mirrored depth", hint: "Mirrors the depth horizontally.", kind: MacroKind::Switch { default: false } },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_IS_LOGARITHMIC", label: "Logarithmic depth", hint: "The game stores depth on a logarithmic scale.", kind: MacroKind::Switch { default: false } },
    KnownMacro { name: "RESHADE_DEPTH_LINEARIZATION_FAR_PLANE", label: "Far plane", hint: "Distance (in game units) treated as the background when linearizing.", kind: number(1000.0, 1.0, 1_000_000.0, 10.0, 1) },
    KnownMacro { name: "RESHADE_DEPTH_MULTIPLIER", label: "Depth multiplier", hint: "Multiplies the depth read.", kind: number(1.0, 0.0, 1000.0, 0.1, 2) },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_X_SCALE", label: "Horizontal scale", hint: "For games whose depth has a different aspect ratio than the screen.", kind: number(1.0, 0.01, 10.0, 0.01, 3) },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_Y_SCALE", label: "Vertical scale", hint: "For games whose depth has a different aspect ratio than the screen.", kind: number(1.0, 0.01, 10.0, 0.01, 3) },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_X_OFFSET", label: "Horizontal offset", hint: "As a fraction of the width.", kind: number(0.0, -1.0, 1.0, 0.001, 3) },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_Y_OFFSET", label: "Vertical offset", hint: "As a fraction of the height.", kind: number(0.0, -1.0, 1.0, 0.001, 3) },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_X_PIXEL_OFFSET", label: "Horizontal offset (pixels)", hint: "In pixels.", kind: number(0.0, -4096.0, 4096.0, 1.0, 0) },
    KnownMacro { name: "RESHADE_DEPTH_INPUT_Y_PIXEL_OFFSET", label: "Vertical offset (pixels)", hint: "In pixels.", kind: number(0.0, -4096.0, 4096.0, 1.0, 0) },
];

pub fn known(name: &str) -> Option<&'static KnownMacro> {
    KNOWN.iter().find(|k| k.name == name)
}

/// O valor de uma definição no texto, se estiver lá.
pub fn get(text: &str, name: &str) -> Option<String> {
    preset::parse_macros(text).into_iter().find(|(n, _)| n == name).map(|(_, v)| v)
}

/// O texto com a definição trocada (ou removida, com `None`). A ordem das outras não muda.
pub fn set(text: &str, name: &str, value: Option<&str>) -> String {
    let mut all = preset::parse_macros(text);
    match (all.iter().position(|(n, _)| n == name), value) {
        (Some(i), Some(v)) => all[i].1 = v.to_string(),
        (Some(i), None) => {
            all.remove(i);
        }
        (None, Some(v)) => all.push((name.to_string(), v.to_string())),
        (None, None) => {}
    }
    macros_text(&all)
}

/// As definições do texto que não estão no catálogo (as que a pessoa escreveu por conta própria).
pub fn custom(text: &str) -> Vec<(String, String)> {
    preset::parse_macros(text).into_iter().filter(|(n, _)| known(n).is_none()).collect()
}

/// Como escrever um número: sem casas inúteis (`1`, não `1.000`).
pub fn number_text(value: f64, digits: u32) -> String {
    let s = format!("{value:.*}", digits as usize);
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_remove_keep_the_others() {
        let t = set("A=1,B=2", "RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN", Some("1"));
        assert_eq!(t, "A=1,B=2,RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1");
        assert_eq!(get(&t, "B").as_deref(), Some("2"));
        assert_eq!(set(&t, "B", None), "A=1,RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1");
        assert_eq!(set(&t, "A", Some("5")), "A=5,B=2,RESHADE_DEPTH_INPUT_IS_UPSIDE_DOWN=1");
        assert_eq!(set("", "X", None), "");
    }

    #[test]
    fn custom_leaves_out_the_catalog() {
        assert_eq!(custom("RESHADE_DEPTH_INPUT_IS_REVERSED=1,MINHA=3"), vec![("MINHA".to_string(), "3".to_string())]);
    }

    #[test]
    fn numbers_are_written_without_useless_digits() {
        assert_eq!(number_text(1000.0, 1), "1000");
        assert_eq!(number_text(0.5, 3), "0.5");
        assert_eq!(number_text(-3.0, 0), "-3");
    }

    #[test]
    fn catalog_names_are_valid_defines() {
        for k in KNOWN {
            assert!(control_core::compiler::valid_define(k.name, "1"), "{}", k.name);
        }
    }
}
