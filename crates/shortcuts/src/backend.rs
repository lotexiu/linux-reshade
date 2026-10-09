use crate::{Error, Event, Shortcut};

/// Resultado do registro de um atalho: o que o sistema de fato associou.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundShortcut {
    pub id: String,
    /// Texto legível da combinação em vigor (pode ser diferente da sugerida).
    pub trigger_description: String,
}

/// Fonte de atalhos globais de um sistema (portal, X11, evdev...).
pub trait Backend: Send {
    /// Nome curto para logs (`"portal"`, `"x11"`).
    fn name(&self) -> &'static str;

    /// Registra os atalhos no sistema. Pode abrir diálogos de confirmação.
    fn bind(&mut self, shortcuts: &[Shortcut]) -> Result<Vec<BoundShortcut>, Error>;

    /// Bloqueia até o próximo evento. `Err(Error::Closed)` quando não vierem mais.
    fn next_event(&mut self) -> Result<Event, Error>;
}
