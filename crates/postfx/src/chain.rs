//! Vários efeitos em ordem sobre o mesmo alvo (a cadeia de técnicas de um preset).
//!
//! Cada [`Effect`] já processa a imagem no lugar: copia o frame, desenha o resultado de volta e deixa a
//! imagem no layout de repouso. Então a etapa N lê o que a N-1 escreveu e a primeira lê o frame do jogo,
//! bastando submeter os command buffers na ordem, num único `vkQueueSubmit`. É o que o ReShade faz (copia o
//! back buffer antes de um passe que o lê). O custo é uma cópia do frame por etapa; economizá-la (revezar
//! duas imagens) fica para depois da medição.

use crate::effect::{Effect, FrameInfo, Target};
use crate::error::Error;
use crate::shared::Shared;
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

/// Uma etapa que a cadeia pulou por usar algo que o runtime ainda não faz.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// Posição da etapa na lista dada a [`Chain::new_lenient`].
    pub step: usize,
    pub why: String,
}

#[derive(Default)]
pub struct Chain {
    effects: Vec<Effect>,
    /// As texturas que mais de uma etapa pode usar (a cadeia é a dona das imagens).
    shared: Shared,
    /// De que etapa da lista original vem cada efeito (difere da posição se alguma foi pulada).
    origin: Vec<usize>,
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
        let mut chain = Self { shared: Shared::scan(steps), ..Self::default() };
        for (n, step) in steps.iter().enumerate() {
            match unsafe { Effect::new_shared(device, memory, step.bundle, step.technique, target, Some(queue), Some(&mut chain.shared)) } {
                Ok(effect) => {
                    chain.effects.push(effect);
                    chain.origin.push(n);
                }
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

    /// Como [`Chain::new`], mas uma etapa que o runtime ainda não suporta (`Error::Unsupported`) é pulada e
    /// contada em vez de derrubar a cadeia inteira: o resto do preset continua valendo. Qualquer outro erro
    /// (falha do Vulkan, bundle inválido) segue sendo erro.
    ///
    /// # Safety
    /// As mesmas de [`Effect::new`].
    pub unsafe fn new_lenient(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        steps: &[Step<'_>],
        target: &Target<'_>,
        queue: vk::Queue,
    ) -> Result<(Self, Vec<Skipped>), Error> {
        unsafe { Self::new_lenient_with(device, memory, steps, target, Some(queue)) }
    }

    /// Como [`Chain::new_lenient`], sem usar fila (`queue` `None`): serve para criar a cadeia numa thread própria, sem mexer na
    /// fila do jogo; depois, na thread que usa a fila, [`Chain::finish_init`] termina a inicialização das texturas.
    ///
    /// # Safety
    /// As mesmas de [`Effect::new`].
    pub unsafe fn new_lenient_with(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        steps: &[Step<'_>],
        target: &Target<'_>,
        queue: Option<vk::Queue>,
    ) -> Result<(Self, Vec<Skipped>), Error> {
        let mut chain = Self { shared: Shared::scan(steps), ..Self::default() };
        let mut skipped = Vec::new();
        for (n, step) in steps.iter().enumerate() {
            match unsafe { Effect::new_shared(device, memory, step.bundle, step.technique, target, queue, Some(&mut chain.shared)) } {
                Ok(effect) => {
                    chain.effects.push(effect);
                    chain.origin.push(n);
                }
                Err(Error::Unsupported(why)) => skipped.push(Skipped { step: n, why }),
                Err(e) => {
                    unsafe { chain.destroy(device) };
                    return Err(e);
                }
            }
        }
        Ok((chain, skipped))
    }

    /// Termina a inicialização das texturas das etapas criadas sem fila (ver [`Chain::new_lenient_with`]).
    ///
    /// # Safety
    /// `queue` da família do alvo, sem outro uso concorrente durante a chamada; a GPU não executa esta cadeia.
    pub unsafe fn finish_init(&mut self, device: &ash::Device, queue: vk::Queue) -> Result<(), Error> {
        for effect in &mut self.effects {
            unsafe { effect.finish_init(device, queue) }?;
        }
        Ok(())
    }

    /// De qual etapa da lista original vem o efeito `position` da cadeia.
    pub fn origin(&self, position: usize) -> Option<usize> {
        self.origin.get(position).copied()
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

    /// A imagem de uma textura compartilhada pelo nome (`unique_name` do manifesto): para diagnóstico.
    pub fn shared_image(&self, name: &str) -> Option<vk::Image> {
        self.shared.image(name)
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
        self.origin.clear();
        // Depois dos efeitos: eles apelidam as imagens compartilhadas.
        unsafe { self.shared.destroy(device) };
    }
}
