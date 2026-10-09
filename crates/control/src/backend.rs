//! Escolha do backend de atalhos globais. Hoje só o portal; X11 e evdev entram aqui quando existirem.

use shortcuts::{Backend, Error};
use shortcuts_portal::PortalBackend;

pub fn select() -> Result<Box<dyn Backend>, Error> {
    PortalBackend::connect().map(|b| Box::new(b) as Box<dyn Backend>)
}
