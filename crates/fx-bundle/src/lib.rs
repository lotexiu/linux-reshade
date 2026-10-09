//! Leitor do bundle de efeito produzido pelo `fx2spv` (formato em `docs/bundle-format.md`).
//!
//! - [`manifest`]: os tipos do `manifest.json`
//! - [`validate`]: as regras que o runtime assume (referências existentes, nada fora da v1)
//! - [`load`]: leitura do diretório do bundle, com limites de tamanho

pub mod error;
pub mod load;
pub mod manifest;
pub mod validate;

pub use error::Error;
pub use load::Bundle;
pub use manifest::Manifest;
