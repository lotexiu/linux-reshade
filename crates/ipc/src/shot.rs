//! Capturas de tela: o daemon pede (um contador num arquivo), a layer de cada jogo grava os pixels crus numa pasta do estado
//! compartilhado (`shots/`, em memória) e o daemon converte para PNG na pasta da pessoa. Assim a layer não precisa de
//! codificador de imagem nem de acesso ao disco do usuário (o Flatpak não o dá).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const REQUEST_FILE: &str = "screenshot-request";
const MAGIC: &[u8; 4] = b"LRSH";

/// Um pedido de captura: o número cresce a cada pedido (a layer age quando vê um número novo) e diz se vale guardar também o
/// quadro sem efeitos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShotRequest {
    pub number: u64,
    pub also_without_effects: bool,
}

/// Pede uma captura a todas as layers (a de cada jogo aberto grava a sua).
pub fn request_screenshot_in(dir: &Path, also_without_effects: bool) -> io::Result<u64> {
    crate::ensure_private_dir(dir)?;
    let number = read_request_in(dir).map_or(1, |r| r.number + 1);
    fs::write(dir.join(REQUEST_FILE), format!("{number}\n{}\n", u8::from(also_without_effects)))?;
    Ok(number)
}

pub fn request_screenshot(also_without_effects: bool) -> io::Result<u64> {
    request_screenshot_in(&crate::state_dir(), also_without_effects)
}

pub fn read_request_in(dir: &Path) -> Option<ShotRequest> {
    let text = fs::read_to_string(dir.join(REQUEST_FILE)).ok()?;
    let mut lines = text.lines();
    let number = lines.next()?.trim().parse().ok()?;
    let also_without_effects = lines.next().is_some_and(|l| l.trim() == "1");
    Some(ShotRequest { number, also_without_effects })
}

pub fn read_request() -> Option<ShotRequest> {
    read_request_in(&crate::state_dir())
}

/// Onde as layers deixam os pixels crus.
pub fn shots_dir_in(dir: &Path) -> PathBuf {
    dir.join("shots")
}

/// Como os pixels estão ordenados na memória da imagem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Rgba,
    Bgra,
    /// `A2R10G10B10` (32 bits por pixel, 10 por canal): o formato de 10 bits mais comum de swapchain.
    A2R10G10B10,
    /// `A2B10G10R10`.
    A2B10G10R10,
}

impl Order {
    fn code(self) -> u8 {
        match self {
            Order::Rgba => 0,
            Order::Bgra => 1,
            Order::A2R10G10B10 => 2,
            Order::A2B10G10R10 => 3,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Order::Bgra,
            2 => Order::A2R10G10B10,
            3 => Order::A2B10G10R10,
            _ => Order::Rgba,
        }
    }

    /// Os pixels como RGBA de 8 bits (10 bits viram 8 descartando os 2 menos significativos).
    pub fn to_rgba8(self, pixels: &[u8]) -> Vec<u8> {
        let ten = |v: u32| (v >> 2) as u8;
        match self {
            Order::Rgba => pixels.to_vec(),
            Order::Bgra => pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect(),
            Order::A2R10G10B10 | Order::A2B10G10R10 => pixels
                .chunks_exact(4)
                .flat_map(|p| {
                    let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                    let (low, mid, high) = (ten(v & 0x3ff), ten((v >> 10) & 0x3ff), ten((v >> 20) & 0x3ff));
                    if self == Order::A2R10G10B10 { [high, mid, low, 255] } else { [low, mid, high, 255] }
                })
                .collect(),
        }
    }
}

/// Se o quadro tem os efeitos ou é o do jogo sem eles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    WithEffects,
    WithoutEffects,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RawShot {
    pub game: String,
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    pub order: Order,
    pub pixels: Vec<u8>,
}

/// Grava a captura crua (primeiro num arquivo temporário, depois renomeado: o daemon só vê arquivos inteiros).
pub fn write_raw_in(dir: &Path, shot: &RawShot, unique: u128) -> io::Result<PathBuf> {
    let shots = shots_dir_in(dir);
    crate::ensure_private_dir(dir)?;
    fs::create_dir_all(&shots)?;
    let kind = if shot.kind == Kind::WithEffects { "com" } else { "sem" };
    let name = format!("{}__{unique}__{kind}", shot.game);
    let tmp = shots.join(format!("{name}.tmp"));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(MAGIC)?;
    file.write_all(&shot.width.to_le_bytes())?;
    file.write_all(&shot.height.to_le_bytes())?;
    file.write_all(&[shot.order.code(), 0, 0, 0])?;
    file.write_all(&shot.pixels)?;
    drop(file);
    let done = shots.join(format!("{name}.raw"));
    fs::rename(&tmp, &done)?;
    Ok(done)
}

/// Lê uma captura crua; `None` se o arquivo não é um (ou está incompleto).
pub fn read_raw(path: &Path) -> Option<RawShot> {
    let name = path.file_name()?.to_str()?.strip_suffix(".raw")?;
    let mut parts = name.rsplitn(3, "__");
    let kind = match parts.next()? {
        "com" => Kind::WithEffects,
        "sem" => Kind::WithoutEffects,
        _ => return None,
    };
    let _unique = parts.next()?;
    let game = parts.next()?.to_string();
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 16 || &bytes[..4] != MAGIC {
        return None;
    }
    let width = u32::from_le_bytes(bytes[4..8].try_into().ok()?);
    let height = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    let order = Order::from_code(bytes[12]);
    let pixels = bytes[16..].to_vec();
    (pixels.len() as u64 == u64::from(width) * u64::from(height) * 4).then_some(RawShot { game, kind, width, height, order, pixels })
}

/// As capturas cruas esperando conversão (as mais antigas primeiro).
pub fn pending_raw_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(shots_dir_in(dir)).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "raw")).collect();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lr-shot-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn requests_count_up_and_remember_the_option() {
        let d = dir("request");
        assert_eq!(read_request_in(&d), None);
        assert_eq!(request_screenshot_in(&d, true).unwrap(), 1);
        assert_eq!(request_screenshot_in(&d, false).unwrap(), 2);
        assert_eq!(read_request_in(&d), Some(ShotRequest { number: 2, also_without_effects: false }));
    }

    #[test]
    fn ten_bit_pixels_become_eight_bit_rgba() {
        // A2R10G10B10: R=1023 (topo), G=512, B=0
        let v: u32 = (3 << 30) | (1023 << 20) | (512 << 10);
        assert_eq!(Order::A2R10G10B10.to_rgba8(&v.to_le_bytes()), [255, 128, 0, 255]);
        // A2B10G10R10: o mesmo valor lê R=0, G=512, B=1023
        assert_eq!(Order::A2B10G10R10.to_rgba8(&v.to_le_bytes()), [0, 128, 255, 255]);
    }

    #[test]
    fn a_raw_shot_roundtrips_and_a_broken_one_is_refused() {
        let d = dir("raw");
        let shot = RawShot { game: "Meu Jogo".into(), kind: Kind::WithoutEffects, width: 2, height: 1, order: Order::Bgra, pixels: vec![1, 2, 3, 4, 5, 6, 7, 8] };
        let path = write_raw_in(&d, &shot, 42).unwrap();
        assert_eq!(read_raw(&path), Some(shot));
        assert_eq!(pending_raw_in(&d), [path.clone()]);
        fs::write(&path, b"LRSH\x02\0\0\0\x01\0\0\0\0\0\0\0\x01").unwrap(); // pixels de menos
        assert_eq!(read_raw(&path), None);
    }
}
