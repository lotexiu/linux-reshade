//! Pequenas ferramentas para a interface não fazer trabalho à toa (o que num app web se faz com `useMemo`, `useCallback` e debounce).
//!
//! - [`Throttle`]: uma entrada contínua (arrastar um slider) vira, no máximo, uma ação por intervalo, e a última sempre vale.

use gtk::glib;
use gtk4 as gtk;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// A primeira chamada roda na hora; as seguintes, dentro do intervalo, se juntam numa só (a mais nova) que roda ao fim dele.
#[derive(Clone)]
pub struct Throttle {
    ms: u64,
    pending: Rc<RefCell<Option<Box<dyn FnOnce()>>>>,
    waiting: Rc<Cell<bool>>,
}

impl Throttle {
    pub fn new(ms: u64) -> Self {
        Self { ms, pending: Rc::new(RefCell::new(None)), waiting: Rc::new(Cell::new(false)) }
    }

    pub fn call(&self, action: impl FnOnce() + 'static) {
        if self.waiting.get() {
            *self.pending.borrow_mut() = Some(Box::new(action));
            return;
        }
        self.waiting.set(true);
        action();
        self.schedule();
    }

    fn schedule(&self) {
        let me = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(self.ms), move || {
            let next = me.pending.borrow_mut().take();
            match next {
                Some(action) => {
                    action();
                    me.schedule();
                }
                None => me.waiting.set(false),
            }
        });
    }
}
