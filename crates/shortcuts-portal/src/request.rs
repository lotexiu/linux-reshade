//! Padrão request/response dos portais: a chamada devolve o caminho de um *request* e o resultado
//! chega no sinal `Response` desse caminho.

use crate::{DESTINATION, backend_err};
use shortcuts::Error;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{DynamicType, OwnedValue};
use i18n::{fmt, tr};

const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";

pub enum Outcome {
    Success(HashMap<String, OwnedValue>),
    /// Código 1: o usuário cancelou o diálogo.
    Cancelled,
    /// Código 2 ou outro: falha da interação.
    Failed(u32),
}

impl Outcome {
    pub fn into_results(self) -> Result<HashMap<String, OwnedValue>, Error> {
        match self {
            Outcome::Success(results) => Ok(results),
            Outcome::Cancelled => Err(Error::Denied),
            Outcome::Failed(code) => Err(Error::Backend(fmt("the portal answered with code {}", &[&code]))),
        }
    }
}

/// `/org/freedesktop/portal/desktop/request/<remetente>/<token>`, onde o remetente é o nome único
/// da conexão sem `:` e com `.` trocado por `_`.
pub(crate) fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("/org/freedesktop/portal/desktop/request/{sender}/{token}")
}

pub struct Requester {
    connection: Connection,
    counter: AtomicU32,
}

impl Requester {
    pub fn new(connection: Connection) -> Self {
        Self { connection, counter: AtomicU32::new(0) }
    }

    fn next_token(&self) -> String {
        format!("lr{}_{}", std::process::id(), self.counter.fetch_add(1, Ordering::Relaxed))
    }

    /// Chama `method` e espera o `Response`. `body` recebe o `handle_token` (por valor) para pôr nas opções.
    pub fn call<B>(&self, portal: &Proxy<'_>, method: &str, body: impl FnOnce(String) -> B) -> Result<Outcome, Error>
    where
        B: zbus::export::serde::Serialize + DynamicType,
    {
        let token = self.next_token();
        let unique = self.connection.unique_name().ok_or_else(|| Error::Backend(tr("connection without a unique name").into()))?;
        let path = request_path(unique.as_str(), &token);
        let request = Proxy::new(&self.connection, DESTINATION, path, REQUEST_INTERFACE).map_err(backend_err)?;
        // Assina antes de chamar: o Response pode chegar antes de a chamada retornar.
        let mut responses = request.receive_signal("Response").map_err(backend_err)?;
        portal.call_method(method, &body(token)).map_err(backend_err)?;
        let message = responses.next().ok_or(Error::Closed)?;
        let (code, results): (u32, HashMap<String, OwnedValue>) = message.body().deserialize().map_err(backend_err)?;
        Ok(match code {
            0 => Outcome::Success(results),
            1 => Outcome::Cancelled,
            other => Outcome::Failed(other),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_path_follows_portal_convention() {
        assert_eq!(
            request_path(":1.154", "tok"),
            "/org/freedesktop/portal/desktop/request/1_154/tok"
        );
    }
}
