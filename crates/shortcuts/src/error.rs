use std::fmt;

#[derive(Debug)]
pub enum Error {
    /// O sistema não oferece este backend (portal ausente, sessão sem suporte etc.).
    Unavailable(String),
    /// O backend falhou no meio do caminho.
    Backend(String),
    /// O usuário recusou o registro.
    Denied,
    /// A fonte de eventos fechou (sessão encerrada).
    Closed,
    /// Dois atalhos com o mesmo `id`.
    DuplicateId(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unavailable(why) => write!(f, "backend indisponível: {why}"),
            Error::Backend(why) => write!(f, "falha no backend: {why}"),
            Error::Denied => write!(f, "o usuário recusou o registro dos atalhos"),
            Error::Closed => write!(f, "a fonte de eventos foi encerrada"),
            Error::DuplicateId(id) => write!(f, "atalho duplicado: {id:?}"),
        }
    }
}

impl std::error::Error for Error {}
