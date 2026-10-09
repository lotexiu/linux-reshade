//! Parâmetros publicados pelo app de controle: o que escrever no buffer de uniforms a cada frame.

use crate::log::log;

/// Estado por efeito criado: o buffer de cópia e se já há parâmetros aplicados.
pub struct ParamState {
    scratch: Vec<u8>,
    defaults: Vec<u8>,
    applied: bool,
}

impl ParamState {
    pub fn new(defaults: Vec<u8>) -> Self {
        Self { scratch: vec![0; defaults.len()], defaults, applied: false }
    }

    /// Os bytes a escrever no buffer de uniforms (a partir do offset 0) neste frame, ou `None` se
    /// não há nada a mudar. Chame a cada frame: a leitura não bloqueia nem faz syscall.
    ///
    /// - Há um bloco deste efeito: devolve uma cópia dele.
    /// - O bloco deixou de ser deste efeito (o usuário trocou de efeito, ou restaurou): volta aos
    ///   padrões, uma única vez.
    /// - A leitura caiu no meio de uma escrita: mantém o que há e tenta de novo no próximo frame.
    pub fn next(&mut self, reader: &ipc::Reader, bundle_id: u64) -> Option<&[u8]> {
        if self.scratch.is_empty() {
            return None;
        }
        if let Some(n) = reader.read_params(bundle_id, &mut self.scratch) {
            if !self.applied {
                log!("parâmetros do app de controle aplicados ({n} bytes)");
                self.applied = true;
            }
            return Some(&self.scratch[..n]);
        }
        if self.applied && reader.published_bundle() != bundle_id {
            log!("parâmetros do app de controle removidos; voltando aos padrões do efeito");
            self.applied = false;
            return Some(&self.defaults);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(name: &str) -> (ipc::Writer, ipc::Reader) {
        let path = std::env::temp_dir().join(format!("lr-layer-params-{}-{name}", std::process::id())).join("state");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let writer = ipc::Writer::create_at(&path).unwrap();
        let reader = ipc::Reader::open_at(&path).unwrap().unwrap();
        (writer, reader)
    }

    #[test]
    fn nothing_published_changes_nothing() {
        let (_w, r) = channel("none");
        let mut p = ParamState::new(vec![1, 2, 3, 4]);
        assert_eq!(p.next(&r, 5), None);
    }

    #[test]
    fn a_published_block_is_applied_every_frame() {
        let (w, r) = channel("apply");
        let mut p = ParamState::new(vec![0; 4]);
        w.publish_params(5, &[9, 8, 7, 6]).unwrap();
        assert_eq!(p.next(&r, 5), Some(&[9u8, 8, 7, 6][..]));
        assert_eq!(p.next(&r, 5), Some(&[9u8, 8, 7, 6][..]), "continua a valer nos frames seguintes");
        w.publish_params(5, &[1, 1, 1, 1]).unwrap();
        assert_eq!(p.next(&r, 5), Some(&[1u8, 1, 1, 1][..]), "mudanças chegam no frame seguinte");
    }

    #[test]
    fn a_block_for_another_effect_is_ignored() {
        let (w, r) = channel("other");
        let mut p = ParamState::new(vec![0; 4]);
        w.publish_params(99, &[9; 4]).unwrap();
        assert_eq!(p.next(&r, 5), None, "nunca aplica o bloco de outro efeito");
    }

    #[test]
    fn switching_to_another_effect_restores_the_defaults_once() {
        let (w, r) = channel("switch");
        let defaults = vec![5, 5, 5, 5];
        let mut p = ParamState::new(defaults.clone());
        w.publish_params(5, &[9; 4]).unwrap();
        assert!(p.next(&r, 5).is_some());

        w.publish_params(99, &[1; 4]).unwrap(); // o usuário passou a controlar outro efeito
        assert_eq!(p.next(&r, 5), Some(&defaults[..]), "volta aos padrões");
        assert_eq!(p.next(&r, 5), None, "só uma vez");
    }

    #[test]
    fn effects_without_uniforms_are_left_alone() {
        let (w, r) = channel("empty");
        let mut p = ParamState::new(Vec::new());
        w.publish_params(5, &[1, 2, 3, 4]).unwrap();
        assert_eq!(p.next(&r, 5), None);
    }
}
