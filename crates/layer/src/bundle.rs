//! O bundle de efeito que a layer executa, e a troca dele em tempo de execução.
//!
//! Quem decide qual bundle vale, em ordem: o que o app de controle escolheu com `lrctl use`
//! (região de seleção do `ipc`) e, na falta dele, a variável `LINUX_RESHADE_BUNDLE`.
//!
//! O custo por frame é uma leitura atômica (a geração da seleção). O trabalho de verdade (ler o
//! disco, validar) só acontece quando a escolha muda, e uma troca incrementa a *época*: quem
//! construiu algo a partir do bundle antigo compara a época e se reconstrói.

use crate::log::log;
use fx_bundle::Bundle;
use ipc::SelectionRead;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Valor de `LAST_GENERATION` antes de a seleção ter sido lida.
const NEVER: u64 = u64::MAX;
/// Valor de `LAST_GENERATION` enquanto o app de controle ainda não criou o canal.
const NO_CHANNEL: u64 = u64::MAX - 1;

/// Última geração da seleção que já foi processada.
static LAST_GENERATION: AtomicU64 = AtomicU64::new(NEVER);
/// Sobe a cada troca efetiva do bundle. 0 = nunca houve um.
static EPOCH: AtomicU64 = AtomicU64::new(0);

struct Current {
    bundle: Option<Arc<Bundle>>,
    /// De onde veio (para não contar como troca recarregar o mesmo bundle).
    source: Option<(PathBuf, u64)>,
}

static CURRENT: Mutex<Current> = Mutex::new(Current { bundle: None, source: None });

/// O caminho de `LINUX_RESHADE_BUNDLE`, se houver.
fn from_environment() -> Option<PathBuf> {
    std::env::var_os("LINUX_RESHADE_BUNDLE").map(PathBuf::from)
}

fn load(dir: &PathBuf) -> Option<Arc<Bundle>> {
    match Bundle::load(dir) {
        Ok(bundle) => {
            log!(
                "bundle carregado: {} ({}x{}, {} técnica(s))",
                dir.display(),
                bundle.manifest.target.width,
                bundle.manifest.target.height,
                bundle.manifest.techniques.len()
            );
            Some(Arc::new(bundle))
        }
        Err(e) => {
            log!("bundle {} recusado: {e}", dir.display());
            None
        }
    }
}

/// O bundle vigente e a época dele. Barato quando nada mudou; recarrega quando a escolha mudou.
pub fn current() -> (u64, Option<Arc<Bundle>>) {
    let reader = crate::control::reader();
    let generation = reader.map_or(NO_CHANNEL, |r| u64::from(r.selection_generation()));
    if generation == LAST_GENERATION.load(Ordering::Acquire) {
        // Nada mudou desde a última vez. Só falta tomar o bundle atual.
        let state = CURRENT.lock().unwrap();
        return (EPOCH.load(Ordering::Acquire), state.bundle.clone());
    }

    let selection = reader.map_or(SelectionRead::Cleared, |r| r.read_selection());
    let wanted = match selection {
        // A leitura caiu no meio de uma escrita: mantém o que há e tenta no próximo frame.
        SelectionRead::Unstable => {
            let state = CURRENT.lock().unwrap();
            return (EPOCH.load(Ordering::Acquire), state.bundle.clone());
        }
        SelectionRead::Path(path) => Some(PathBuf::from(path)),
        SelectionRead::Cleared => from_environment(),
    };

    let mut state = CURRENT.lock().unwrap();
    let loaded = wanted.as_ref().and_then(load);
    let key = wanted.zip(loaded.as_ref().map(|b| b.id));
    if key != state.source {
        state.bundle = loaded;
        state.source = key;
        EPOCH.fetch_add(1, Ordering::AcqRel);
    }
    LAST_GENERATION.store(generation, Ordering::Release);
    (EPOCH.load(Ordering::Acquire), state.bundle.clone())
}
