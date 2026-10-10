//! Bloco de bytes publicado por um escritor e lido sem bloqueio por leitores (um *seqlock*). Serve
//! aos dois canais do app de controle para a layer:
//!
//! - [`Params`]: a imagem do buffer de uniforms de um efeito, copiada pela layer a cada frame;
//! - [`Selection`]: o caminho do bundle que a layer deve carregar.
//!
//! É um *seqlock*: o contador `seq` é ímpar enquanto alguém escreve e par quando o conteúdo é
//! consistente. O leitor só aceita uma cópia se `seq` era par e não mudou durante a leitura, então
//! nunca vê dados "rasgados", e nunca bloqueia nem faz syscall. Os dados ficam em palavras atômicas
//! (acessos `Relaxed`) para que a leitura concorrente não seja uma corrida de dados.
//!
//! Cada bloco leva uma etiqueta (`tag`): nos parâmetros, o `bundle_id` do efeito a que pertencem (a
//! layer só aplica um bloco do efeito que ela mesma carregou, então valores de outro efeito nunca
//! vazam para este); na seleção, uma constante.

use std::hint::spin_loop;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};
use std::time::{Duration, Instant};

/// Tamanho máximo dos parâmetros: o mínimo que o Vulkan garante para um buffer de uniforms.
pub const MAX_PARAM_BYTES: usize = 16384;
/// Tamanho máximo do caminho do bundle selecionado.
pub const MAX_SELECTION_BYTES: usize = 1024;

/// Um escritor que não termina em tanto tempo é considerado morto no meio da escrita.
const STUCK_WRITER: Duration = Duration::from_millis(50);
const READ_ATTEMPTS: usize = 16;

#[repr(C)]
pub(crate) struct Blob<const WORDS: usize> {
    seq: AtomicU32,
    len: AtomicU32,
    tag: AtomicU64,
    words: [AtomicU32; WORDS],
}

pub(crate) type Params = Blob<{ MAX_PARAM_BYTES / 4 }>;
pub(crate) type Selection = Blob<{ MAX_SELECTION_BYTES / 4 }>;
pub(crate) type PlanBlob = Blob<{ crate::plan::MAX_PLAN_BYTES / 4 }>;

/// Resultado de uma tentativa de leitura.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Snapshot {
    /// Cópia consistente de `len` bytes (pode ser 0: bloco vazio ou nunca publicado).
    Consistent { len: usize },
    /// O bloco publicado tem outra etiqueta; nada foi copiado.
    Mismatch,
    /// A leitura caiu no meio de uma escrita; tente de novo depois.
    Torn,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PublishError {
    /// Os dados não cabem no bloco.
    TooLarge { len: usize, max: usize },
}

impl std::fmt::Display for PublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PublishError::TooLarge { len, max } => write!(f, "{len} bytes (máximo {max})"),
        }
    }
}

impl std::error::Error for PublishError {}

impl<const WORDS: usize> Blob<WORDS> {
    const MAX_BYTES: usize = WORDS * 4;

    /// Publica `data` com a etiqueta `tag`. Vários escritores se revezam.
    pub(crate) fn publish(&self, tag: u64, data: &[u8]) -> Result<(), PublishError> {
        if data.len() > Self::MAX_BYTES {
            return Err(PublishError::TooLarge { len: data.len(), max: Self::MAX_BYTES });
        }
        let started = Instant::now();
        let before = loop {
            let s = self.seq.load(Ordering::Relaxed);
            if s % 2 == 0 {
                if self.seq.compare_exchange(s, s.wrapping_add(1), Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    break s;
                }
            } else if started.elapsed() > STUCK_WRITER {
                // Um escritor morreu no meio: toma o lugar dele (o conteúdo é reescrito por inteiro de qualquer forma).
                if self.seq.compare_exchange(s, s.wrapping_add(1), Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    break s.wrapping_sub(1);
                }
            }
            spin_loop();
        };
        fence(Ordering::Release); // o contador ímpar precisa ser visível antes dos dados
        self.len.store(data.len() as u32, Ordering::Relaxed);
        self.tag.store(tag, Ordering::Relaxed);
        for (i, chunk) in data.chunks(4).enumerate() {
            let mut word = [0u8; 4];
            word[..chunk.len()].copy_from_slice(chunk);
            self.words[i].store(u32::from_le_bytes(word), Ordering::Relaxed);
        }
        self.seq.store(before.wrapping_add(2), Ordering::Release);
        Ok(())
    }

    /// Tenta copiar para `out` o bloco publicado com a etiqueta `tag` (no máximo `out.len()` bytes).
    pub(crate) fn snapshot(&self, tag: u64, out: &mut [u8]) -> Snapshot {
        for _ in 0..READ_ATTEMPTS {
            let s1 = self.seq.load(Ordering::Acquire);
            if s1 % 2 == 1 {
                spin_loop();
                continue;
            }
            if self.tag.load(Ordering::Relaxed) != tag {
                return Snapshot::Mismatch;
            }
            let n = (self.len.load(Ordering::Relaxed) as usize).min(Self::MAX_BYTES).min(out.len());
            for (i, chunk) in out[..n].chunks_mut(4).enumerate() {
                let word = self.words[i].load(Ordering::Relaxed).to_le_bytes();
                chunk.copy_from_slice(&word[..chunk.len()]);
            }
            fence(Ordering::Acquire);
            if self.seq.load(Ordering::Relaxed) == s1 {
                return Snapshot::Consistent { len: n };
            }
        }
        Snapshot::Torn
    }

    /// Copia para `out` o bloco com a etiqueta `tag`. `None` se não há bloco dessa etiqueta, se está
    /// vazio ou se a leitura não conseguiu uma cópia consistente (o chamador tenta no próximo frame).
    /// Devolve quantos bytes foram copiados (no máximo `out.len()`).
    pub(crate) fn read(&self, tag: u64, out: &mut [u8]) -> Option<usize> {
        match self.snapshot(tag, out) {
            Snapshot::Consistent { len } if len > 0 => Some(len),
            _ => None,
        }
    }

    /// Número da publicação atual: muda a cada `publish`. Uma leitura barata para saber se algo mudou.
    pub(crate) fn generation(&self) -> u32 {
        self.seq.load(Ordering::Acquire) / 2
    }

    /// Etiqueta do bloco publicado (0 se nunca foi publicado).
    pub(crate) fn published_tag(&self) -> u64 {
        self.tag.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn force_seq(&self, value: u32) {
        self.seq.store(value, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    fn params() -> Arc<Params> {
        // SAFETY: atomics com todos os bits em zero são valores válidos.
        Arc::from(unsafe { Box::<Params>::new_zeroed().assume_init() })
    }

    #[test]
    fn a_published_block_is_read_back_exactly() {
        let p = params();
        p.publish(7, &[1, 2, 3, 4, 5, 6, 7]).unwrap();
        let mut out = [0u8; 16];
        assert_eq!(p.read(7, &mut out), Some(7));
        assert_eq!(&out[..7], &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(&out[7..], &[0u8; 9], "não escreve além do que foi publicado");
    }

    #[test]
    fn another_effects_block_is_never_returned() {
        let p = params();
        p.publish(7, &[9; 8]).unwrap();
        let mut out = [0u8; 8];
        assert_eq!(p.read(8, &mut out), None);
        assert_eq!(out, [0u8; 8], "nada é copiado");
        assert_eq!(p.published_tag(), 7);
    }

    #[test]
    fn nothing_published_reads_as_none() {
        let p = params();
        assert_eq!(p.read(0, &mut [0u8; 4]), None, "bloco vazio");
        assert_eq!(p.read(5, &mut [0u8; 4]), None);
    }

    #[test]
    fn a_shorter_destination_gets_a_prefix() {
        let p = params();
        p.publish(1, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let mut out = [0u8; 5];
        assert_eq!(p.read(1, &mut out), Some(5));
        assert_eq!(out, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn oversized_blocks_are_refused_and_leave_the_old_one() {
        let p = params();
        p.publish(1, &[1; 8]).unwrap();
        assert_eq!(p.publish(1, &vec![0u8; MAX_PARAM_BYTES + 1]), Err(PublishError::TooLarge { len: MAX_PARAM_BYTES + 1, max: MAX_PARAM_BYTES }));
        let mut out = [0u8; 8];
        assert_eq!(p.read(1, &mut out), Some(8));
        assert_eq!(out, [1; 8]);
        assert!(p.publish(1, &vec![0u8; MAX_PARAM_BYTES]).is_ok(), "o limite exato cabe");
    }

    #[test]
    fn a_writer_that_died_mid_write_does_not_block_the_next_one() {
        let p = params();
        p.publish(1, &[1; 4]).unwrap();
        p.force_seq(3); // contador ímpar: simula um escritor que morreu escrevendo
        // O leitor não vê um bloco consistente enquanto o contador está ímpar...
        assert_eq!(p.read(1, &mut [0u8; 4]), None);
        // ...e o próximo escritor assume depois do prazo e deixa tudo consistente de novo.
        let started = Instant::now();
        p.publish(1, &[2; 4]).unwrap();
        assert!(started.elapsed() >= STUCK_WRITER, "esperou o prazo antes de assumir");
        let mut out = [0u8; 4];
        assert_eq!(p.read(1, &mut out), Some(4));
        assert_eq!(out, [2; 4]);
    }

    /// O leitor nunca pode ver um bloco misturado de duas publicações. Cada bloco tem todos os bytes
    /// iguais; qualquer leitura bem-sucedida tem de ter todos iguais, com escrita e leitura concorrentes.
    ///
    /// O escritor descansa entre as publicações: um seqlock deixa o leitor sem vez se o escritor nunca
    /// para (as leituras são recusadas, nunca erradas). Na prática um humano mexendo num slider escreve
    /// centenas de vezes por segundo, no máximo, e o leitor só tenta de novo no frame seguinte.
    #[test]
    fn readers_never_see_a_torn_block() {
        let p = params();
        let writer = {
            let p = p.clone();
            thread::spawn(move || {
                let end = Instant::now() + Duration::from_millis(300);
                let mut i = 0u32;
                while Instant::now() < end {
                    p.publish(42, &[(i % 251) as u8; 1024]).unwrap();
                    i += 1;
                    thread::sleep(Duration::from_micros(50));
                }
                i
            })
        };
        let readers: Vec<_> = (0..3)
            .map(|_| {
                let p = p.clone();
                thread::spawn(move || {
                    let end = Instant::now() + Duration::from_millis(300);
                    let (mut ok, mut torn) = (0u32, 0u32);
                    let mut out = vec![0u8; 1024];
                    while Instant::now() < end {
                        if let Some(n) = p.read(42, &mut out) {
                            ok += 1;
                            if out[..n].iter().any(|&b| b != out[0]) {
                                torn += 1;
                            }
                        }
                    }
                    (ok, torn)
                })
            })
            .collect();
        let writes = writer.join().unwrap();
        for r in readers {
            let (ok, torn) = r.join().unwrap();
            assert_eq!(torn, 0, "leitura rasgada");
            assert!(ok > 100, "o leitor quase não conseguiu ler ({ok}) durante {writes} escritas");
        }
    }

    #[test]
    fn two_writers_take_turns_without_corrupting_the_block() {
        let p = params();
        let spawn = |value: u8| {
            let p = p.clone();
            thread::spawn(move || {
                for _ in 0..2000 {
                    p.publish(1, &[value; 256]).unwrap();
                }
            })
        };
        let (a, b) = (spawn(0xAA), spawn(0x55));
        a.join().unwrap();
        b.join().unwrap();
        let mut out = [0u8; 256];
        assert_eq!(p.read(1, &mut out), Some(256));
        assert!(out.iter().all(|&x| x == out[0]) && (out[0] == 0xAA || out[0] == 0x55), "o bloco final é de um dos escritores, inteiro");
    }

    #[test]
    fn generation_changes_on_every_publication_and_only_then() {
        let p = params();
        let g0 = p.generation();
        let mut out = [0u8; 4];
        p.read(1, &mut out);
        assert_eq!(p.generation(), g0, "ler não muda a geração");
        p.publish(1, &[1; 4]).unwrap();
        let g1 = p.generation();
        assert_ne!(g0, g1);
        p.publish(1, &[1; 4]).unwrap();
        assert_ne!(g1, p.generation(), "mesmo conteúdo publicado de novo é outra publicação");
    }

    #[test]
    fn snapshot_tells_empty_apart_from_mismatched() {
        let p = params();
        let mut out = [0u8; 8];
        assert_eq!(p.snapshot(0, &mut out), Snapshot::Consistent { len: 0 }, "nunca publicado");
        p.publish(4, &[]).unwrap();
        assert_eq!(p.snapshot(4, &mut out), Snapshot::Consistent { len: 0 }, "publicado vazio (limpo)");
        assert_eq!(p.snapshot(5, &mut out), Snapshot::Mismatch);
        p.force_seq(1);
        assert_eq!(p.snapshot(4, &mut out), Snapshot::Torn);
    }

    #[test]
    fn a_selection_blob_is_smaller_and_enforces_its_own_limit() {
        // SAFETY: atomics com todos os bits em zero são valores válidos.
        let s: Box<Selection> = unsafe { Box::<Selection>::new_zeroed().assume_init() };
        assert!(s.publish(1, &vec![b'x'; MAX_SELECTION_BYTES]).is_ok());
        assert_eq!(
            s.publish(1, &vec![b'x'; MAX_SELECTION_BYTES + 1]),
            Err(PublishError::TooLarge { len: MAX_SELECTION_BYTES + 1, max: MAX_SELECTION_BYTES })
        );
    }
}
