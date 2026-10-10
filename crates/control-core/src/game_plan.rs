//! Do preset do jogo ao plano que a layer executa.
//!
//! Para cada técnica ligada, na ordem de execução do preset: compila o efeito (o bundle também dá o layout
//! dos uniforms), confere que a técnica existe e monta a imagem do buffer de uniforms (padrões do manifesto
//! com os valores do preset por cima). O que não puder entrar (efeito que não existe, que não compila, técnica
//! que o arquivo não declara) vira um **aviso** e a etapa é pulada: o resto do preset continua valendo, como
//! o ReShade faz com técnica desconhecida.
//!
//! Macros (`PreprocessorDefinitions`) ainda não recompilam o efeito: ficam para a F6.

use crate::compiler::{Compiler, valid_name};
use fx_bundle::{Bundle, uniforms};
use ipc::{Plan, PlanStep};
use preset::{Preset, TechId};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Built {
    pub plan: Plan,
    /// O que ficou de fora e por quê (uma linha por problema).
    pub warnings: Vec<String>,
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
            warnings.push(format!("{file}: {name}={text:?} não é número"));
            continue;
        };
        match uniforms::encode(u, &values) {
            Ok(bytes) => match block.get_mut(u.offset as usize..u.offset as usize + bytes.len()) {
                Some(slot) => slot.copy_from_slice(&bytes),
                None => warnings.push(format!("{file}: {name} não cabe no buffer")),
            },
            Err(e) => warnings.push(format!("{file}: {e}")),
        }
    }
    block
}

/// Monta o plano de `preset` para uma resolução (compila o que faltar). A resolução só serve para obter o
/// manifesto: o plano vale para qualquer tamanho (o modelo de caminho leva `{W}x{H}`).
pub fn build(compiler: &Compiler, preset: &Preset, width: u32, height: u32) -> Built {
    let mut built = Built::default();
    for id in enabled_order(preset) {
        let Some(file) = id.file.as_deref() else {
            built.warnings.push(format!("técnica {} sem arquivo (esperava Nome@Arquivo.fx)", id.name));
            continue;
        };
        let name = file.strip_suffix(".fx").unwrap_or(file);
        if !valid_name(name) {
            built.warnings.push(format!("{id}: nome de efeito inválido"));
            continue;
        }
        let dir = match compiler.ensure(name, width, height) {
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
            built.warnings.push(format!("{id}: o efeito não declara esta técnica"));
            continue;
        }
        let block = uniform_block(&bundle, preset, file, &mut built.warnings);
        built.plan.steps.push(PlanStep { template: compiler.template(name), technique: id.name.clone(), params_id: bundle.params_id(), uniforms: block });
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
        let built = build(&f.compiler, &preset, 8, 4);
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
        let built = build(&f.compiler, &Preset::parse("Techniques=Sepia@Sepia.fx\n"), 8, 4);
        assert_eq!(strength(&built.plan.steps[0]), 1.0, "Strength padrão do efeito");
    }

    #[test]
    fn disabled_techniques_are_left_out() {
        let f = setup("plan-disabled");
        let preset = Preset::parse("Techniques=Sepia@Sepia.fx\nTechniqueSorting=Invert@Invert.fx,Sepia@Sepia.fx\n");
        let built = build(&f.compiler, &preset, 8, 4);
        assert_eq!(built.plan.steps.len(), 1);
        assert_eq!(built.plan.steps[0].technique, "Sepia");
    }

    #[test]
    fn a_step_that_cannot_be_built_is_a_warning_and_the_rest_goes_on() {
        let f = setup("plan-warnings");
        let preset = Preset::parse(
            "Techniques=Fantasma@Fantasma.fx,Quebrado@Quebrado.fx,SemArquivo,Outra@Sepia.fx,Sepia@Sepia.fx,Sepia@..%2Fx.fx\n",
        );
        let built = build(&f.compiler, &preset, 8, 4);
        let kept: Vec<_> = built.plan.steps.iter().map(|s| s.technique.as_str()).collect();
        assert_eq!(kept, ["Sepia"]);
        let text = built.warnings.join("\n");
        assert!(text.contains("Fantasma@Fantasma.fx"), "{text}");
        assert!(text.contains("Quebrado@Quebrado.fx"), "{text}");
        assert!(text.contains("SemArquivo sem arquivo"), "{text}");
        assert!(text.contains("Outra@Sepia.fx: o efeito não declara esta técnica"), "{text}");
        assert_eq!(built.warnings.len(), 5, "{text}");
    }

    #[test]
    fn bad_values_warn_and_keep_the_default() {
        let f = setup("plan-values");
        let preset = Preset::parse("Techniques=Sepia@Sepia.fx\n\n[Sepia.fx]\nStrength=muito\nNaoExiste=3\n");
        let built = build(&f.compiler, &preset, 8, 4);
        assert_eq!(strength(&built.plan.steps[0]), 1.0);
        assert_eq!(built.warnings, ["Sepia.fx: Strength=\"muito\" não é número"], "uniform desconhecido é ignorado sem aviso");
    }

    #[test]
    fn an_empty_preset_gives_an_empty_plan() {
        let f = setup("plan-empty");
        let built = build(&f.compiler, &Preset::parse("Techniques=\n"), 8, 4);
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
