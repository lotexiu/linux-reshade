//! Decodifica o arquivo de imagem de uma textura (`texture T < source = "lut.png"; >`) para pixels RGBA de
//! 8 bits, a forma que a layer carrega sem precisar de nenhuma biblioteca de imagem dentro do jogo.
//!
//! Só PNG por enquanto (é o formato das texturas do `reshade-shaders`); JPEG e DDS ficam para quando algum
//! efeito usado precisar. O decodificador (`png`) mora apenas neste crate, do lado do app de controle.

use std::fmt;
use std::io::Cursor;

/// Pixels RGBA, 4 bytes por pixel, linha a linha de cima para baixo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8 {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Debug)]
pub enum Error {
    /// Formato que ainda não lemos (pelo nome do arquivo ou pelo conteúdo).
    Unsupported(String),
    Decode(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported(why) => write!(f, "formato de imagem não suportado: {why}"),
            Error::Decode(why) => write!(f, "imagem inválida: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Imagens maiores que isto (por lado) são recusadas: protege a memória de arquivos malformados.
pub const MAX_SIDE: u32 = 16384;

const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Decodifica `bytes` (o conteúdo de `file_name`, que só serve para a mensagem de erro).
pub fn decode(file_name: &str, bytes: &[u8]) -> Result<Rgba8, Error> {
    if !bytes.starts_with(PNG_MAGIC) {
        let ext = file_name.rsplit_once('.').map_or("sem extensão", |(_, e)| e);
        return Err(Error::Unsupported(format!("{file_name} ({ext}); só PNG por enquanto")));
    }
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    // Paleta e tons de cinza viram RGB(A), 16 bits viram 8.
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| Error::Decode(format!("{file_name}: {e}")))?;
    let info = reader.info();
    if info.width == 0 || info.height == 0 || info.width > MAX_SIDE || info.height > MAX_SIDE {
        return Err(Error::Decode(format!("{file_name}: tamanho {}x{} fora do limite", info.width, info.height)));
    }
    let mut buffer = vec![0u8; reader.output_buffer_size().ok_or_else(|| Error::Decode(format!("{file_name}: grande demais")))?];
    let frame = reader.next_frame(&mut buffer).map_err(|e| Error::Decode(format!("{file_name}: {e}")))?;
    buffer.truncate(frame.buffer_size());

    let pixels = match frame.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buffer.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(Error::Decode(format!("{file_name}: paleta não expandida"))),
    };
    if pixels.len() != frame.width as usize * frame.height as usize * 4 {
        return Err(Error::Decode(format!("{file_name}: tamanho dos dados não confere")));
    }
    Ok(Rgba8 { width: frame.width, height: frame.height, pixels })
}

#[cfg(test)]
mod tests;
