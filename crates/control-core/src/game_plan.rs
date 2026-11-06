//! Do preset do jogo ao plano que a layer executa.
//!
//! Para cada técnica ligada, na ordem de execução do preset: compila o efeito (o bundle também dá o layout
//! dos uniforms), confere que a técnica existe e monta a imagem do buffer de uniforms (padrões do manifesto
//! com os valores do preset por cima). O que não puder entrar (efeito que não existe, que não compila, técnica
//! que o arquivo não declara) vira um **aviso** e a etapa é pulada: o resto do preset continua valendo, como
//! o ReShade faz com técnica desconhecida.
//!
//! Macros (`PreprocessorDefinitions`) ainda não recompilam o efeito: ficam para a F6.

use crate::analysis::{self, Feature};
use crate::compiler::{Compiler, Defines, valid_define, valid_name, variant_key};
use fx_bundle::{Bundle, uniforms};
use ipc::{Plan, PlanStep};
use preset::{Macros, Preset, TechId, merge_macros};
use i18n::{fmt};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Built {
    pub plan: Plan,
    /// O que ficou de fora e por quê (uma linha por problema).
    pub warnings: Vec<String>,
    /// Cada efeito do plano com as definições com que foi compilado (para compilar as outras resoluções).
    pub variants: Vec<(String, Defines)>,
}

/// As técnicas ligadas do preset, na ordem de execução.
pub fn enabled_order(preset: &Preset) -> Vec<TechId> {
    let mut known = preset.sorting();
    for t in preset.techniques() {
        if !known.iter().any(|k| k.matches(&t)) {
            known.push(t);
        }
    }
    preset.resolve(&known).enabled().cloned().collect()
}

/// O valor de um uniform como o ReShade o grava: números separados por vírgula (`true`/`false` valem para bool).
fn parse_components(text: &str) -> Option<Vec<f64>> {
    // Versões anteriores gravaram vetores como `(a, b, c)`: ainda se leem.
    let text = text.trim().trim_start_matches('(').trim_end_matches(')');
    text.split(',')
        .map(|part| match part.trim() {
            "true" => Some(1.0),
            "false" => Some(0.0),
            other => other.parse::<f64>().ok(),
        })
        .collect()
}

/// A imagem do buffer de uniforms de um efeito para este preset. Os valores que não se aplicam (uniform que
/// o efeito não tem mais, preenchido pelo runtime, valor malformado) viram avisos.
pub fn uniform_block(bundle: &Bundle, preset: &Preset, file: &str, warnings: &mut Vec<String>) -> Vec<u8> {
    let mut block = uniforms::default_block(&bundle.manifest);
    for (name, text) in preset.values(file) {
        let Some(u) = uniforms::find(&bundle.manifest, name) else { continue }; // sobra de outra versão do efeito
        if uniforms::is_runtime_source(u) {
            continue;
        }
        let Some(values) = parse_components(text) else {
            warnings.push(fmt("{}: {}={} is not a number", &[&file, &name, &format!("{text:?}")]));
            continue;
        };
        match uniforms::encode(u, &values) {
            Ok(bytes) => match block.get_mut(u.offset as usize..u.offset as usize + bytes.len()) {
                Some(slot) => slot.copy_from_slice(&bytes),
                None => warnings.push(fmt("{}: {} does not fit in the buffer", &[&file, &name])),
            },
            Err(e) => warnings.push(format!("{file}: {e}")),
        }
    }
    block
}

/// As definições que a layer deduz do jogo para os efeitos que usam profundidade (precedência: acima das globais, abaixo
/// do preset e do efeito). Hoje: `RESHADE_DEPTH_INPUT_IS_REVERSED`, pela profundidade que o jogo usa (invertida ou não).
pub fn auto_depth_macros(reversed: Option<bool>) -> Macros {
    match reversed {
        Some(r) => vec![("RESHADE_DEPTH_INPUT_IS_REVERSED".to_string(), if r { "1" } else { "0" }.to_string())],
        None => Macros::new(),
    }
}

/// As definições de um efeito: as globais (`config.ini`), as do preset e as do próprio efeito, nessa ordem de
/// precedência. As que não podem ir ao compilador (nome que não é identificador) viram avisos.
pub fn defines_for(preset: &Preset, global: &Macros, file: &str, warnings: &mut Vec<String>) -> Defines {
    let mut all = preset.effective_macros(global, file);
    all.retain(|(name, value)| {
        let ok = valid_define(name, value);
        if !ok {
            warnings.push(fmt("{}: invalid definition {}={}; ignored", &[&file, &name, &format!("{value:?}")]));
        }
        ok
    });
    all
}

/// Monta o plano de `preset` para uma resolução (compila o que faltar). A resolução só serve para obter o
/// manifesto: o plano vale para qualquer tamanho (o modelo de caminho leva `{W}x{H}`). Cada efeito é compilado com as
/// definições (macros) que valem para ele; a mesma combinação reaproveita o bundle.
pub fn build(compiler: &Compiler, preset: &Preset, global: &Macros, auto_depth: &Macros, width: u32, height: u32) -> Built {
    let mut built = Built::default();
    for id in enabled_order(preset) {
        let Some(file) = id.file.as_deref() else {
            built.warnings.push(fmt("technique {} has no file (expected Name@File.fx)", &[&id.name]));
            continue;
        };
        let name = file.strip_suffix(".fx").unwrap_or(file);
        if !valid_name(name) {
            built.warnings.push(fmt("{}: invalid effect name", &[&id]));
            continue;
        }
        let mut warnings = Vec::new();
        // As definições deduzidas da profundidade só valem para os efeitos que a usam (os demais não ganham variante à toa).
        let base = if auto_depth.is_empty() || !analysis::ensure(compiler, name).has(Feature::Depth) { global.clone() } else { merge_macros(&[global, auto_depth]) };
        let defines = defines_for(preset, &base, file, &mut warnings);
        let fresh: Vec<String> = warnings.into_iter().filter(|w| !built.warnings.contains(w)).collect();
        built.warnings.extend(fresh);
        let dir = match compiler.ensure_with(name, width, height, &defines) {
            Ok(ensured) => ensured.dir_owned(),
            Err(e) => {
                built.warnings.push(format!("{id}: {e}"));
                continue;
            }
        };
        let bundle = match Bundle::load(&dir) {
            Ok(b) => b,
            Err(e) => {
                built.warnings.push(format!("{id}: {e}"));
                continue;
            }
        };
        if !bundle.manifest.techniques.iter().any(|t| t.name == id.name) {
            built.warnings.push(fmt("{}: the effect does not declare this technique", &[&id]));
            continue;
        }
        let block = uniform_block(&bundle, preset, file, &mut built.warnings);
        let key = variant_key(name, &defines);
        if !built.variants.iter().any(|(n, d)| n == name && *d == defines) {
            built.variants.push((name.to_string(), defines.clone()));
        }
        built.plan.steps.push(PlanStep { template: compiler.template(&key), technique: id.name.clone(), params_id: bundle.params_id(), uniforms: block });
    }
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::testing::fake_with_bundles;

    fn setup(name: &str) -> crate::compiler::testing::Fake {
        fake_with_bundles(name)
    }

    fn strength(step: &PlanStep) -> f32 {
        f32::from_le_bytes(step.uniforms[..4].try_into().unwrap())
    }

    #[test]
    fn builds_the_enabled_techniques_in_preset_order_with_their_values() {
        let f = setup("plan-order");
        let preset = Preset::parse("Techniques=Invert@Invert.fx,Sepia@Sepia.fx\nTechniqueSorting=Sepia@Sepia.fx,Invert@Invert.fx\n\n[Sepia.fx]\nStrength=0.25\n");
        let built = build(&f.compiler, &preset, &Macros::new(), &Macros::new(), 8, 4);
        assert!(built.warnings.is_empty(), "{:?}", built.warnings);
        let names: Vec<_> = built.plan.steps.iter().map(|s| s.technique.as_str()).collect();
        assert_eq!(names, ["Sepia", "Invert"], "a ordem é a de TechniqueSorting");
        assert_eq!(strength(&built.plan.steps[0]), 0.25, "valor do preset sobre o padrão");
        assert!(built.plan.steps[0].template.ends_with("/Sepia-{W}x{H}"));
        assert_ne!(built.plan.steps[0].params_id, built.plan.steps[1].params_id);
    }

    #[test]
    fn the_default_applies_when_the_preset_has_no_value() {
        let f = setup("plan-default");
        let built = build(&f.compiler, &Preset::parse("Techniques=Sepia@Sepia.fx\n"), &Macros::new(), &Macros::new(), 8, 4);
        assert_eq!(strength(&built.plan.steps[0]), 1.0, "Strength padrão do efeito");
    }

    #[test]
    fn disabled_techniques_are_left_out() {
        let f = setup("plan-disabled");
        let preset = Preset::parse("Techniques=Sepia@Sepia.fx\nTechniqueSorting=Invert@Invert.fx,Sepia@Sepia.fx\n");
        let built = build(&f.compiler, &preset, &Macros::new(), &Macros::new(), 8, 4);
        assert_eq!(built.plan.steps.len(), 1);
        assert_eq!(built.plan.steps[0].technique, "Sepia");
    }

    #[test]
    fn a_step_that_cannot_be_built_is_a_warning_and_the_rest_goes_on() {
        let f = setup("plan-warnings");
        let preset = Preset::parse(
            "Techniques=Fantasma@Fantasma.fx,Quebrado@Quebrado.fx,SemArquivo,Outra@Sepia.fx,Sepia@Sepia.fx,Sepia@..%2Fx.fx\n",
        );
        let built = build(&f.compiler, &preset, &Macros::new(), &Macros::new(), 8, 4);
        let kept: Vec<_> = built.plan.steps.iter().map(|s| s.technique.as_str()).collect();
        assert_eq!(kept, ["Sepia"]);
        let text = built.warnings.join("\n");
        assert!(text.contains("Fantasma@Fantasma.fx"), "{text}");
        assert!(text.contains("Quebrado@Quebrado.fx"), "{text}");
        assert!(text.contains("SemArquivo has no file"), "{text}");
        assert!(text.contains("Outra@Sepia.fx: the effect does not declare this technique"), "{text}");
        assert_eq!(built.warnings.len(), 5, "{text}");
    }

    #[test]
    fn bad_values_warn_and_keep_the_default() {
        let f = setup("plan-values");
        let preset = Preset::parse("Techniques=Sepia@Sepia.fx\n\n[Sepia.fx]\nStrength=muito\nNaoExiste=3\n");
        let built = build(&f.compiler, &preset, &Macros::new(), &Macros::new(), 8, 4);
        assert_eq!(strength(&built.plan.steps[0]), 1.0);
        assert_eq!(built.warnings, ["Sepia.fx: Strength=\"muito\" is not a number"], "uniform desconhecido é ignorado sem aviso");
    }

    #[test]
    fn the_depth_macro_is_deduced_only_for_effects_that_use_depth_and_the_preset_wins() {
        let f = setup("plan-depth-macro");
        let preset = Preset::parse("Techniques=Sepia@Sepia.fx,Depth@Depth.fx\n");
        let reversed = auto_depth_macros(Some(true));
        let built = build(&f.compiler, &preset, &Macros::new(), &reversed, 8, 4);
        assert!(built.warnings.is_empty(), "{:?}", built.warnings);
        let templates: Vec<_> = built.plan.steps.iter().map(|s| s.template.clone()).collect();
        assert!(!templates[0].contains(".m"), "o Sépia não usa profundidade: sem variante ({templates:?})");
        assert!(templates[1].contains("Depth.m"), "{templates:?}");
        let (_, defines) = built.variants.iter().find(|(n, _)| n == "Depth").unwrap();
        assert_eq!(defines, &vec![("RESHADE_DEPTH_INPUT_IS_REVERSED".to_string(), "1".to_string())]);

        // Normal (0) também vale; e o que o usuário escreveu no preset vence o deduzido.
        let normal = build(&f.compiler, &preset, &Macros::new(), &auto_depth_macros(Some(false)), 8, 4);
        assert_eq!(normal.variants.iter().find(|(n, _)| n == "Depth").unwrap().1[0].1, "0");
        let user = Preset::parse("Techniques=Depth@Depth.fx\n\n[Depth.fx]\nPreprocessorDefinitions=RESHADE_DEPTH_INPUT_IS_REVERSED=0\n");
        let own = build(&f.compiler, &user, &Macros::new(), &reversed, 8, 4);
        assert_eq!(own.variants[0].1, vec![("RESHADE_DEPTH_INPUT_IS_REVERSED".to_string(), "0".to_string())]);
        // Sem saber (None): nada é acrescentado.
        assert!(!build(&f.compiler, &preset, &Macros::new(), &auto_depth_macros(None), 8, 4).plan.steps[1].template.contains(".m"));
    }

    #[test]
    fn an_empty_preset_gives_an_empty_plan() {
        let f = setup("plan-empty");
        let built = build(&f.compiler, &Preset::parse("Techniques=\n"), &Macros::new(), &Macros::new(), 8, 4);
        assert_eq!(built, Built::default());
    }

    #[test]
    fn components_accept_numbers_and_booleans() {
        assert_eq!(parse_components("1.0, 0.5,0"), Some(vec![1.0, 0.5, 0.0]));
        assert_eq!(parse_components("true"), Some(vec![1.0]));
        assert_eq!(parse_components("x"), None);
        assert_eq!(parse_components("(0.5, 0.25, 1)"), Some(vec![0.5, 0.25, 1.0]), "o formato antigo, com parênteses");
    }
}
