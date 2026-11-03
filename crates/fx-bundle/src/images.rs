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

impl TextureImage {
    /// A imagem esticada para `width` x `height` (interpolação bilinear), como o ReShade faz quando o `.fx` declara para a
    /// textura de arquivo um tamanho diferente do da imagem (ex.: `Width = BUFFER_WIDTH`).
    pub fn resized(&self, width: u32, height: u32) -> TextureImage {
        if (self.width, self.height) == (width, height) || self.width == 0 || self.height == 0 {
            return self.clone();
        }
        let at = |x: u32, y: u32, c: usize| f32::from(self.rgba[(y as usize * self.width as usize + x as usize) * 4 + c]);
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            let fy = ((y as f32 + 0.5) * self.height as f32 / height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
            let (y0, ty) = (fy.floor() as u32, fy.fract());
            let y1 = (y0 + 1).min(self.height - 1);
            for x in 0..width {
                let fx = ((x as f32 + 0.5) * self.width as f32 / width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
                let (x0, tx) = (fx.floor() as u32, fx.fract());
                let x1 = (x0 + 1).min(self.width - 1);
                for c in 0..4 {
                    let top = at(x0, y0, c) * (1.0 - tx) + at(x1, y0, c) * tx;
                    let bottom = at(x0, y1, c) * (1.0 - tx) + at(x1, y1, c) * tx;
                    rgba.push((top * (1.0 - ty) + bottom * ty).round() as u8);
                }
            }
        }
        TextureImage { width, height, rgba }
    }
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
    fn an_image_is_stretched_to_the_declared_size() {
        let image = TextureImage { width: 2, height: 1, rgba: vec![0, 0, 0, 255, 255, 255, 255, 255] };
        let big = image.resized(4, 2);
        assert_eq!((big.width, big.height, big.rgba.len()), (4, 2, 32));
        assert_eq!(&big.rgba[..4], &[0, 0, 0, 255], "o canto esquerdo continua preto");
        assert_eq!(&big.rgba[12..16], &[255, 255, 255, 255], "o direito continua branco");
        assert_eq!(image.resized(2, 1), image);
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
