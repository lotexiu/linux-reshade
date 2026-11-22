//! O que a tela mostra: os dados que o modelo entrega à interface (jogos, presets, linhas de técnica, grupos de efeito e o retrato inteiro).

use super::*;

/// Um jogo da lista: tem pasta de presets, está aberto, ou os dois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameEntry {
    pub id: String,
    pub open: bool,
}

/// O preset que está sendo mostrado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetInfo {
    pub path: PathBuf,
    /// Nome para mostrar (`Noturno`, sem a extensão).
    pub name: String,
    /// Veio do `game.ini` do jogo, ou é o global que o jogo herda.
    pub inherited: bool,
}

/// Uma técnica do preset.
#[derive(Debug, Clone, PartialEq)]
pub struct TechniqueView {
    pub id: TechId,
    /// `ui_label` da técnica, ou o nome.
    pub label: String,
    /// Arquivo do efeito (`Sepia.fx`).
    pub file: String,
    pub enabled: bool,
    /// Os parâmetros do efeito, na primeira técnica de cada arquivo (as outras do mesmo arquivo não os repetem).
    pub controls: Vec<Control>,
    /// O que a layer do jogo aberto fez com esta técnica (aplicada ou pulada e por quê); `None` se o jogo não está
    /// aberto com este preset, ou a técnica está desligada.
    pub runtime: Option<TechniqueState>,
    /// Por que não dá para mostrar os parâmetros ou rodar (efeito não achado, ainda não compilado...).
    pub problem: Option<String>,
    /// O atalho que liga e desliga a técnica (formato XDG, `CTRL+F11`); vazio: sem atalho. Vem do `Key<técnica>` do preset.
    pub key: String,
}

/// Um efeito (um arquivo `.fx`) com as técnicas dele: é a unidade que a pessoa vê, move e remove. As técnicas de um
/// mesmo efeito (por exemplo o desfoque horizontal e o vertical) andam juntas e partilham os parâmetros do efeito;
/// dá para ligar e desligar cada uma, mas não remover só uma.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectGroup {
    /// Arquivo do efeito (`cGaussianBlur.fx`).
    pub file: String,
    /// Nome para mostrar: o do arquivo sem `.fx`.
    pub name: String,
    pub techniques: Vec<TechniqueView>,
    /// Os parâmetros do efeito (de um só lugar, não repetidos por técnica).
    pub controls: Vec<Control>,
    /// As definições de pré-processador só deste efeito (`NOME=VALOR,OUTRO=2`). Mudar recompila o efeito.
    pub macros: String,
}

impl EffectGroup {
    /// Alguma técnica do efeito está ligada.
    pub fn any_enabled(&self) -> bool {
        self.techniques.iter().any(|t| t.enabled)
    }

    pub fn all_enabled(&self) -> bool {
        self.techniques.iter().all(|t| t.enabled)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HomeSnapshot {
    /// Os efeitos estão ligados (a flag global que o atalho alterna).
    pub enabled: bool,
    pub games: Vec<GameEntry>,
    /// O jogo escolhido; `None` é o preset global.
    pub game: Option<String>,
    /// Presets do jogo escolhido (nomes sem extensão), e qual está em uso (`None`: usa o global).
    pub presets: Vec<String>,
    pub active_preset: Option<String>,
    pub preset: Option<PresetInfo>,
    pub techniques: Vec<TechniqueView>,
    /// As mesmas técnicas agrupadas por efeito, na ordem em que cada efeito aparece pela primeira vez.
    pub groups: Vec<EffectGroup>,
    /// As definições de pré-processador do preset inteiro (`NOME=VALOR,OUTRO=2`); valem para todos os efeitos dele.
    pub preset_macros: String,
    /// Efeitos que podem ser adicionados (nomes de arquivo sem `.fx`).
    pub effects: Vec<String>,
    /// A subpasta de cada efeito, relativa à pasta de busca que o contém (`Cor/PD80`; vazio: direto na pasta de busca).
    pub folders: HashMap<String, String>,
    /// O que se sabe de cada efeito (compatibilidade e características), conforme a análise em segundo plano avança.
    pub infos: HashMap<String, EffectInfo>,
    /// Andamento da análise: efeitos analisados e total.
    pub analysis: (usize, usize),
    pub layers: Vec<LiveLayer>,
    /// O que dizer sobre a profundidade do jogo escolhido (`None`: nada a dizer).
    pub depth_note: Option<crate::DepthNote>,
    /// As candidatas de profundidade do jogo aberto e o que a pessoa pediu (vazio: nada a escolher).
    pub depth_choices: Vec<crate::DepthChoice>,
    /// As limpezas de profundidade do último quadro do jogo aberto (para o painel listar os pontos de captura possíveis).
    pub depth_clears: Vec<crate::DepthClear>,
    pub depth_passes: Vec<crate::DepthPass>,
    pub depth_pick: ipc::DepthPick,
    /// A última mensagem para o usuário; some na próxima ação.
    pub notice: Option<String>,
}
