//! Valores dos uniforms de um efeito, no layout do buffer que os shaders leem.
//!
//! O compilador já calculou `offset` e `size` de cada uniform (layout std140 do ReShade); aqui se
//! escrevem e leem os valores nesses deslocamentos. Escalares e vetores são suportados; matrizes e
//! arrays ficam fora (não aparecem como controles nos efeitos de tela).

use crate::manifest::{Manifest, Scalar, ScalarBase, Uniform, Value};
use crate::Error;
use i18n::fmt;

/// Bloco de `total_uniform_size` bytes com os valores padrão de todos os uniforms.
pub fn default_block(manifest: &Manifest) -> Vec<u8> {
    let mut block = vec![0u8; manifest.total_uniform_size as usize];
    for u in &manifest.uniforms {
        write_default(&mut block, u);
    }
    block
}

/// Identidade dos parâmetros de um efeito, **igual em todas as resoluções**: o arquivo de origem e o
/// layout do buffer de uniforms (nome, tipo, deslocamento e tamanho de cada um).
///
/// O `Bundle::id` muda a cada resolução (o SPIR-V muda), mas os valores que o usuário ajusta valem
/// para o efeito, não para um tamanho de janela. Dois efeitos diferentes nunca compartilham a identidade
/// (o arquivo de origem entra), e uma mudança no layout dos uniforms a muda (os bytes deixariam de significar o mesmo).
pub fn params_id(manifest: &Manifest) -> u64 {
    const OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01B3;
    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            hash = (hash ^ u64::from(b)).wrapping_mul(PRIME);
        }
        hash = (hash ^ 0xFF).wrapping_mul(PRIME); // separador, para "ab"+"c" não colidir com "a"+"bc"
    };
    feed(manifest.source.as_bytes());
    feed(&manifest.total_uniform_size.to_le_bytes());
    for u in &manifest.uniforms {
        feed(u.unique_name.as_bytes());
        feed(&[u.ty.base as u8]);
        for n in [u.ty.rows, u.ty.cols, u.ty.array, u.offset, u.size] {
            feed(&n.to_le_bytes());
        }
    }
    hash
}

/// O uniform é alimentado pelo runtime (`source = "timer"` etc.), não é um controle do usuário.
pub fn is_runtime_source(u: &Uniform) -> bool {
    u.annotations.contains_key("source")
}

/// O uniform com este nome (o nome que o autor do efeito deu, não o `unique_name`).
pub fn find<'a>(manifest: &'a Manifest, name: &str) -> Option<&'a Uniform> {
    manifest.uniforms.iter().find(|u| u.name == name)
}

/// Escalares e vetores simples (1 a 4 componentes, sem array) são os que podemos ler e escrever.
fn is_plain(u: &Uniform) -> bool {
    u.ty.cols == 1 && u.ty.array == 0 && !matches!(u.ty.base, ScalarBase::String | ScalarBase::Other)
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
    if !is_plain(u) {
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

/// Converte `values` (um número por componente) nos bytes do uniform, conferindo o tipo e a faixa.
pub fn encode(u: &Uniform, values: &[f64]) -> Result<Vec<u8>, Error> {
    if !is_plain(u) {
        return Err(Error::Unsupported(fmt("uniform {}: only scalars and vectors can be changed", &[&format!("{:?}", u.name)])));
    }
    let rows = u.ty.rows as usize;
    if values.len() != rows {
        return Err(Error::Invalid(format!("uniform {:?} tem {rows} componente(s), foram dados {}", u.name, values.len())));
    }
    let mut bytes = Vec::with_capacity(rows * 4);
    for &v in values {
        if !v.is_finite() {
            return Err(Error::Invalid(fmt("uniform {}: non-finite value", &[&format!("{:?}", u.name)])));
        }
        match u.ty.base {
            ScalarBase::Float => bytes.extend_from_slice(&(v as f32).to_le_bytes()),
            ScalarBase::Int => {
                if v.fract() != 0.0 || v < f64::from(i32::MIN) || v > f64::from(i32::MAX) {
                    return Err(Error::Invalid(fmt("uniform {}: {} is not a 32-bit int", &[&format!("{:?}", u.name), &v])));
                }
                bytes.extend_from_slice(&(v as i32).to_le_bytes());
            }
            ScalarBase::Uint => {
                if v.fract() != 0.0 || v < 0.0 || v > f64::from(u32::MAX) {
                    return Err(Error::Invalid(fmt("uniform {}: {} is not a 32-bit uint", &[&format!("{:?}", u.name), &v])));
                }
                bytes.extend_from_slice(&(v as u32).to_le_bytes());
            }
            ScalarBase::Bool => {
                if v != 0.0 && v != 1.0 {
                    return Err(Error::Invalid(fmt("uniform {}: bool must be 0 or 1", &[&format!("{:?}", u.name)])));
                }
                bytes.extend_from_slice(&(v as u32).to_le_bytes());
            }
            ScalarBase::String | ScalarBase::Other => unreachable!("is_plain exclui estes"),
        }
    }
    Ok(bytes)
}

/// Lê de volta o valor do uniform em `block`; `None` se o bloco é curto demais ou o tipo não é simples.
pub fn decode(u: &Uniform, block: &[u8]) -> Option<Vec<f64>> {
    if !is_plain(u) {
        return None;
    }
    (0..u.ty.rows as usize)
        .map(|i| {
            let at = u.offset as usize + i * 4;
            let word: [u8; 4] = block.get(at..at + 4)?.try_into().ok()?;
            Some(match u.ty.base {
                ScalarBase::Float => f64::from(f32::from_le_bytes(word)),
                ScalarBase::Int => f64::from(i32::from_le_bytes(word)),
                _ => f64::from(u32::from_le_bytes(word)),
            })
        })
        .collect()
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
            r#"{{"name":"u{offset}","unique_name":"u{offset}","type":{{"base":"{base}","rows":{rows},"cols":1,"array":0}},
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

    #[test]
    fn encode_then_decode_roundtrips_each_type() {
        let m = manifest(
            48,
            &[
                uniform("float", 1, 0, 4, "null"),
                uniform("int", 1, 4, 4, "null"),
                uniform("uint", 1, 8, 4, "null"),
                uniform("bool", 1, 12, 4, "null"),
                uniform("float", 3, 16, 12, "null"),
            ]
            .join(","),
        );
        let mut block = vec![0u8; 48];
        for (u, values) in m.uniforms.iter().zip([vec![0.25], vec![-7.0], vec![4_000_000_000.0], vec![1.0], vec![0.1, 0.2, 0.3]]) {
            let bytes = encode(u, &values).unwrap();
            block[u.offset as usize..u.offset as usize + bytes.len()].copy_from_slice(&bytes);
            let back = decode(u, &block).unwrap();
            for (a, b) in back.iter().zip(&values) {
                assert!((a - b).abs() < 1e-6, "{} voltou {a}, esperado {b}", u.name);
            }
        }
    }

    #[test]
    fn encode_rejects_wrong_counts_and_out_of_range_values() {
        let m = manifest(
            32,
            &[
                uniform("float", 3, 0, 12, "null"),
                uniform("int", 1, 16, 4, "null"),
                uniform("uint", 1, 20, 4, "null"),
                uniform("bool", 1, 24, 4, "null"),
            ]
            .join(","),
        );
        let [vec3, int, uint, boolean] = [&m.uniforms[0], &m.uniforms[1], &m.uniforms[2], &m.uniforms[3]];
        assert!(encode(vec3, &[1.0, 2.0]).is_err(), "componentes a menos");
        assert!(encode(vec3, &[1.0, 2.0, f64::NAN]).is_err(), "NaN");
        assert!(encode(int, &[1.5]).is_err(), "int fracionário");
        assert!(encode(int, &[3e10]).is_err(), "int estoura");
        assert!(encode(uint, &[-1.0]).is_err(), "uint negativo");
        assert!(encode(boolean, &[2.0]).is_err(), "bool 2");
        assert!(encode(boolean, &[1.0]).is_ok());
    }

    #[test]
    fn params_id_ignores_the_resolution_but_not_the_layout_or_the_effect() {
        let layout = |source: &str, offset: u32, total: u32| {
            let mut m = manifest(total, &uniform("float", 1, offset, 4, "null"));
            m.source = source.into();
            m
        };
        let base = layout("Sepia.fx", 0, 16);
        let mut other_size = base.clone();
        other_size.target.width = 1234; // outra resolução: o mesmo efeito, os mesmos controles
        other_size.compiler.reshade_commit = "outro".into();
        assert_eq!(params_id(&base), params_id(&other_size), "a resolução não muda a identidade");

        assert_ne!(params_id(&base), params_id(&layout("Invert.fx", 0, 16)), "outro efeito");
        assert_ne!(params_id(&base), params_id(&layout("Sepia.fx", 4, 16)), "outro deslocamento");
        assert_ne!(params_id(&base), params_id(&layout("Sepia.fx", 0, 32)), "outro tamanho de buffer");
    }

    #[test]
    fn decode_of_a_short_block_is_none() {
        let m = manifest(8, &uniform("float", 1, 4, 4, "null"));
        assert!(decode(&m.uniforms[0], &[0u8; 6]).is_none());
    }
}
