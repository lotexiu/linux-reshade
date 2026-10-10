//! Vários efeitos em ordem sobre o mesmo alvo (a cadeia de técnicas de um preset).
//!
//! Cada [`Effect`] já processa a imagem no lugar: copia o frame, desenha o resultado de volta e deixa a
//! imagem no layout de repouso. Então a etapa N lê o que a N-1 escreveu e a primeira lê o frame do jogo,
//! bastando submeter os command buffers na ordem, num único `vkQueueSubmit`. É o que o ReShade faz (copia o
//! back buffer antes de um passe que o lê). O custo é uma cópia do frame por etapa; economizá-la (revezar
//! duas imagens) fica para depois da medição.

use crate::effect::{Effect, FrameInfo, Target};
use crate::error::Error;
use ash::vk;
use fx_bundle::Bundle;

/// Uma etapa: uma técnica de um bundle.
#[derive(Clone, Copy)]
pub struct Step<'a> {
    pub bundle: &'a Bundle,
    pub technique: usize,
}

impl<'a> Step<'a> {
    /// A técnica pelo nome (`Sepia` em `Sepia@Sepia.fx`). `None` se o bundle não a tem.
    pub fn named(bundle: &'a Bundle, technique: &str) -> Option<Self> {
        let technique = technique_index(bundle, technique)?;
        Some(Self { bundle, technique })
    }
}

/// Posição de uma técnica no bundle, pelo nome.
pub fn technique_index(bundle: &Bundle, name: &str) -> Option<usize> {
    bundle.manifest.techniques.iter().position(|t| t.name == name)
}

#[derive(Default)]
pub struct Chain {
    effects: Vec<Effect>,
}

impl Chain {
    /// Cria a cadeia na ordem dada (a primeira etapa roda primeiro). Se alguma etapa falhar, as já criadas
    /// são liberadas e o erro diz qual foi (`etapa N`). Cadeia vazia é válida: não faz nada.
    ///
    /// # Safety
    /// As mesmas de [`Effect::new`].
    pub unsafe fn new(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        steps: &[Step<'_>],
        target: &Target<'_>,
        queue: vk::Queue,
    ) -> Result<Self, Error> {
        let mut chain = Self::default();
        for (n, step) in steps.iter().enumerate() {
            match unsafe { Effect::new(device, memory, step.bundle, step.technique, target, queue) } {
                Ok(effect) => chain.effects.push(effect),
                Err(e) => {
                    unsafe { chain.destroy(device) };
                    return Err(match e {
                        Error::Unsupported(why) => Error::Unsupported(format!("etapa {n}: {why}")),
                        Error::Invalid(why) => Error::Invalid(format!("etapa {n}: {why}")),
                        other => other,
                    });
                }
            }
        }
        Ok(chain)
    }

    pub fn len(&self) -> usize {
        self.effects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// Os command buffers que processam a imagem `index`, na ordem de execução. Submeta todos de uma vez
    /// (um único `VkSubmitInfo`), esperando o semáforo do renderizador em `ALL_COMMANDS`.
    pub fn command_buffers(&self, index: usize) -> Vec<vk::CommandBuffer> {
        self.effects.iter().map(|e| e.command_buffer(index)).collect()
    }

    /// Escreve nos uniforms da etapa `step` (ver [`Effect::write_uniforms`]). Etapa inexistente é ignorada.
    ///
    /// # Safety
    /// A GPU não deve estar executando os command buffers dessa imagem.
    pub unsafe fn write_uniforms(&self, step: usize, index: usize, offset: usize, bytes: &[u8]) {
        if let Some(effect) = self.effects.get(step) {
            unsafe { effect.write_uniforms(index, offset, bytes) };
        }
    }

    /// Alimenta, em todas as etapas, os uniforms vindos do runtime (tempo, frame...).
    ///
    /// # Safety
    /// A GPU não deve estar executando os command buffers dessa imagem.
    pub unsafe fn update_sources(&mut self, index: usize, frame: &FrameInfo) {
        for effect in &mut self.effects {
            unsafe { effect.update_sources(index, frame) };
        }
    }

    /// Uniforms sem fonte no runtime, por etapa (para o log).
    pub fn ignored_sources(&self) -> Vec<(usize, &str)> {
        self.effects.iter().enumerate().flat_map(|(n, e)| e.ignored_sources().iter().map(move |s| (n, s.as_str()))).collect()
    }

    /// # Safety
    /// A GPU não pode estar executando os command buffers da cadeia.
    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        for effect in &mut self.effects {
            unsafe { effect.destroy(device) };
        }
        self.effects.clear();
    }
}
