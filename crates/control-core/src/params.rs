//! Listar e alterar os parâmetros (uniforms) de um efeito. A lógica não faz E/S de terminal: recebe o
//! bundle e o canal com a layer e devolve valores, para ser testada sem um jogo rodando.

use fx_bundle::manifest::{Scalar, Uniform, Value};
use fx_bundle::{Bundle, uniforms};
use ipc::{ChannelWriter, ParamSource, ParamStore, PublishError, Writer};
use std::fmt;

/// Os parâmetros de **um jogo** sobre os globais: lê o bloco do jogo e, se ele não tem, o global (o jogo
/// começa herdando os valores de todos); grava sempre no canal do jogo.
pub struct Layered<'a> {
    pub own: &'a ChannelWriter,
    pub global: &'a Writer,
}

impl ParamSource for Layered<'_> {
    fn read_params(&self, bundle_id: u64, out: &mut [u8]) -> Option<usize> {
        self.own.read_params(bundle_id, out).or_else(|| self.global.read_params(bundle_id, out))
    }

    fn published_bundle(&self) -> u64 {
        match self.own.published_bundle() {
            0 => self.global.published_bundle(),
            id => id,
        }
    }
}

impl ParamStore for Layered<'_> {
    fn publish_params(&self, bundle_id: u64, data: &[u8]) -> Result<(), PublishError> {
        self.own.publish_params(bundle_id, data)
    }
}

#[derive(Debug)]
pub enum Error {
    /// Não existe um parâmetro com esse nome.
    NotFound { name: String, available: Vec<String> },
    /// O uniform é preenchido pelo runtime (tempo, número do frame...), não é um controle.
    RuntimeSource { name: String, source: String },
    BadValue(String),
    Bundle(fx_bundle::Error),
    Publish(ipc::PublishError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound { name, available } => {
                write!(f, "não existe o parâmetro {name:?}")?;
                if available.is_empty() { write!(f, " (este efeito não tem parâmetros)") } else { write!(f, " (existem: {})", available.join(", ")) }
            }
            Error::RuntimeSource { name, source } => write!(f, "{name:?} é preenchido pelo runtime (source = {source:?}); não é um controle"),
            Error::BadValue(why) => write!(f, "valor inválido: {why}"),
            Error::Bundle(e) => write!(f, "{e}"),
            Error::Publish(e) => write!(f, "não consegui publicar os parâmetros: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<fx_bundle::Error> for Error {
    fn from(e: fx_bundle::Error) -> Self {
        Error::Bundle(e)
    }
}

/// Um parâmetro como o usuário o vê.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub name: String,
    /// `float`, `int`, `float3`...
    pub type_name: String,
    pub value: Vec<f64>,
    pub default: Vec<f64>,
    pub label: Option<String>,
    pub range: Option<(f64, f64)>,
    /// `Some(fonte)` se o runtime preenche este uniform.
    pub source: Option<String>,
}

fn text<'a>(u: &'a Uniform, key: &str) -> Option<&'a str> {
    match u.annotations.get(key) {
        Some(Value::Scalar(Scalar::Text(t))) => Some(t),
        _ => None,
    }
}

fn first_number(u: &Uniform, key: &str) -> Option<f64> {
    match u.annotations.get(key) {
        Some(Value::Scalar(Scalar::Number(n))) => Some(*n),
        Some(Value::List(l)) => l.iter().find_map(|s| if let Scalar::Number(n) = s { Some(*n) } else { None }),
        _ => None,
    }
}

fn type_name(u: &Uniform) -> String {
    use fx_bundle::manifest::ScalarBase::*;
    let base = match u.ty.base {
        Bool => "bool",
        Int => "int",
        Uint => "uint",
        Float => "float",
        String => "string",
        Other => "?",
    };
    if u.ty.cols > 1 {
        format!("{base}{}x{}", u.ty.rows, u.ty.cols)
    } else if u.ty.rows > 1 {
        format!("{base}{}", u.ty.rows)
    } else {
        base.to_string()
    }
}

/// A imagem atual do buffer de uniforms: o que está publicado para este efeito, ou os padrões.
pub fn current_block(bundle: &Bundle, state: &dyn ipc::ParamStore) -> Vec<u8> {
    let total = bundle.manifest.total_uniform_size as usize;
    let mut block = vec![0u8; total];
    match state.read_params(bundle.params_id(), &mut block) {
        Some(n) if n == total => block,
        _ => uniforms::default_block(&bundle.manifest),
    }
}

/// Todos os uniforms do efeito, com valor atual e padrão.
pub fn describe(bundle: &Bundle, state: &dyn ipc::ParamStore) -> Vec<Row> {
    let block = current_block(bundle, state);
    let defaults = uniforms::default_block(&bundle.manifest);
    bundle
        .manifest
        .uniforms
        .iter()
        .map(|u| Row {
            name: u.name.clone(),
            type_name: type_name(u),
            value: uniforms::decode(u, &block).unwrap_or_default(),
            default: uniforms::decode(u, &defaults).unwrap_or_default(),
            label: text(u, "ui_label").map(str::to_string),
            range: first_number(u, "ui_min").zip(first_number(u, "ui_max")),
            source: text(u, "source").map(str::to_string),
        })
        .collect()
}

fn lookup<'a>(bundle: &'a Bundle, name: &str) -> Result<&'a Uniform, Error> {
    let u = uniforms::find(&bundle.manifest, name).ok_or_else(|| Error::NotFound {
        name: name.to_string(),
        available: bundle.manifest.uniforms.iter().filter(|u| !uniforms::is_runtime_source(u)).map(|u| u.name.clone()).collect(),
    })?;
    if let Some(source) = text(u, "source") {
        return Err(Error::RuntimeSource { name: name.to_string(), source: source.to_string() });
    }
    Ok(u)
}

/// O valor atual de um parâmetro.
pub fn get(bundle: &Bundle, state: &dyn ipc::ParamStore, name: &str) -> Result<Vec<f64>, Error> {
    let u = lookup(bundle, name)?;
    uniforms::decode(u, &current_block(bundle, state)).ok_or_else(|| Error::BadValue(format!("{name:?} não é um escalar nem um vetor")))
}

#[derive(Debug, PartialEq)]
pub struct Change {
    pub old: Vec<f64>,
    pub new: Vec<f64>,
    /// Avisos que não impedem a alteração (por exemplo, fora da faixa sugerida pelo efeito).
    pub warnings: Vec<String>,
}

/// Altera um parâmetro e publica o bloco completo para a layer.
pub fn set(bundle: &Bundle, state: &dyn ipc::ParamStore, name: &str, values: &[f64]) -> Result<Change, Error> {
    let u = lookup(bundle, name)?;
    let bytes = uniforms::encode(u, values).map_err(|e| Error::BadValue(e.to_string()))?;
    let mut block = current_block(bundle, state);
    let old = uniforms::decode(u, &block).unwrap_or_default();
    let at = u.offset as usize;
    block.get_mut(at..at + bytes.len()).ok_or_else(|| Error::BadValue("o uniform não cabe no buffer".into()))?.copy_from_slice(&bytes);
    state.publish_params(bundle.params_id(), &block).map_err(Error::Publish)?;

    Ok(Change { old, new: uniforms::decode(u, &block).unwrap_or_default(), warnings: range_warnings(u, values) })
}

/// Avisos de valores fora da faixa que o efeito sugere (`ui_min`/`ui_max`); não impedem nada.
pub fn range_warnings(u: &Uniform, values: &[f64]) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some((min, max)) = first_number(u, "ui_min").zip(first_number(u, "ui_max")) {
        for v in values {
            if *v < min || *v > max {
                warnings.push(format!("{v} está fora da faixa sugerida pelo efeito [{min} .. {max}]"));
            }
        }
    }
    warnings
}

/// Volta todos os parâmetros aos padrões do manifesto.
pub fn reset(bundle: &Bundle, state: &dyn ipc::ParamStore) -> Result<(), Error> {
    state.publish_params(bundle.params_id(), &uniforms::default_block(&bundle.manifest)).map_err(Error::Publish)
}

/// Número como o usuário espera ler: inteiros sem casas e floats no menor texto que volta ao mesmo f32
/// (os valores vêm de uniforms de 32 bits, então `0.4` não deve aparecer como `0.4000000059604645`).
pub fn fmt_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        (v as f32).to_string()
    }
}

/// `0.5` para um valor, `(0.1, 0.2, 0.3)` para um vetor.
pub fn fmt_values(values: &[f64]) -> String {
    match values {
        [one] => fmt_number(*one),
        many => format!("({})", many.iter().map(|v| fmt_number(*v)).collect::<Vec<_>>().join(", ")),
    }
}

/// Interpreta os valores digitados: números, `true/false/on/off`, vírgulas ou espaços como separador.
pub fn parse_values(args: &[String]) -> Result<Vec<f64>, Error> {
    args.iter()
        .flat_map(|a| a.split(','))
        .filter(|t| !t.trim().is_empty())
        .map(|t| match t.trim().to_ascii_lowercase().as_str() {
            "true" | "on" | "sim" => Ok(1.0),
            "false" | "off" | "nao" | "não" => Ok(0.0),
            n => n.parse::<f64>().map_err(|_| Error::BadValue(format!("{t:?} não é um número"))),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// O bundle de referência (Sépia, com `Strength` de 0 a 1, padrão 1) e um canal de teste.
    fn setup(name: &str) -> (Bundle, ipc::Writer) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fx-bundle/tests/data/Sepia");
        let bundle = Bundle::load(&dir).expect("bundle de referência");
        let state_path = std::env::temp_dir().join(format!("lr-control-params-{}-{name}", std::process::id())).join("state");
        let _ = std::fs::remove_dir_all(state_path.parent().unwrap());
        (bundle, ipc::Writer::create_at(&state_path).unwrap())
    }

    #[test]
    fn describe_shows_label_range_and_defaults() {
        let (bundle, state) = setup("describe");
        let rows = describe(&bundle, &state);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!((r.name.as_str(), r.type_name.as_str()), ("Strength", "float"));
        assert_eq!((r.value.clone(), r.default.clone()), (vec![1.0], vec![1.0]));
        assert_eq!(r.label.as_deref(), Some("Intensidade"));
        assert_eq!(r.range, Some((0.0, 1.0)));
        assert_eq!(r.source, None);
    }

    #[test]
    fn set_publishes_a_block_the_layer_side_can_read() {
        let (bundle, state) = setup("set");
        let change = set(&bundle, &state, "Strength", &[0.25]).unwrap();
        assert_eq!((change.old, change.new), (vec![1.0], vec![0.25]));
        assert!(change.warnings.is_empty());

        // O que a layer leria: o bloco inteiro do buffer de uniforms, deste efeito.
        let mut seen = vec![0u8; bundle.manifest.total_uniform_size as usize];
        assert_eq!(state.read_params(bundle.params_id(), &mut seen), Some(seen.len()));
        assert_eq!(f32::from_le_bytes(seen[..4].try_into().unwrap()), 0.25);
        assert_eq!(get(&bundle, &state, "Strength").unwrap(), vec![0.25]);
    }

    #[test]
    fn out_of_range_is_applied_with_a_warning() {
        let (bundle, state) = setup("range");
        let change = set(&bundle, &state, "Strength", &[3.0]).unwrap();
        assert_eq!(change.new, vec![3.0], "o efeito decide o que fazer com o valor");
        assert_eq!(change.warnings.len(), 1);
        assert!(change.warnings[0].contains("fora da faixa"));
    }

    #[test]
    fn reset_goes_back_to_the_defaults() {
        let (bundle, state) = setup("reset");
        set(&bundle, &state, "Strength", &[0.0]).unwrap();
        reset(&bundle, &state).unwrap();
        assert_eq!(get(&bundle, &state, "Strength").unwrap(), vec![1.0]);
    }

    #[test]
    fn unknown_and_runtime_driven_names_are_explained() {
        let (bundle, state) = setup("names");
        match set(&bundle, &state, "Forca", &[1.0]) {
            Err(Error::NotFound { available, .. }) => assert_eq!(available, vec!["Strength".to_string()]),
            other => panic!("{other:?}"),
        }
        // Sem o bloco publicado para outro efeito: o do Sépia só vale para o Sépia.
        assert_eq!(state.read_params(bundle.params_id() ^ 1, &mut [0u8; 4]), None);
    }

    #[test]
    fn runtime_driven_uniforms_are_listed_but_cannot_be_set() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../postfx/tests/data/Timer");
        let bundle = Bundle::load(&dir).expect("bundle Timer");
        let state_path = std::env::temp_dir().join(format!("lr-control-params-{}-timer", std::process::id())).join("state");
        let _ = std::fs::remove_dir_all(state_path.parent().unwrap());
        let state = ipc::Writer::create_at(&state_path).unwrap();

        let rows = describe(&bundle, &state);
        let timer = rows.iter().find(|r| r.name == "timer").expect("timer");
        assert_eq!(timer.source.as_deref(), Some("timer"));

        match set(&bundle, &state, "timer", &[5.0]) {
            Err(Error::RuntimeSource { source, .. }) => assert_eq!(source, "timer"),
            other => panic!("esperava RuntimeSource, veio {other:?}"),
        }
        assert_eq!(state.published_bundle(), 0, "nada foi publicado");
        // A mensagem de "não existe" só sugere o que é de fato um controle.
        match set(&bundle, &state, "nada", &[1.0]) {
            Err(Error::NotFound { available, .. }) => assert!(available.is_empty(), "{available:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn wrong_component_counts_are_refused_and_nothing_is_published() {
        let (bundle, state) = setup("count");
        assert!(matches!(set(&bundle, &state, "Strength", &[1.0, 2.0]), Err(Error::BadValue(_))));
        assert_eq!(state.published_bundle(), 0, "nada foi publicado");
    }

    #[test]
    fn numbers_print_the_way_people_write_them() {
        assert_eq!(fmt_number(f64::from(0.4f32)), "0.4");
        assert_eq!(fmt_number(f64::from(0.1f32)), "0.1");
        assert_eq!(fmt_number(8.0), "8");
        assert_eq!(fmt_number(-1.0), "-1");
        assert_eq!(fmt_number(0.0), "0");
        assert_eq!(fmt_number(4_000_000_000.0), "4000000000");
        assert_eq!(fmt_values(&[f64::from(0.1f32), 2.0]), "(0.1, 2)");
        assert_eq!(fmt_values(&[0.5]), "0.5");
    }

    #[test]
    fn values_are_parsed_from_numbers_words_and_lists() {
        let a = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_values(&a(&["0.5"])).unwrap(), vec![0.5]);
        assert_eq!(parse_values(&a(&["0.1,0.2", "0.3"])).unwrap(), vec![0.1, 0.2, 0.3]);
        assert_eq!(parse_values(&a(&["on", "off", "true"])).unwrap(), vec![1.0, 0.0, 1.0]);
        assert!(parse_values(&a(&["abc"])).is_err());
    }

    #[test]
    fn a_game_starts_from_the_global_values_and_then_diverges() {
        let (bundle, global) = setup("layered");
        let dir = std::env::temp_dir().join(format!("lr-control-params-{}-layered-own", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let own = ipc::ChannelWriter::create_in(&dir, "layer-1-1").unwrap();
        set(&bundle, &global, "Strength", &[0.25]).unwrap();

        let game = Layered { own: &own, global: &global };
        assert_eq!(get(&bundle, &game, "Strength").unwrap(), vec![0.25], "herda o valor global");
        set(&bundle, &game, "Strength", &[0.75]).unwrap();
        assert_eq!(get(&bundle, &game, "Strength").unwrap(), vec![0.75]);
        assert_eq!(get(&bundle, &global, "Strength").unwrap(), vec![0.25], "o global não mudou");

        reset(&bundle, &game).unwrap();
        assert_eq!(get(&bundle, &game, "Strength").unwrap(), vec![1.0], "restaurar no jogo volta ao padrão do efeito");
        assert_eq!(get(&bundle, &global, "Strength").unwrap(), vec![0.25]);
    }
}
