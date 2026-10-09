/// Descrição de um atalho que o app quer registrar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// Identificador estável, usado nos eventos e guardado pelo sistema (não mude entre versões).
    pub id: String,
    /// Texto mostrado ao usuário nas configurações do sistema.
    pub description: String,
    /// Sugestão de combinação no formato XDG (`"CTRL+SHIFT+F10"`). O usuário pode trocá-la, então
    /// o app nunca deve assumir que é essa a tecla.
    pub preferred_trigger: Option<String>,
}

/// Quando um atalho vale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Sempre, mesmo com o jogo em foco. Precisa ser registrado no sistema.
    Global,
    /// Só enquanto a janela do app de controle está visível. Vem dos eventos da própria janela,
    /// por isso nunca é registrado no sistema.
    WindowVisible,
}

/// Pressionar ou soltar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pressed,
    Released,
}

/// Um atalho foi acionado.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: String,
    pub phase: Phase,
    /// Milissegundos em relógio monotônico, como entregue pelo sistema.
    pub timestamp: u64,
}
