//! Atalhos de teclado independentes de sistema de janelas.
//!
//! - [`model`]: o que é um atalho, um evento e um scope.
//! - [`dispatch`]: dado um evento e o contexto, qual ação executar (regra dos scopes).
//! - [`backend`]: a trait que cada sistema (portal, X11, evdev) implementa para entregar eventos.
//!
//! Nada aqui conhece D-Bus, X11 nem Vulkan, então tudo é testável sem sessão gráfica.

pub mod backend;
pub mod dispatch;
pub mod error;
pub mod model;

pub use backend::{Backend, BoundShortcut};
pub use dispatch::{Binding, Context, Dispatcher};
pub use error::Error;
pub use model::{Event, Phase, Scope, Shortcut};
