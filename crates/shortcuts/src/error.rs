use std::fmt;
use i18n::{fmt, tr};

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
            Error::Unavailable(why) => write!(f, "{}", fmt("backend unavailable: {}", &[&why])),
            Error::Backend(why) => write!(f, "{}", fmt("backend failure: {}", &[&why])),
            Error::Denied => write!(f, "{}", tr("the user refused to register the shortcuts")),
            Error::Closed => write!(f, "a fonte de eventos foi encerrada"),
            Error::DuplicateId(id) => write!(f, "{}", fmt("duplicate shortcut: {}", &[&format!("{:?}", id)])),
        }
    }
}

impl std::error::Error for Error {}
