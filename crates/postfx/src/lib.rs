//! Passes de pós-processamento sobre imagens já renderizadas.
//!
//! O crate não conhece layers nem swapchains: recebe as imagens do alvo ([`effect::Target`]) e um
//! bundle de efeito, e devolve command buffers prontos, um por imagem. Quem usa (a layer) decide
//! quando e em qual fila submetê-los.
//!
//! - [`chain`]: vários efeitos em ordem sobre o mesmo alvo (a cadeia de um preset)
//! - [`effect`]: executa uma técnica de um bundle (múltiplos passes, texturas, uniforms)
//! - [`backbuffer`]: cópias amostráveis do alvo (um shader não lê e escreve a mesma imagem)

pub mod backbuffer;
pub mod chain;
pub mod effect;
mod error;
mod image;
mod memory;

pub use backbuffer::BackbufferCopies;
pub use chain::{Chain, Skipped, Step};
pub use effect::{Effect, Target};
pub use error::Error;
