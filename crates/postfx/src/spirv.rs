use ash::vk;

const MAGIC: u32 = 0x0723_0203;

/// Converte bytes de um `.spv` (little endian) em palavras de 32 bits, conferindo o magic number.
pub fn words(bytes: &[u8]) -> Result<Vec<u32>, vk::Result> {
    if bytes.len() % 4 != 0 || bytes.len() < 4 {
        return Err(vk::Result::ERROR_INITIALIZATION_FAILED);
    }
    let words: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if words[0] != MAGIC {
        return Err(vk::Result::ERROR_INITIALIZATION_FAILED);
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_embedded_shaders() {
        for bytes in [
            &include_bytes!("../shaders/fullscreen.vert.spv")[..],
            &include_bytes!("../shaders/invert.frag.spv")[..],
        ] {
            assert!(words(bytes).is_ok());
        }
    }

    #[test]
    fn rejects_garbage() {
        assert!(words(&[1, 2, 3]).is_err());
        assert!(words(&[0, 0, 0, 0]).is_err());
        assert!(words(&[]).is_err());
    }
}
