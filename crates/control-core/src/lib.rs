//! O que o app de controle faz, sem saber como é apresentado: a linha de comando (`lrctl`) e a interface
//! gráfica usam as mesmas peças.
//!
//! - [`action`]: as ações do usuário (ligar/desligar efeitos) sobre o estado compartilhado
//! - [`analysis`]: o que cada efeito pede e se o runtime dá conta (lista "Adicionar efeito")
//! - [`compiler`]: compilação sob demanda dos efeitos por resolução, com cache
//! - [`game_plan`]: do preset do jogo ao plano que a layer executa
//! - [`games`]: o preset de cada jogo aberto vira o plano da layer dele
//! - [`layers`]: a lista de jogos com a layer ativa
//! - [`location`]: onde estão os dados do usuário (pastas de shaders, presets, cache)
//! - [`params`]: listar e alterar os parâmetros de um efeito
//! - [`presets`]: editar presets (técnicas, ordem, valores; preset de cada jogo)
//! - [`select`]: escolher o efeito que a layer executa
//! - [`watch`]: o vigia, que mantém compilado o que os jogos abertos precisam

pub mod action;
pub mod analysis;
pub mod compiler;
pub mod game_plan;
pub mod games;
pub mod layers;
pub mod location;
pub mod params;
pub mod patches;
pub mod presets;
pub mod select;
pub mod watch;
