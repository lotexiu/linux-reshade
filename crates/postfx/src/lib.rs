//! Passes de pós-processamento sobre imagens já renderizadas.
//!
//! O crate não conhece layers nem swapchains: recebe as imagens do alvo ([`Target`]) e devolve
//! command buffers prontos, um por imagem. Quem usa (a layer) decide quando e em qual fila submetê-los.
//!
//! - [`backbuffer`]: cópias amostráveis do alvo (um shader não lê e escreve a mesma imagem)
//! - [`invert`]: o primeiro passe, que inverte as cores (marco 2 do ROADMAP)

pub mod backbuffer;
pub mod invert;
mod memory;
mod spirv;

pub use backbuffer::BackbufferCopies;
pub use invert::{InvertPass, Target};
