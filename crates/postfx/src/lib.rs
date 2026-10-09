//! Passes de pós-processamento sobre imagens já renderizadas.
//!
//! O crate não conhece layers nem swapchains: recebe as imagens do alvo ([`Target`]) e devolve
//! command buffers prontos, um por imagem. Quem usa (a layer) decide quando e em qual fila submetê-los.
//!
//! - [`backbuffer`]: cópias amostráveis do alvo (um shader não lê e escreve a mesma imagem)
//! - [`invert`]: o primeiro passe, que inverte as cores (marco 2 do ROADMAP)

pub mod backbuffer;
pub mod effect;
mod error;
mod image;
pub mod invert;
mod memory;
mod spirv;

pub use backbuffer::BackbufferCopies;
pub use effect::Effect;
pub use error::Error;
pub use invert::{InvertPass, Target};
// `effect::Target` e `invert::Target` têm o mesmo formato; o `Effect` é o caminho novo.
