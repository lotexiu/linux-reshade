use ash::vk;
use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// Uma chamada do Vulkan falhou.
    Vulkan(vk::Result),
    /// O efeito (ou o alvo) usa algo que esta versão não executa; a mensagem diz o quê.
    Unsupported(String),
    /// O bundle ou os argumentos são inconsistentes.
    Invalid(String),
}

impl From<vk::Result> for Error {
    fn from(r: vk::Result) -> Self {
        Error::Vulkan(r)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Vulkan(r) => write!(f, "falha do Vulkan (código {})", r.as_raw()),
            Error::Unsupported(what) => write!(f, "não suportado: {what}"),
            Error::Invalid(why) => write!(f, "inválido: {why}"),
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn unsupported(what: impl Into<String>) -> Error {
    Error::Unsupported(what.into())
}
