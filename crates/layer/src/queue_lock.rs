//! Um trinco por `VkQueue`. O Vulkan exige que uma fila não seja usada por duas threads ao mesmo tempo (`vkQueueSubmit`, `vkQueueWaitIdle` e
//! `vkQueuePresentKHR` pedem sincronização externa). A layer submete trabalho dos efeitos na fila do jogo de dentro do gancho do present, e o
//! jogo submete na mesma fila de outra thread; um jogo que sincroniza o próprio submit e o próprio present (o DXVK) não sincroniza o NOSSO
//! submit. Dois comandos ao mesmo tempo na fila corrompem o que a GPU vai executar. Os submits do jogo (o gancho) e os nossos passam por aqui.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

type Locks = Mutex<HashMap<u64, Arc<Mutex<()>>>>;

fn locks() -> &'static Locks {
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// O trinco de uma fila (o handle bruto), criado na primeira vez.
pub(crate) fn of(queue: u64) -> Arc<Mutex<()>> {
    locks().lock().unwrap_or_else(|e| e.into_inner()).entry(queue).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
}

/// Pega o trinco da fila enquanto durar `f`.
pub(crate) fn with<T>(queue: u64, f: impl FnOnce() -> T) -> T {
    let lock = of(queue);
    let _guard: MutexGuard<'_, ()> = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}
