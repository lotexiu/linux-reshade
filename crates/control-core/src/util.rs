//! Pequenos auxiliares do núcleo que mais de um módulo usa.

use std::path::Path;
use std::time::SystemTime;

/// A data de modificação de um arquivo, ou `None` se ele não existe (ou o sistema não informa).
pub(crate) fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
