//! Texturas compartilhadas entre os efeitos de uma cadeia, como no ReShade: duas declarações de uma textura com o mesmo
//! nome e as mesmas propriedades (tipo, tamanho, formato, níveis) são a mesma imagem. É assim que o `LAUNCHPAD` do Marty
//! entrega normais, movimento e profundidade linear aos outros efeitos dele (`MXAO`, `RTGI`...).

use crate::chain::Step;
use crate::image::GpuImage;
use fx_bundle::manifest::{Texture, TextureFormat, TextureType};
use std::collections::HashMap;

/// As propriedades que precisam coincidir para duas declarações serem a mesma textura.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Desc {
    kind: TextureType,
    width: u32,
    height: u32,
    depth: u32,
    levels: u32,
    format: TextureFormat,
}

impl Desc {
    pub fn of(t: &Texture) -> Self {
        Self { kind: t.kind, width: t.width, height: t.height, depth: t.depth, levels: t.levels, format: t.format }
    }
}

/// Os usos que alguma etapa da cadeia faz da textura: a imagem compartilhada precisa servir a todos.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Uses {
    pub render_target: bool,
    pub stored: bool,
}

#[derive(Default)]
pub struct Shared {
    entries: HashMap<String, (Desc, GpuImage)>,
    uses: HashMap<String, Uses>,
}

impl Shared {
    /// Olha todas as etapas e anota, por nome de textura, se alguma a usa como alvo de renderização ou de armazenamento
    /// (a imagem criada pela primeira etapa já precisa ter esses usos para a etapa seguinte poder escrevê-la).
    pub fn scan(steps: &[Step<'_>]) -> Self {
        let mut uses: HashMap<String, Uses> = HashMap::new();
        for step in steps {
            let m = &step.bundle.manifest;
            let Some(tech) = m.techniques.get(step.technique) else { continue };
            for pass in &tech.passes {
                for name in pass.render_targets.iter().flatten() {
                    uses.entry(name.clone()).or_default().render_target = true;
                }
                for b in &pass.storages {
                    if let Some(st) = m.storages.get(b.storage as usize) {
                        uses.entry(st.texture.clone()).or_default().stored = true;
                    }
                }
            }
        }
        Self { entries: HashMap::new(), uses }
    }

    pub(crate) fn uses(&self, name: &str) -> Uses {
        self.uses.get(name).copied().unwrap_or_default()
    }

    /// A imagem já criada por outra etapa com este nome e estas propriedades, se houver.
    pub(crate) fn find(&self, name: &str, desc: &Desc) -> Option<GpuImage> {
        self.entries.get(name).filter(|(d, _)| d == desc).map(|(_, image)| image.alias())
    }

    /// Guarda a imagem recém-criada (a cadeia passa a ser a dona) e devolve o apelido que o efeito usa.
    pub(crate) fn insert(&mut self, name: &str, desc: Desc, image: GpuImage) -> GpuImage {
        let alias = image.alias();
        self.entries.insert(name.to_string(), (desc, image));
        alias
    }

    /// A imagem guardada sob este nome (diagnóstico).
    pub(crate) fn image(&self, name: &str) -> Option<ash::vk::Image> {
        self.entries.get(name).map(|(_, image)| image.image)
    }

    /// # Safety
    /// A GPU não pode estar usando as imagens, e os efeitos que as apelidam já foram destruídos.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for (_, (_, mut image)) in self.entries.drain() {
            unsafe { image.destroy(device) };
        }
    }
}
