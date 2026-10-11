//! Uniforms alimentados pelo runtime (`< source = "timer"; >` e afins).
//!
//! Efeitos FX pedem valores como o tempo, o número do frame ou um número aleatório com a anotação
//! `source`. Sem alimentá-los o valor fica 0, e um efeito como o NightVision (que multiplica a
//! imagem por `smoothstep(.., timer)`) sai todo preto. Aqui estão os que não dependem de entrada do
//! usuário (e `overlay_active`/`overlay_open`, que o painel alimenta); os de mouse/teclado, `bufready_depth` e `screenshot` mantêm o valor padrão
//! do manifesto (zero), que é a resposta correta para "indisponível", e são listados em
//! [`Sources::ignored`] para a layer registrá-los.

use fx_bundle::manifest::{Manifest, Scalar, ScalarBase, Uniform, Value};

/// Valores do frame atual, calculados por quem usa o crate (a layer).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInfo {
    /// Milissegundos desde o início do jogo.
    pub time_ms: f32,
    /// Milissegundos que o frame anterior levou.
    pub frame_time_ms: f32,
    pub frame_count: u32,
    /// `[ano, mês (1-12), dia (1-31), segundos desde a meia-noite]`.
    pub date: [f32; 4],
}

/// Data e hora em UTC a partir de segundos Unix, no formato do `source = "date"`.
pub fn utc_date(unix_seconds: u64) -> [f32; 4] {
    let days = (unix_seconds / 86_400) as i64;
    let secs = (unix_seconds % 86_400) as f32;
    // Dias desde 1970-01-01 -> data civil (algoritmo de Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    [year as f32, month as f32, day as f32, secs]
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Timer,
    FrameTime,
    FrameCount,
    Date,
    Random { min: f32, max: f32 },
    PingPong { min: f32, max: f32, step_min: f32, step_max: f32 },
    /// O controle (de 1, na ordem do efeito) que a pessoa está editando; 0 se nenhum.
    OverlayActive,
    /// A interface de edição está aberta.
    OverlayOpen,
}

#[derive(Clone, Copy, Debug)]
struct Source {
    offset: usize,
    base: ScalarBase,
    rows: usize,
    kind: Kind,
}

/// Os uniforms com `source` de um efeito e o estado que alguns precisam entre frames.
#[derive(Default)]
pub struct Sources {
    list: Vec<Source>,
    /// Valor e direção de cada `pingpong`, na ordem em que aparecem em `list`.
    pingpong: Vec<[f32; 2]>,
    rng: u64,
    ignored: Vec<String>,
    /// Os uniforms editáveis (sem `source`), na ordem do efeito: a posição (de 1) é o valor de `overlay_active`.
    controls: Vec<String>,
    active: u32,
}

fn text<'a>(u: &'a Uniform, key: &str) -> Option<&'a str> {
    match u.annotations.get(key) {
        Some(Value::Scalar(Scalar::Text(t))) => Some(t),
        _ => None,
    }
}

/// Primeiro número de uma anotação numérica (escalar ou lista).
fn number(u: &Uniform, key: &str, default: f32) -> f32 {
    numbers(u, key).first().copied().unwrap_or(default)
}

fn numbers(u: &Uniform, key: &str) -> Vec<f32> {
    let as_f32 = |s: &Scalar| match s {
        Scalar::Number(n) => Some(*n as f32),
        _ => None,
    };
    match u.annotations.get(key) {
        Some(Value::Scalar(s)) => as_f32(s).into_iter().collect(),
        Some(Value::List(l)) => l.iter().filter_map(as_f32).collect(),
        None => Vec::new(),
    }
}

impl Sources {
    /// Procura no manifesto os uniforms com a anotação `source`.
    pub fn collect(manifest: &Manifest) -> Self {
        let mut s = Self { rng: 0x9E37_79B9_7F4A_7C15, ..Self::default() };
        s.controls = manifest.uniforms.iter().filter(|u| text(u, "source").is_none()).map(|u| u.name.clone()).collect();
        for u in &manifest.uniforms {
            let Some(name) = text(u, "source") else { continue };
            let kind = match name {
                "timer" => Kind::Timer,
                "frametime" => Kind::FrameTime,
                "framecount" => Kind::FrameCount,
                "date" => Kind::Date,
                "overlay_active" | "overlay_hovered" => Kind::OverlayActive,
                "overlay_open" => Kind::OverlayOpen,
                "random" => Kind::Random { min: number(u, "min", 0.0), max: number(u, "max", 1.0) },
                "pingpong" => {
                    let step = numbers(u, "step");
                    Kind::PingPong {
                        min: number(u, "min", 0.0),
                        max: number(u, "max", 1.0),
                        step_min: step.first().copied().unwrap_or(1.0),
                        step_max: step.get(1).copied().unwrap_or(0.0),
                    }
                }
                other => {
                    s.ignored.push(format!("{}: source = {other:?}", u.name));
                    continue;
                }
            };
            if matches!(kind, Kind::PingPong { .. }) {
                s.pingpong.push([match kind { Kind::PingPong { min, .. } => min, _ => 0.0 }, 1.0]);
            }
            s.list.push(Source { offset: u.offset as usize, base: u.ty.base, rows: u.ty.rows as usize, kind });
        }
        s
    }

    /// Diz qual controle está sendo editado (pelo nome do uniform); um nome que o efeito não tem vale como nenhum.
    pub fn set_active_control(&mut self, name: Option<&str>) {
        self.active = name.and_then(|n| self.controls.iter().position(|c| c == n)).map_or(0, |i| i as u32 + 1);
    }

    /// Uniforms cujo `source` não é implementado (ficam com o valor padrão do manifesto).
    pub fn ignored(&self) -> &[String] {
        &self.ignored
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// xorshift64*: barato, sem dependências; a qualidade basta para ruído e dither.
    fn next_unit(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let r = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((r >> 40) as f32) / ((1u64 << 24) as f32) // [0, 1)
    }

    /// Calcula os valores do frame e devolve `(deslocamento, bytes)` para escrever no buffer de uniforms.
    pub fn evaluate(&mut self, frame: &FrameInfo) -> Vec<(usize, Vec<u8>)> {
        // Deixa o gerador diferente a cada frame mesmo se o chamador não pedir nenhum `random`.
        self.rng ^= u64::from(frame.frame_count).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut writes = Vec::with_capacity(self.list.len());
        let mut pingpong_index = 0;
        for i in 0..self.list.len() {
            let src = self.list[i];
            let values: Vec<f32> = match src.kind {
                Kind::Timer => vec![frame.time_ms],
                Kind::FrameTime => vec![frame.frame_time_ms],
                Kind::FrameCount => vec![frame.frame_count as f32],
                Kind::Date => frame.date.to_vec(),
                Kind::OverlayActive => vec![self.active as f32],
                Kind::OverlayOpen => vec![f32::from(self.active != 0)],
                Kind::Random { min, max } => {
                    let r = self.next_unit();
                    // int: inclusivo nos dois extremos, como o `rand() % (max-min+1)` do ReShade.
                    vec![if src.base == ScalarBase::Float { min + r * (max - min) } else { min + (r * (max - min + 1.0)).floor() }]
                }
                Kind::PingPong { min, max, step_min, step_max } => {
                    let inc_per_second = if step_max == 0.0 { step_min } else { step_min + self.next_unit() * (step_max - step_min) };
                    let state = &mut self.pingpong[pingpong_index];
                    pingpong_index += 1;
                    let seconds = frame.frame_time_ms / 1000.0;
                    state[0] += state[1] * inc_per_second * seconds;
                    if state[0] >= max {
                        state[0] = max;
                        state[1] = -1.0;
                    } else if state[0] <= min {
                        state[0] = min;
                        state[1] = 1.0;
                    }
                    vec![state[0], state[1]]
                }
            };
            let mut bytes = Vec::with_capacity(values.len() * 4);
            for v in values.iter().take(src.rows.max(1)) {
                match src.base {
                    ScalarBase::Int => bytes.extend_from_slice(&(*v as i32).to_le_bytes()),
                    ScalarBase::Uint => bytes.extend_from_slice(&(*v as u32).to_le_bytes()),
                    ScalarBase::Bool => bytes.extend_from_slice(&u32::from(*v != 0.0).to_le_bytes()),
                    _ => bytes.extend_from_slice(&v.to_le_bytes()),
                }
            }
            writes.push((src.offset, bytes));
        }
        writes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(uniforms: &[(&str, &str, u32, &str)]) -> Manifest {
        // (nome, tipo base, linhas, anotações em JSON)
        let items: Vec<String> = uniforms
            .iter()
            .enumerate()
            .map(|(i, (name, base, rows, ann))| {
                format!(
                    r#"{{"name":"{name}","unique_name":"{name}","type":{{"base":"{base}","rows":{rows},"cols":1,"array":0}},
                    "offset":{},"size":{},"default":null,"annotations":{ann}}}"#,
                    i * 16,
                    rows * 4
                )
            })
            .collect();
        let json = format!(
            r#"{{"format":"linux-reshade-bundle","version":1,"source":"t.fx",
            "compiler":{{"name":"x","reshade_commit":"x","spirv_headers_commit":"x"}},
            "target":{{"width":8,"height":8,"color_space":"srgb"}},"spirv_version":"1.0",
            "entry_points":[],"total_uniform_size":128,"uniforms":[{}],"techniques":[]}}"#,
            items.join(",")
        );
        Manifest::parse(&json).expect("manifesto de teste")
    }

    fn f32_at(w: &[(usize, Vec<u8>)], i: usize, c: usize) -> f32 {
        f32::from_le_bytes(w[i].1[c * 4..c * 4 + 4].try_into().unwrap())
    }

    #[test]
    fn timer_frametime_and_framecount_follow_the_frame() {
        let m = manifest(&[
            ("t", "float", 1, r#"{"source":"timer"}"#),
            ("ft", "float", 1, r#"{"source":"frametime"}"#),
            ("fc", "uint", 1, r#"{"source":"framecount"}"#),
            ("normal", "float", 1, r#"{"ui_type":"slider"}"#),
        ]);
        let mut s = Sources::collect(&m);
        assert_eq!(s.list.len(), 3, "só os uniforms com source");
        let w = s.evaluate(&FrameInfo { time_ms: 1234.5, frame_time_ms: 16.7, frame_count: 42, date: [0.0; 4] });
        assert_eq!((w[0].0, f32_at(&w, 0, 0)), (0, 1234.5));
        assert_eq!((w[1].0, f32_at(&w, 1, 0)), (16, 16.7));
        assert_eq!((w[2].0, u32::from_le_bytes(w[2].1[..4].try_into().unwrap())), (32, 42));
    }

    #[test]
    fn overlay_active_is_the_one_based_position_of_the_edited_control() {
        let m = manifest(&[
            ("mode", "int", 1, r#"{"ui_type":"combo"}"#),
            ("range", "float", 1, r#"{"ui_type":"slider"}"#),
            ("active", "int", 1, r#"{"source":"overlay_active"}"#),
            ("open", "bool", 1, r#"{"source":"overlay_open"}"#),
        ]);
        let mut s = Sources::collect(&m);
        let read = |s: &mut Sources| s.evaluate(&FrameInfo::default()).iter().map(|(_, b)| u32::from_le_bytes(b[..4].try_into().unwrap())).collect::<Vec<_>>();
        assert_eq!(read(&mut s), [0, 0], "ninguém editando");
        s.set_active_control(Some("range"));
        assert_eq!(read(&mut s), [2, 1], "o segundo controle, e a edição está aberta");
        s.set_active_control(Some("outro_efeito"));
        assert_eq!(read(&mut s), [0, 0], "um nome que o efeito não tem vale como nenhum");
    }

    #[test]
    fn date_has_four_components() {
        let m = manifest(&[("d", "float", 4, r#"{"source":"date"}"#)]);
        let mut s = Sources::collect(&m);
        let w = s.evaluate(&FrameInfo { date: [2026.0, 10.0, 9.0, 43200.0], ..FrameInfo::default() });
        assert_eq!(w[0].1.len(), 16);
        assert_eq!([f32_at(&w, 0, 0), f32_at(&w, 0, 1), f32_at(&w, 0, 2), f32_at(&w, 0, 3)], [2026.0, 10.0, 9.0, 43200.0]);
    }

    #[test]
    fn utc_date_matches_known_instants() {
        assert_eq!(utc_date(0), [1970.0, 1.0, 1.0, 0.0]);
        // 2026-10-09 12:00:00 UTC
        assert_eq!(utc_date(1_791_547_200), [2026.0, 10.0, 9.0, 43_200.0]);
        // 2024-02-29 (ano bissexto) 23:59:59 UTC
        assert_eq!(utc_date(1_709_251_199), [2024.0, 2.0, 29.0, 86_399.0]);
    }

    #[test]
    fn random_stays_inside_its_range_and_varies() {
        let m = manifest(&[
            ("i", "int", 1, r#"{"source":"random","min":0,"max":9}"#),
            ("f", "float", 1, r#"{"source":"random","min":-1.0,"max":1.0}"#),
        ]);
        let mut s = Sources::collect(&m);
        let (mut ints, mut floats) = (std::collections::HashSet::new(), std::collections::HashSet::new());
        for n in 0..500 {
            let w = s.evaluate(&FrameInfo { frame_count: n, ..FrameInfo::default() });
            let i = i32::from_le_bytes(w[0].1[..4].try_into().unwrap());
            let f = f32_at(&w, 1, 0);
            assert!((0..=9).contains(&i), "int fora da faixa: {i}");
            assert!((-1.0..1.0).contains(&f), "float fora da faixa: {f}");
            ints.insert(i);
            floats.insert(f.to_bits());
        }
        assert_eq!(ints.len(), 10, "todos os inteiros da faixa aparecem");
        assert!(floats.len() > 400, "o float varia a cada frame");
    }

    #[test]
    fn pingpong_bounces_between_min_and_max() {
        let m = manifest(&[("p", "float", 2, r#"{"source":"pingpong","min":0.0,"max":10.0,"step":2.0}"#)]);
        let mut s = Sources::collect(&m);
        let frame = FrameInfo { frame_time_ms: 1000.0, ..FrameInfo::default() }; // 1 s por frame
        let mut seen = Vec::new();
        for n in 0..12 {
            let w = s.evaluate(&FrameInfo { frame_count: n, ..frame });
            seen.push((f32_at(&w, 0, 0), f32_at(&w, 0, 1)));
        }
        // sobe 2 por segundo até 10 (5 s), desce até 0 (5 s), sobe de novo.
        assert_eq!(seen[0], (2.0, 1.0));
        assert_eq!(seen[4], (10.0, -1.0), "bate no máximo e inverte: {seen:?}");
        assert_eq!(seen[9], (0.0, 1.0), "bate no mínimo e inverte: {seen:?}");
        assert!(seen.iter().all(|(v, d)| (0.0..=10.0).contains(v) && d.abs() == 1.0));
    }

    #[test]
    fn unsupported_sources_are_reported_and_left_alone() {
        let m = manifest(&[
            ("mouse", "float", 2, r#"{"source":"mousepoint"}"#),
            ("depth", "bool", 1, r#"{"source":"bufready_depth"}"#),
        ]);
        let s = Sources::collect(&m);
        assert!(s.is_empty());
        assert_eq!(s.ignored().len(), 2);
        assert!(s.ignored()[0].contains("mousepoint"));
    }
}
