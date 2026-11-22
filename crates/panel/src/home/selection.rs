//! Escolher o jogo e o preset, criar e apagar presets, e o interruptor global dos efeitos.

use super::*;

impl Home {
    pub fn set_enabled(&mut self, on: bool) {
        self.state.set_effects_enabled(on);
        self.done(None);
    }

    /// Escolhe o jogo mostrado (`None`: o preset global).
    pub fn select_game(&mut self, game: Option<&str>) {
        self.game = game.filter(|g| game::valid_game_id(g)).map(str::to_string);
        self.loaded = None;
        self.done(None);
        self.prepare_effects();
    }

    /// Usa um preset do jogo escolhido (`None`: volta a seguir o global).
    pub fn select_preset(&mut self, name: Option<&str>) {
        let result = match (&self.game, self.presets_dir.clone()) {
            (Some(game), Some(dir)) => presets::use_preset(&dir, game, name),
            _ => Err(tr("pick a game first").to_string()),
        };
        self.loaded = None;
        self.done(result.err());
        self.prepare_effects();
    }

    /// Cria um preset do jogo (cópia do que vale agora) e passa a usá-lo.
    pub fn new_preset(&mut self, name: &str) {
        let result = match (&self.game, self.presets_dir.clone()) {
            (Some(game), Some(dir)) => presets::new_for_game(&dir, game, name.trim()).map(drop),
            _ => Err(tr("pick a game first").to_string()),
        };
        self.loaded = None;
        self.done(result.err());
    }

    /// Apaga um preset do jogo escolhido. Se era o que o jogo usava, ele volta a seguir o global.
    pub fn delete_preset(&mut self, name: &str) {
        let result = match (&self.game, self.presets_dir.clone()) {
            (Some(game), Some(dir)) => presets::delete_for_game(&dir, game, name),
            _ => Err(tr("pick a game first").to_string()),
        };
        self.loaded = None;
        self.done(result.err());
    }
}
