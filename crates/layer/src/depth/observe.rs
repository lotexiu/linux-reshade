//! A parte pura do observador de profundidade (sem Vulkan): estatísticas por imagem de profundidade num quadro e
//! a escolha da candidata. Testada sem GPU.
//!
//! A heurística de escolha segue a do add-on de profundidade do ReShade (descarta as sem desenhos, as de proporção
//! diferente da tela e as multiamostradas; entre as que sobram, a de mais vértices, ou de mais chamadas quando há
//! muitos desenhos indiretos), com uma diferença: aqui os dados vêm de passes de renderização completos, com o que
//! cada passe faz com a profundidade ao começar e ao terminar.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoadOp {
    #[default]
    Load,
    Clear,
    DontCare,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StoreOp {
    #[default]
    Store,
    DontCare,
    None,
}

/// O que se sabe de uma imagem de profundidade desde a criação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    /// `VkFormat` como número.
    pub format: i32,
    pub samples: u32,
}

/// Um passe de renderização gravado que usa uma imagem de profundidade.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PassRecord {
    pub image: u64,
    pub draws: u32,
    pub vertices: u64,
    /// Desenhos indiretos e execuções de command buffers secundários (não dá para contar os vértices).
    pub indirect: u32,
    pub load: LoadOp,
    pub store: StoreOp,
    /// Valor de limpeza da profundidade, se o passe a limpa.
    pub clear_depth: Option<f32>,
    pub area: (u32, u32),
    /// Desenhos feitos com comparação de profundidade "maior" (profundidade invertida) e "menor".
    pub greater: u32,
    pub less: u32,
}

/// Totais de uma imagem de profundidade num quadro.
#[derive(Debug, Clone, PartialEq, Default)]
struct Aggregate {
    draws: u32,
    vertices: u64,
    indirect: u32,
    passes: u32,
    /// Posição (dentro do quadro) do primeiro e do último passe que a usou.
    first_pass: u32,
    last_pass: u32,
    last: PassRecord,
    greater: u32,
    less: u32,
    /// O primeiro valor com que um passe limpou a profundidade neste quadro (0 = quase sempre profundidade invertida).
    first_clear: Option<f32>,
}

/// Uma candidata a "profundidade da cena", como se publica para o app de controle.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub width: u32,
    pub height: u32,
    pub format: i32,
    pub samples: u32,
    pub draws: u32,
    pub vertices: u64,
    pub passes: u32,
    pub load: LoadOp,
    pub store: StoreOp,
    /// `Some(true)` se a maioria dos desenhos compara "maior" (profundidade invertida), `Some(false)` se "menor",
    /// `None` se não deu para saber.
    pub reversed: Option<bool>,
    /// Posição do último passe dentro do quadro (onde uma cópia deveria acontecer).
    pub last_pass: u32,
    pub chosen: bool,
    /// Por que não serve, se não serve (MSAA, proporção...). Vazio: serve.
    pub unusable: Option<&'static str>,
}

#[derive(Debug, Default)]
pub struct FrameStats {
    images: HashMap<u64, Aggregate>,
    infos: HashMap<u64, ImageInfo>,
    passes_seen: u32,
}

impl FrameStats {
    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    /// Soma os passes de um command buffer enviado à fila, na ordem em que foram gravados.
    pub fn merge(&mut self, passes: &[PassRecord], infos: &HashMap<u64, ImageInfo>) {
        for p in passes {
            let Some(info) = infos.get(&p.image) else { continue };
            self.infos.insert(p.image, *info);
            let ordinal = self.passes_seen;
            self.passes_seen += 1;
            let a = self.images.entry(p.image).or_insert_with(|| Aggregate { first_pass: ordinal, ..Aggregate::default() });
            a.draws += p.draws;
            a.vertices += p.vertices;
            a.indirect += p.indirect;
            a.passes += 1;
            a.last_pass = ordinal;
            a.last = *p;
            a.greater += p.greater;
            a.less += p.less;
            if a.first_clear.is_none() {
                a.first_clear = p.clear_depth;
            }
        }
    }

    /// As candidatas do quadro e qual foi escolhida para uma tela de `frame` (largura, altura).
    pub fn candidates(&self, frame: (u32, u32)) -> Vec<Candidate> {
        let mut out: Vec<(u64, Candidate)> = self
            .images
            .iter()
            .filter_map(|(image, a)| {
                let info = self.infos.get(image)?;
                let unusable = if a.draws == 0 || (a.vertices <= 3 && a.indirect == 0) {
                    Some("sem desenhos")
                } else if info.samples > 1 {
                    Some("MSAA (precisa de resolve de profundidade)")
                } else if !similar_aspect(info.width, info.height, frame.0, frame.1) {
                    Some("proporção diferente da tela")
                } else {
                    None
                };
                // A comparação dos pipelines manda; sem ela (pipelines que não declaram o teste, ou o declaram em estado
                // dinâmico), o valor de limpeza: 0 indica profundidade invertida, 1 a normal.
                let reversed = match (a.greater, a.less) {
                    (0, 0) => match a.first_clear {
                        Some(v) if v <= 0.01 => Some(true),
                        Some(v) if v >= 0.99 => Some(false),
                        _ => None,
                    },
                    (g, l) => Some(g > l),
                };
                Some((
                    *image,
                    Candidate {
                        width: info.width,
                        height: info.height,
                        format: info.format,
                        samples: info.samples,
                        draws: a.draws,
                        vertices: a.vertices,
                        passes: a.passes,
                        load: a.last.load,
                        store: a.last.store,
                        reversed,
                        last_pass: a.last_pass,
                        chosen: false,
                        unusable,
                    },
                ))
            })
            .collect();
        // Ordem estável: pela posição do primeiro passe (a mesma de um quadro para o outro).
        out.sort_by_key(|(image, _)| (self.images[image].first_pass, *image));
        let mut best: Option<usize> = None;
        for (i, (image, c)) in out.iter().enumerate() {
            if c.unusable.is_some() {
                continue;
            }
            let better = match best {
                None => true,
                Some(b) => self.better(*image, out[b].0),
            };
            if better {
                best = Some(i);
            }
        }
        if let Some(b) = best {
            out[b].1.chosen = true;
        }
        out.into_iter().map(|(_, c)| c).collect()
    }

    /// `a` é melhor que `b`? Se algum dos dois tem muitos desenhos indiretos (os vértices deles não aparecem), compara
    /// chamadas; senão, vértices. O ReShade olha só para o candidato testado, e então o resultado depende da ordem;
    /// aqui a regra é simétrica.
    fn better(&self, a: u64, b: u64) -> bool {
        let (a, b) = (&self.images[&a], &self.images[&b]);
        let indirect_heavy = |x: &Aggregate| x.indirect >= x.draws / 3;
        if indirect_heavy(a) || indirect_heavy(b) { a.draws > b.draws } else { a.vertices > b.vertices }
    }
}

/// A proporção da imagem se parece com a da tela? (A mesma regra do ReShade: razão largura/altura até 0,1 de
/// diferença e razões de tamanho entre 0,5 e 1,85.) Sem tamanho conhecido, aceita.
pub fn similar_aspect(width: u32, height: u32, frame_width: u32, frame_height: u32) -> bool {
    if width == 0 || height == 0 || frame_width == 0 || frame_height == 0 {
        return true;
    }
    let (w, h, fw, fh) = (width as f32, height as f32, frame_width as f32, frame_height as f32);
    let (w_ratio, h_ratio) = (fw / w, fh / h);
    ((fw / fh) - (w / h)).abs() <= 0.1 && (0.5..=1.85).contains(&w_ratio) && (0.5..=1.85).contains(&h_ratio)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(w: u32, h: u32, samples: u32) -> ImageInfo {
        ImageInfo { width: w, height: h, format: 126, samples }
    }

    fn pass(image: u64, draws: u32, vertices: u64) -> PassRecord {
        PassRecord { image, draws, vertices, area: (1920, 1080), ..PassRecord::default() }
    }

    fn infos(list: &[(u64, ImageInfo)]) -> HashMap<u64, ImageInfo> {
        list.iter().copied().collect()
    }

    #[test]
    fn the_scene_depth_is_the_one_with_the_most_vertices_at_screen_proportion() {
        let i = infos(&[(1, info(1920, 1080, 1)), (2, info(2048, 2048, 1)), (3, info(1920, 1080, 1))]);
        let mut s = FrameStats::default();
        // 2 é um mapa de sombras (quadrado, muitos vértices): proporção errada. 3 é a cena. 1 é um passe pequeno.
        s.merge(&[pass(2, 100, 900_000), pass(3, 300, 500_000), pass(1, 5, 30)], &i);
        let c = s.candidates((1920, 1080));
        assert_eq!(c.len(), 3);
        let chosen: Vec<_> = c.iter().filter(|c| c.chosen).collect();
        assert_eq!(chosen.len(), 1);
        assert_eq!((chosen[0].width, chosen[0].vertices), (1920, 500_000));
        let shadow = c.iter().find(|c| c.width == 2048).unwrap();
        assert_eq!(shadow.unusable, Some("proporção diferente da tela"));
    }

    #[test]
    fn unusable_candidates_say_why_and_nothing_is_chosen_without_a_good_one() {
        let i = infos(&[(1, info(1920, 1080, 4)), (2, info(1920, 1080, 1))]);
        let mut s = FrameStats::default();
        s.merge(&[pass(1, 200, 100_000), pass(2, 0, 0)], &i);
        let c = s.candidates((1920, 1080));
        assert_eq!(c.iter().find(|c| c.samples == 4).unwrap().unusable, Some("MSAA (precisa de resolve de profundidade)"));
        assert_eq!(c.iter().find(|c| c.draws == 0).unwrap().unusable, Some("sem desenhos"));
        assert!(c.iter().all(|c| !c.chosen));
    }

    #[test]
    fn many_indirect_draws_compare_draw_calls_instead_of_vertices() {
        let i = infos(&[(1, info(1920, 1080, 1)), (2, info(1920, 1080, 1))]);
        let mut s = FrameStats::default();
        // 1: poucos vértices contáveis mas muitas chamadas, quase todas indiretas; 2: mais vértices, menos chamadas.
        s.merge(&[PassRecord { indirect: 90, ..pass(1, 100, 1000) }, pass(2, 40, 50_000)], &i);
        let c = s.candidates((1920, 1080));
        assert!(c.iter().find(|c| c.draws == 100).unwrap().chosen, "{c:?}");
    }

    #[test]
    fn passes_of_the_same_image_add_up_and_remember_the_last_one() {
        let i = infos(&[(7, info(1280, 720, 1))]);
        let mut s = FrameStats::default();
        let first = PassRecord { load: LoadOp::Clear, store: StoreOp::Store, clear_depth: Some(0.0), greater: 10, ..pass(7, 10, 1000) };
        let second = PassRecord { load: LoadOp::Load, store: StoreOp::DontCare, greater: 5, less: 1, ..pass(7, 5, 500) };
        s.merge(&[first], &i);
        s.merge(&[second], &i);
        let c = &s.candidates((1280, 720))[0];
        assert_eq!((c.draws, c.vertices, c.passes), (15, 1500, 2));
        assert_eq!((c.load, c.store), (LoadOp::Load, StoreOp::DontCare), "o último passe manda no que se mostra");
        assert_eq!((c.last_pass, c.reversed), (1, Some(true)), "15 de comparação 'maior' contra 1 de 'menor'");
    }

    #[test]
    fn without_pipeline_votes_the_clear_value_tells_if_the_depth_is_reversed() {
        let i = infos(&[(1, info(1280, 720, 1)), (2, info(1280, 720, 1)), (3, info(1280, 720, 1))]);
        let mut s = FrameStats::default();
        s.merge(&[PassRecord { clear_depth: Some(0.0), load: LoadOp::Clear, ..pass(1, 5, 500) }], &i);
        s.merge(&[PassRecord { clear_depth: Some(1.0), load: LoadOp::Clear, ..pass(2, 5, 500) }], &i);
        s.merge(&[PassRecord { clear_depth: Some(0.5), load: LoadOp::Clear, ..pass(3, 5, 500) }], &i);
        let all: Vec<_> = s.candidates((1280, 720)).into_iter().map(|c| c.reversed).collect();
        assert_eq!(all, [Some(true), Some(false), None]);
    }

    #[test]
    fn pipeline_votes_beat_the_clear_value() {
        let i = infos(&[(1, info(1280, 720, 1))]);
        let mut s = FrameStats::default();
        s.merge(&[PassRecord { clear_depth: Some(1.0), load: LoadOp::Clear, less: 1, greater: 9, ..pass(1, 10, 1000) }], &i);
        assert_eq!(s.candidates((1280, 720))[0].reversed, Some(true));
    }

    #[test]
    fn an_image_the_layer_never_saw_created_is_ignored_and_the_order_is_stable() {
        let i = infos(&[(1, info(100, 100, 1)), (2, info(100, 100, 1))]);
        let mut s = FrameStats::default();
        s.merge(&[pass(9, 5, 5000), pass(2, 5, 5000), pass(1, 5, 5000)], &i);
        let c = s.candidates((100, 100));
        assert_eq!(c.len(), 2, "a imagem 9 não tem informação");
        assert!(s.images.contains_key(&2) && !s.images.contains_key(&9));
    }

    #[test]
    fn the_aspect_rule_matches_the_reshade_one() {
        assert!(similar_aspect(1920, 1080, 1920, 1080));
        assert!(similar_aspect(1280, 720, 1920, 1080), "escala de 1,5");
        assert!(similar_aspect(3840, 2160, 1920, 1080), "razão 0,5 no limite");
        assert!(!similar_aspect(2048, 2048, 1920, 1080), "quadrada");
        assert!(!similar_aspect(512, 288, 1920, 1080), "mesma proporção, mas pequena demais");
        assert!(similar_aspect(0, 0, 1920, 1080), "sem tamanho conhecido");
    }
}
