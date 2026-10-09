use std::fmt;
use std::path::PathBuf;

#[derive(Debug)]
pub enum Error {
    /// Falha ao ler um arquivo do bundle.
    Io { path: PathBuf, source: std::io::Error },
    /// O `manifest.json` não é um JSON do formato esperado.
    Parse(String),
    /// O bundle usa algo que esta versão do runtime não suporta (por exemplo, depth ou compute).
    Unsupported(String),
    /// O bundle é inconsistente (referência a algo que não existe, valores fora de faixa).
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::Parse(why) => write!(f, "manifest.json inválido: {why}"),
            Error::Unsupported(what) => write!(f, "não suportado: {what}"),
            Error::Invalid(why) => write!(f, "bundle inválido: {why}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
