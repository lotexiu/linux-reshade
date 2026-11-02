//! Backend de [`shortcuts::Backend`] sobre o portal `org.freedesktop.portal.GlobalShortcuts`.
//!
//! Protocolo (validado em spike no GNOME 50.1):
//! `CreateSession` -> `BindShortcuts` -> sinais `Activated`/`Deactivated`. Cada método devolve um
//! *request*; o resultado vem no sinal `Response` desse request. Como o sinal pode chegar antes de
//! a chamada retornar, o caminho do request é calculado antes e o sinal é assinado antes da chamada.

mod kde;
mod request;

pub use kde::forget;

use request::Requester;
use shortcuts::{Backend, BoundShortcut, Error, Event, Phase, Shortcut};
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy, proxy::SignalIterator};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

fn backend_err(e: impl std::fmt::Display) -> Error {
    Error::Backend(e.to_string())
}

pub struct PortalBackend {
    requester: Requester,
    proxy: Proxy<'static>,
    session: Option<OwnedObjectPath>,
    signals: Option<SignalIterator<'static>>,
}

impl PortalBackend {
    /// Conecta ao barramento de sessão e confirma que o portal oferece `GlobalShortcuts`.
    pub fn connect() -> Result<Self, Error> {
        let connection = Connection::session().map_err(|e| Error::Unavailable(e.to_string()))?;
        let proxy = Proxy::new(&connection, DESTINATION, PORTAL_PATH, INTERFACE)
            .map_err(|e| Error::Unavailable(e.to_string()))?;
        // Só a existência da propriedade interessa: confirma que o portal expõe a interface.
        proxy
            .get_property::<u32>("version")
            .map_err(|e| Error::Unavailable(format!("portal sem GlobalShortcuts: {e}")))?;
        Ok(Self { requester: Requester::new(connection), proxy, session: None, signals: None })
    }

    fn create_session(&mut self) -> Result<OwnedObjectPath, Error> {
        let outcome = self.requester.call(&self.proxy, "CreateSession", |token| {
            let mut options: HashMap<&str, Value<'_>> = HashMap::new();
            options.insert("handle_token", Value::from(token));
            options.insert("session_handle_token", Value::from("linux_reshade"));
            (options,)
        })?;
        let results = outcome.into_results()?;
        // O portal devolve o caminho da sessão como string (`s`), não como `o`.
        let handle = results
            .get("session_handle")
            .ok_or_else(|| Error::Backend("CreateSession sem session_handle".into()))?;
        let text: String = handle.try_clone().map_err(backend_err)?.try_into().map_err(backend_err)?;
        OwnedObjectPath::try_from(text).map_err(backend_err)
    }
}

impl Backend for PortalBackend {
    fn name(&self) -> &'static str {
        "portal"
    }

    fn bind(&mut self, shortcuts: &[Shortcut]) -> Result<Vec<BoundShortcut>, Error> {
        let session = self.create_session()?;
        // Assina antes de registrar: depois do BindShortcuts os eventos já podem chegar.
        self.signals = Some(self.proxy.receive_all_signals().map_err(backend_err)?);

        let wire: Vec<(String, HashMap<String, Value<'_>>)> = shortcuts
            .iter()
            .map(|s| {
                let mut props: HashMap<String, Value<'_>> = HashMap::new();
                props.insert("description".into(), Value::from(s.description.clone()));
                if let Some(trigger) = &s.preferred_trigger {
                    props.insert("preferred_trigger".into(), Value::from(trigger.clone()));
                }
                (s.id.clone(), props)
            })
            .collect();

        let outcome = self.requester.call(&self.proxy, "BindShortcuts", |token| {
            let mut options: HashMap<&str, Value<'_>> = HashMap::new();
            options.insert("handle_token", Value::from(token));
            (ObjectPath::from(&session), wire.clone(), "", options)
        })?;
        let results = outcome.into_results()?;
        self.session = Some(session);

        let bound: Vec<(String, HashMap<String, OwnedValue>)> = results
            .get("shortcuts")
            .ok_or_else(|| Error::Backend("BindShortcuts sem shortcuts".into()))?
            .try_clone()
            .map_err(backend_err)?
            .try_into()
            .map_err(backend_err)?;
        Ok(bound
            .into_iter()
            .map(|(id, props)| {
                let trigger_description = props
                    .get("trigger_description")
                    .and_then(|v| v.try_clone().ok())
                    .and_then(|v| String::try_from(v).ok())
                    .unwrap_or_default();
                BoundShortcut { id, trigger_description }
            })
            .collect())
    }

    fn next_event(&mut self) -> Result<Event, Error> {
        let session = self.session.clone().ok_or_else(|| Error::Backend("bind() ainda não foi chamado".into()))?;
        let signals = self.signals.as_mut().ok_or(Error::Closed)?;
        loop {
            let message = signals.next().ok_or(Error::Closed)?;
            let header = message.header();
            let phase = match header.member().map(|m| m.as_str()) {
                Some("Activated") => Phase::Pressed,
                Some("Deactivated") => Phase::Released,
                _ => continue, // ShortcutsChanged etc.
            };
            let (path, id, timestamp, _options): (OwnedObjectPath, String, u64, HashMap<String, OwnedValue>) =
                message.body().deserialize().map_err(backend_err)?;
            if path == session {
                return Ok(Event { id, phase, timestamp });
            }
        }
    }
}
