//! Valores iniciais do buffer de uniforms, a partir dos `default` do manifesto.
//!
//! O compilador já calculou `offset` e `size` de cada uniform (layout std140 do ReShade); aqui só
//! se escrevem os valores nesses deslocamentos. Escalares e vetores são suportados; matrizes e
//! arrays ficam em zero (não aparecem como controles de interface nos efeitos de tela).

use fx_bundle::manifest::{Manifest, Scalar, ScalarBase, Uniform, Value};

/// Bloco de `total_uniform_size` bytes com os valores padrão de todos os uniforms.
pub fn default_block(manifest: &Manifest) -> Vec<u8> {
    let mut block = vec![0u8; manifest.total_uniform_size as usize];
    for u in &manifest.uniforms {
        write_default(&mut block, u);
    }
    block
}

fn scalar_bits(base: ScalarBase, s: &Scalar) -> Option<[u8; 4]> {
    Some(match (base, s) {
        (ScalarBase::Float, Scalar::Number(n)) => (*n as f32).to_le_bytes(),
        (ScalarBase::Int, Scalar::Number(n)) => (*n as i32).to_le_bytes(),
        (ScalarBase::Uint, Scalar::Number(n)) => (*n as u32).to_le_bytes(),
        // bool num buffer std140 ocupa 32 bits.
        (ScalarBase::Bool, Scalar::Bool(b)) => u32::from(*b).to_le_bytes(),
        (ScalarBase::Bool, Scalar::Number(n)) => u32::from(*n != 0.0).to_le_bytes(),
        _ => return None,
    })
}

fn write_default(block: &mut [u8], u: &Uniform) {
    let Some(value) = &u.default else { return };
    if u.ty.cols != 1 || u.ty.array != 0 {
        return;
    }
    let components: Vec<&Scalar> = match value {
        Value::Scalar(s) => vec![s],
        Value::List(list) => list.iter().collect(),
    };
    for (i, s) in components.iter().take(u.ty.rows as usize).enumerate() {
        let Some(bits) = scalar_bits(u.ty.base, s) else { return };
        let at = u.offset as usize + i * 4;
        // Nunca escreve além do que o compilador reservou para este uniform nem do bloco.
        if i * 4 + 4 <= u.size as usize && at + 4 <= block.len() {
            block[at..at + 4].copy_from_slice(&bits);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manifesto mínimo com os uniforms dados, no formato JSON do bundle.
    fn manifest(total: u32, uniforms: &str) -> Manifest {
        let json = format!(
            r#"{{"format":"linux-reshade-bundle","version":1,"source":"t.fx",
            "compiler":{{"name":"x","reshade_commit":"x","spirv_headers_commit":"x"}},
            "target":{{"width":8,"height":8,"color_space":"srgb"}},"spirv_version":"1.0",
            "entry_points":[],"total_uniform_size":{total},"uniforms":[{uniforms}],"techniques":[]}}"#
        );
        Manifest::parse(&json).expect("manifesto de teste")
    }

    fn uniform(base: &str, rows: u32, offset: u32, size: u32, default: &str) -> String {
        format!(
            r#"{{"name":"u","unique_name":"u{offset}","type":{{"base":"{base}","rows":{rows},"cols":1,"array":0}},
            "offset":{offset},"size":{size},"default":{default},"annotations":{{}}}}"#
        )
    }

    #[test]
    fn writes_scalars_and_vectors_at_their_offsets() {
        let m = manifest(
            32,
            &[
                uniform("float", 1, 0, 4, "0.5"),
                uniform("int", 1, 4, 4, "-3"),
                uniform("uint", 1, 8, 4, "7"),
                uniform("bool", 1, 12, 4, "true"),
                uniform("float", 3, 16, 12, "[1.0, 2.0, 3.0]"),
            ]
            .join(","),
        );
        let b = default_block(&m);
        assert_eq!(b.len(), 32);
        assert_eq!(f32::from_le_bytes(b[0..4].try_into().unwrap()), 0.5);
        assert_eq!(i32::from_le_bytes(b[4..8].try_into().unwrap()), -3);
        assert_eq!(u32::from_le_bytes(b[8..12].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(b[12..16].try_into().unwrap()), 1);
        let v: Vec<f32> = (0..3).map(|i| f32::from_le_bytes(b[16 + i * 4..20 + i * 4].try_into().unwrap())).collect();
        assert_eq!(v, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn uniform_without_default_stays_zero() {
        let m = manifest(8, &uniform("float", 1, 0, 4, "null"));
        assert_eq!(default_block(&m), vec![0u8; 8]);
    }

    #[test]
    fn never_writes_outside_the_reserved_range() {
        // Vetor de 4 componentes declarado com size 8: só os 2 primeiros cabem.
        let m = manifest(16, &uniform("float", 4, 0, 8, "[1.0, 2.0, 3.0, 4.0]"));
        let b = default_block(&m);
        assert_eq!(f32::from_le_bytes(b[4..8].try_into().unwrap()), 2.0);
        assert_eq!(&b[8..16], &[0u8; 8]);
        // Offset além do bloco também não pode estourar.
        let m = manifest(4, &uniform("float", 1, 8, 4, "1.0"));
        assert_eq!(default_block(&m), vec![0u8; 4]);
    }

    #[test]
    fn mismatched_default_type_is_ignored() {
        let m = manifest(4, &uniform("float", 1, 0, 4, r#""texto""#));
        assert_eq!(default_block(&m), vec![0u8; 4]);
    }
}
