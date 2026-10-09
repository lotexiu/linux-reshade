//! Parte genérica de uma layer Vulkan: a ABI de negociação com o loader, a busca dos links na
//! cadeia de create-info e o registro de dispatch por handle. Não tem estado global nem sabe o
//! que a layer faz, para poder ser testada sem GPU.

pub mod abi;
pub mod chain;
pub mod dispatch;
