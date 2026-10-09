//! O que o app de controle faz, sem saber como é apresentado: a linha de comando (`lrctl`) e a interface
//! gráfica usam as mesmas peças.
//!
//! - [`action`]: as ações do usuário (ligar/desligar efeitos) sobre o estado compartilhado
//! - [`compiler`]: compilação sob demanda dos efeitos por resolução, com cache
//! - [`layers`]: a lista de jogos com a layer ativa
//! - [`location`]: onde estão os dados do usuário (pastas de shaders, presets, cache)
//! - [`params`]: listar e alterar os parâmetros de um efeito
//! - [`select`]: escolher o efeito que a layer executa
//! - [`watch`]: o vigia, que mantém compilado o que os jogos abertos precisam

pub mod action;
pub mod compiler;
pub mod layers;
pub mod location;
pub mod params;
pub mod select;
pub mod watch;
