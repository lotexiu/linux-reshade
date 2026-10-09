use crate::{Error, Event, Phase, Scope, Shortcut};

/// Liga um atalho a uma ação do app. `A` é o tipo de ação definido por quem usa a crate.
#[derive(Clone, Debug)]
pub struct Binding<A> {
    pub shortcut: Shortcut,
    pub scope: Scope,
    /// Em qual fase a ação dispara (normalmente ao pressionar).
    pub phase: Phase,
    pub action: A,
}

impl<A> Binding<A> {
    /// Atalho que dispara ao pressionar.
    pub fn on_press(shortcut: Shortcut, scope: Scope, action: A) -> Self {
        Self { shortcut, scope, phase: Phase::Pressed, action }
    }
}

/// Estado do app que decide quais scopes estão ativos.
#[derive(Clone, Copy, Debug, Default)]
pub struct Context {
    pub window_visible: bool,
}

/// Tabela de atalhos e a regra de qual ação um evento dispara.
pub struct Dispatcher<A> {
    bindings: Vec<Binding<A>>,
}

impl<A> Dispatcher<A> {
    pub fn new(bindings: Vec<Binding<A>>) -> Result<Self, Error> {
        for (i, b) in bindings.iter().enumerate() {
            if bindings[..i].iter().any(|o| o.shortcut.id == b.shortcut.id) {
                return Err(Error::DuplicateId(b.shortcut.id.clone()));
            }
        }
        Ok(Self { bindings })
    }

    /// Atalhos que precisam ser registrados no sistema (só os de scope `Global`).
    pub fn global_shortcuts(&self) -> Vec<Shortcut> {
        self.bindings
            .iter()
            .filter(|b| b.scope == Scope::Global)
            .map(|b| b.shortcut.clone())
            .collect()
    }

    /// Ação que o evento dispara no contexto atual, se alguma.
    pub fn resolve(&self, event: &Event, ctx: &Context) -> Option<&A> {
        let binding = self.bindings.iter().find(|b| b.shortcut.id == event.id)?;
        let active = match binding.scope {
            Scope::Global => true,
            Scope::WindowVisible => ctx.window_visible,
        };
        (active && binding.phase == event.phase).then_some(&binding.action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(id: &str) -> Shortcut {
        Shortcut { id: id.into(), description: id.into(), preferred_trigger: None }
    }

    fn event(id: &str, phase: Phase) -> Event {
        Event { id: id.into(), phase, timestamp: 0 }
    }

    fn dispatcher() -> Dispatcher<&'static str> {
        Dispatcher::new(vec![
            Binding::on_press(shortcut("toggle"), Scope::Global, "toggle-effects"),
            Binding::on_press(shortcut("reload"), Scope::WindowVisible, "reload-shaders"),
            Binding { phase: Phase::Released, ..Binding::on_press(shortcut("hold"), Scope::Global, "hold-end") },
        ])
        .unwrap()
    }

    #[test]
    fn global_fires_regardless_of_window() {
        let d = dispatcher();
        for visible in [false, true] {
            let ctx = Context { window_visible: visible };
            assert_eq!(d.resolve(&event("toggle", Phase::Pressed), &ctx), Some(&"toggle-effects"));
        }
    }

    #[test]
    fn window_scope_only_when_visible() {
        let d = dispatcher();
        let hidden = Context { window_visible: false };
        let shown = Context { window_visible: true };
        assert_eq!(d.resolve(&event("reload", Phase::Pressed), &hidden), None);
        assert_eq!(d.resolve(&event("reload", Phase::Pressed), &shown), Some(&"reload-shaders"));
    }

    #[test]
    fn respects_trigger_phase() {
        let d = dispatcher();
        let ctx = Context::default();
        assert_eq!(d.resolve(&event("toggle", Phase::Released), &ctx), None);
        assert_eq!(d.resolve(&event("hold", Phase::Pressed), &ctx), None);
        assert_eq!(d.resolve(&event("hold", Phase::Released), &ctx), Some(&"hold-end"));
    }

    #[test]
    fn unknown_id_does_nothing() {
        assert_eq!(dispatcher().resolve(&event("nope", Phase::Pressed), &Context::default()), None);
    }

    #[test]
    fn only_global_shortcuts_are_registered_with_the_system() {
        let ids: Vec<_> = dispatcher().global_shortcuts().into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["toggle", "hold"]);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let r = Dispatcher::new(vec![
            Binding::on_press(shortcut("a"), Scope::Global, 1),
            Binding::on_press(shortcut("a"), Scope::WindowVisible, 2),
        ]);
        assert!(matches!(r, Err(Error::DuplicateId(id)) if id == "a"));
    }
}
