//! Leitura do bundle em disco.

use crate::{Error, Manifest};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Limites para um bundle corrompido ou malicioso não consumir memória à toa.
const MAX_MANIFEST_BYTES: u64 = 4 << 20;
const MAX_SPIRV_BYTES: u64 = 8 << 20;
const SPIRV_MAGIC: u32 = 0x0723_0203;

/// Um bundle carregado e validado: manifesto e o SPIR-V (em palavras de 32 bits) de cada entry point.
#[derive(Debug)]
pub struct Bundle {
    pub manifest: Manifest,
    /// Identifica este bundle entre processos (a layer e o app de controle): hash do `manifest.json`.
    /// Muda quando o efeito é recompilado, por exemplo para outra resolução.
    pub id: u64,
    shaders: HashMap<String, Vec<u32>>,
}

fn io(path: &Path, source: std::io::Error) -> Error {
    Error::Io { path: path.to_path_buf(), source }
}

fn read_limited(path: &Path, max: u64) -> Result<Vec<u8>, Error> {
    let len = fs::metadata(path).map_err(|e| io(path, e))?.len();
    if len > max {
        return Err(Error::Invalid(format!("{} tem {len} bytes (máximo {max})", path.display())));
    }
    fs::read(path).map_err(|e| io(path, e))
}

/// FNV-1a de 64 bits: simples, estável entre versões e sem dependências. Não é criptográfico; só
/// precisa distinguir bundles diferentes.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3))
}

/// Converte bytes de um `.spv` (little endian) em palavras, conferindo o tamanho e o magic number.
pub fn spirv_words(bytes: &[u8]) -> Result<Vec<u32>, Error> {
    if bytes.len() < 20 || bytes.len() % 4 != 0 {
        return Err(Error::Invalid(format!("SPIR-V com {} bytes", bytes.len())));
    }
    let words: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if words[0] != SPIRV_MAGIC {
        return Err(Error::Invalid("SPIR-V sem o magic number".into()));
    }
    Ok(words)
}

impl Bundle {
    /// Carrega e valida o bundle em `dir`.
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let manifest_path: PathBuf = dir.join("manifest.json");
        let bytes = read_limited(&manifest_path, MAX_MANIFEST_BYTES)?;
        let id = fnv1a64(&bytes);
        let text = String::from_utf8(bytes).map_err(|_| Error::Parse("não é UTF-8".into()))?;
        let manifest = Manifest::parse(&text)?;
        manifest.validate()?;

        let mut shaders = HashMap::new();
        for entry in &manifest.entry_points {
            // `validate` já garantiu que `file` é um nome simples, sem diretórios.
            let path = dir.join(&entry.file);
            let words = spirv_words(&read_limited(&path, MAX_SPIRV_BYTES)?)
                .map_err(|e| Error::Invalid(format!("{}: {e}", entry.file)))?;
            shaders.insert(entry.name.clone(), words);
        }
        Ok(Self { manifest, id, shaders })
    }

    /// SPIR-V do entry point `name` (um nome presente em `manifest.entry_points`).
    pub fn shader(&self, name: &str) -> Option<&[u32]> {
        self.shaders.get(name).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::fnv1a64;

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        // Vetores de teste publicados do FNV-1a de 64 bits.
        assert_eq!(fnv1a64(b""), 0xCBF2_9CE4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xAF63_DC4C_8601_EC8C);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_F739_67E8);
    }
}
