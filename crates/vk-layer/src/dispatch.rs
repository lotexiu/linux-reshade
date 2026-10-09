//! Registro de dados por objeto Vulkan.
//!
//! O loader coloca, como primeira palavra de todo handle despachável, um ponteiro para a sua
//! tabela de dispatch. Todos os handles ligados ao mesmo instance/device compartilham esse
//! ponteiro, então ele serve de chave: o `PhysicalDevice` acha o dado do `Instance`, e a
//! `Queue`/`CommandBuffer` acha o do `Device`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Chave de dispatch de um handle despachável (`as_raw()` do `ash`).
///
/// # Safety
/// `raw` deve ser um handle despachável válido, entregue pelo loader.
pub unsafe fn dispatch_key(raw: u64) -> usize {
    // SAFETY: handles despacháveis são ponteiros para uma struct cujo primeiro campo é um ponteiro.
    unsafe { *(raw as usize as *const usize) }
}

/// Mapa chave de dispatch -> dado. Os dados são `Arc` para a chamada seguir usando-os depois de
/// soltar o lock (e para sobreviverem a um `Destroy` concorrente).
pub struct Registry<T> {
    map: RwLock<HashMap<usize, Arc<T>>>,
}

impl<T> Registry<T> {
    pub fn new() -> Self {
        Self { map: RwLock::new(HashMap::new()) }
    }

    pub fn insert(&self, key: usize, value: T) -> Arc<T> {
        let value = Arc::new(value);
        self.map.write().unwrap().insert(key, value.clone());
        value
    }

    pub fn get(&self, key: usize) -> Option<Arc<T>> {
        self.map.read().unwrap().get(&key).cloned()
    }

    pub fn remove(&self, key: usize) -> Option<Arc<T>> {
        self.map.write().unwrap().remove(&key)
    }
}

impl<T> Default for Registry<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_first_word_of_the_object() {
        let table = 0xABCDusize;
        let object = [table, 0usize]; // objeto falso: [ptr da tabela, resto]
        assert_eq!(unsafe { dispatch_key(object.as_ptr() as u64) }, table);
    }

    #[test]
    fn registry_insert_get_remove() {
        let r = Registry::new();
        r.insert(1, "a");
        assert_eq!(*r.get(1).unwrap(), "a");
        assert!(r.get(2).is_none());
        assert_eq!(*r.remove(1).unwrap(), "a");
        assert!(r.get(1).is_none());
    }
}
