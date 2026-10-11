//! Correções do código-fonte de efeitos que dependem de um comportamento do DirectX que o Vulkan não tem.
//!
//! O exemplo: um acesso fora dos limites a memória compartilhada (`groupshared`) é ignorado no HLSL, mas no Vulkan é
//! comportamento indefinido: numa GPU de software passa despercebido, numa NVIDIA corrompe outras variáveis do grupo. O
//! autofoco do Depth of Field do iMMERSE soma prefixos com um laço que roda uma volta a mais (`b <= AF_POINTS`) e escreve
//! em `focus_tgsm_w[2048 + n]`; o resultado era um foco sempre errado. Cada correção troca um trecho exato do texto e só vale
//! se o trecho está lá (uma versão nova do efeito, que já corrigiu o defeito, não é tocada).

use std::path::Path;

pub struct Patch {
    /// Nome do efeito (sem `.fx`).
    pub effect: &'static str,
    /// Identifica a correção (fica anotado no bundle para recompilar quando uma nova é acrescentada).
    pub id: &'static str,
    pub find: &'static str,
    pub replace: &'static str,
}

pub const PATCHES: &[Patch] = &[Patch {
    effect: "MartysMods_DEPTHOFFIELD",
    id: "af-prefix-sum-bounds",
    find: "for(uint b = 1; b <= AF_POINTS; b <<= 1, barrier())",
    replace: "for(uint b = 1; b < AF_POINTS; b <<= 1, barrier())",
}];

/// O texto com as correções aplicáveis e os identificadores das que se aplicaram.
pub fn apply(effect: &str, text: &str) -> (String, Vec<&'static str>) {
    let mut out = text.to_string();
    let mut applied = Vec::new();
    for patch in PATCHES.iter().filter(|p| p.effect == effect) {
        if out.contains(patch.find) {
            out = out.replacen(patch.find, patch.replace, 1);
            applied.push(patch.id);
        }
    }
    (out, applied)
}

/// As correções que valem para o arquivo (as que se aplicariam), sem montar o texto novo.
pub fn applicable(effect: &str, source: &Path) -> Vec<&'static str> {
    if !PATCHES.iter().any(|p| p.effect == effect) {
        return Vec::new();
    }
    let Ok(text) = std::fs::read_to_string(source) else { return Vec::new() };
    apply(effect, &text).1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_autofocus_loop_stops_one_round_early_and_other_text_is_untouched() {
        let text = "a\n    for(uint b = 1; b <= AF_POINTS; b <<= 1, barrier())\n{ }\nb";
        let (patched, ids) = apply("MartysMods_DEPTHOFFIELD", text);
        assert_eq!(ids, ["af-prefix-sum-bounds"]);
        assert!(patched.contains("b < AF_POINTS") && !patched.contains("<= AF_POINTS"));
        let (same, none) = apply("Sepia", text);
        assert_eq!((same.as_str(), none.len()), (text, 0), "outro efeito");
        let (fixed, none) = apply("MartysMods_DEPTHOFFIELD", "for(uint b = 1; b < AF_POINTS; b <<= 1, barrier())");
        assert!(none.is_empty() && fixed.contains("b < AF_POINTS"), "versão que já corrigiu");
    }
}
