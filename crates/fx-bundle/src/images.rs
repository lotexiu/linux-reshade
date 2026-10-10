//! Imagens das texturas que vêm de arquivo (`texture T < source = "lut.png"; >`).
//!
//! O app de controle decodifica o arquivo e grava os pixels crus no bundle; a layer só os lê. Cada textura com
//! `source` tem o seu arquivo `images/<posição da textura no manifesto>.rgba`:
//!
//! ```text
//! "LRIM"  u32 largura  u32 altura  largura*altura*4 bytes RGBA   (little endian, de cima para baixo)
//! ```

use crate::manifest::{Manifest, Scalar, Texture, Value};
use crate::Error;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"LRIM";
/// Limite de uma imagem: 128 MiB de pixels (por exemplo 5000x6500), para um bundle corrompido não esgotar a memória.
pub const MAX_IMAGE_BYTES: u64 = 128 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// O arquivo de origem de uma textura (a anotação `source`), se tem.
pub fn source_of(texture: &Texture) -> Option<&str> {
    match texture.annotations.get("source") {
        Some(Value::Scalar(Scalar::Text(path))) if !path.is_empty() => Some(path),
        _ => None,
    }
}

/// Onde fica a imagem da textura `index` dentro do bundle.
pub fn path(bundle_dir: &Path, index: usize) -> PathBuf {
    bundle_dir.join("images").join(format!("{index}.rgba"))
}

pub fn encode(image: &TextureImage) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + image.rgba.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&image.width.to_le_bytes());
    out.extend_from_slice(&image.height.to_le_bytes());
    out.extend_from_slice(&image.rgba);
    out
}

pub fn decode(bytes: &[u8]) -> Result<TextureImage, Error> {
    let invalid = |why: &str| Error::Invalid(format!("imagem de textura: {why}"));
    let header = bytes.get(..12).filter(|h| &h[..4] == MAGIC).ok_or_else(|| invalid("cabeçalho ausente"))?;
    let width = u32::from_le_bytes(header[4..8].try_into().expect("4 bytes"));
    let height = u32::from_le_bytes(header[8..12].try_into().expect("4 bytes"));
    let expected = u64::from(width) * u64::from(height) * 4;
    if width == 0 || height == 0 || expected != (bytes.len() - 12) as u64 {
        return Err(invalid(&format!("{width}x{height} não bate com {} bytes de pixels", bytes.len() - 12)));
    }
    Ok(TextureImage { width, height, rgba: bytes[12..].to_vec() })
}

/// Grava a imagem da textura `index` no bundle (cria `images/`).
pub fn write(bundle_dir: &Path, index: usize, image: &TextureImage) -> std::io::Result<()> {
    std::fs::create_dir_all(bundle_dir.join("images"))?;
    std::fs::write(path(bundle_dir, index), encode(image))
}

/// Lê as imagens que o manifesto pede e o bundle tem: `(posição da textura, imagem)`. Uma textura com `source`
/// sem arquivo é omitida (quem usa decide se isso é um erro). Confere que o tamanho bate com o declarado.
pub fn load_all(dir: &Path, manifest: &Manifest) -> Result<Vec<(usize, TextureImage)>, Error> {
    let mut out = Vec::new();
    for (index, texture) in manifest.textures.iter().enumerate() {
        if source_of(texture).is_none() {
            continue;
        }
        let file = path(dir, index);
        let len = match std::fs::metadata(&file) {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(Error::Io { path: file, source }),
        };
        if len > MAX_IMAGE_BYTES {
            return Err(Error::Invalid(format!("{} tem {len} bytes (máximo {MAX_IMAGE_BYTES})", file.display())));
        }
        let bytes = std::fs::read(&file).map_err(|source| Error::Io { path: file.clone(), source })?;
        let image = decode(&bytes).map_err(|e| Error::Invalid(format!("{}: {e}", file.display())))?;
        if (image.width, image.height) != (texture.width, texture.height) {
            return Err(Error::Invalid(format!(
                "textura {:?}: o manifesto declara {}x{} e a imagem tem {}x{}",
                texture.name, texture.width, texture.height, image.width, image.height
            )));
        }
        out.push((index, image));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_round_trips() {
        let image = TextureImage { width: 2, height: 1, rgba: vec![1, 2, 3, 4, 5, 6, 7, 8] };
        assert_eq!(decode(&encode(&image)).unwrap(), image);
    }

    #[test]
    fn bad_headers_and_sizes_are_refused() {
        let good = encode(&TextureImage { width: 2, height: 2, rgba: vec![0; 16] });
        for cut in 0..good.len() {
            assert!(decode(&good[..cut]).is_err(), "cortado em {cut}");
        }
        let mut wrong_magic = good.clone();
        wrong_magic[0] = b'X';
        assert!(decode(&wrong_magic).is_err());
        let mut longer = good.clone();
        longer.push(0);
        assert!(decode(&longer).is_err());
        let zero = encode(&TextureImage { width: 0, height: 5, rgba: vec![] });
        assert!(decode(&zero).is_err());
    }
}
