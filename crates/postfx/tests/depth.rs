//! A profundidade do jogo entra como textura `DEPTH`: o efeito amostra a imagem original (sem cópia) e ela volta ao
//! layout em que estava. Sem profundidade, o efeito é recusado com o motivo.

mod common;

use ash::vk;
use common::*;
use fx_bundle::Bundle;
use postfx::effect::{DepthInput, Target};
use postfx::{Effect, Error};
use std::path::Path;

macro_rules! gpu_or_skip {
    () => {
        match Gpu::software() {
            Some(g) => g,
            None => {
                eprintln!("sem dispositivo Vulkan de software (lavapipe); teste ignorado");
                return;
            }
        }
    };
}

fn bundle() -> Bundle {
    Bundle::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/Depth")).expect("carregar o bundle de teste")
}

fn target(images: &[vk::Image], gpu: &Gpu, depth: Option<DepthInput>) -> Target<'static> {
    let images: &'static [vk::Image] = Box::leak(images.to_vec().into_boxed_slice());
    Target { images, format: FORMAT, extent: vk::Extent2D { width: W, height: H }, rest_layout: REST, queue_family: gpu.family, init_command_buffer: None, depth }
}

#[test]
fn the_effect_samples_the_games_own_depth_image_and_leaves_its_layout_as_it_was() {
    let gpu = gpu_or_skip!();
    unsafe {
        // Uma profundidade menor que a tela (como o Haste com FSR): as coordenadas normalizadas resolvem.
        let depth = depth_image(&gpu, 4, 2, 0.25);
        let targets = gpu.targets(2, input);
        let input_depth = DepthInput { image: depth.image, format: DEPTH_FORMAT, layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL, extent: vk::Extent2D { width: 4, height: 2 }, linear_filter: true };
        let mut effect = Effect::new(&gpu.device, &gpu.memory, &bundle(), 0, &target(&targets.images, &gpu, Some(input_depth)), gpu.queue).expect("criar o efeito");
        // Dois quadros seguidos: o layout volta ao original a cada um, então o segundo parte do mesmo estado.
        for frame in 0..2 {
            for (i, &image) in targets.images.iter().enumerate() {
                gpu.submit(effect.command_buffer(i));
                for (n, got) in gpu.read(image).into_iter().enumerate() {
                    assert_pixel(got, [64, 64, 64, 255], 1, &format!("quadro {frame} imagem {i} pixel {n}"));
                }
            }
        }
        effect.destroy(&gpu.device);
        gpu.destroy_targets(targets);
        gpu.device.destroy_image(depth.image, None);
        gpu.device.free_memory(depth.memory, None);
    }
}

#[test]
fn without_a_depth_image_an_effect_that_needs_depth_is_refused_with_the_reason() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(1, input);
        match Effect::new(&gpu.device, &gpu.memory, &bundle(), 0, &target(&targets.images, &gpu, None), gpu.queue) {
            Err(Error::Unsupported(why)) => assert!(why.contains("profundidade"), "{why}"),
            other => panic!("esperava Unsupported, veio {:?}", other.map(|_| ())),
        }
        // Layout desconhecido: também recusa (a barreira descartaria o conteúdo).
        let depth = depth_image(&gpu, 4, 2, 0.5);
        let unknown = DepthInput { image: depth.image, format: DEPTH_FORMAT, layout: vk::ImageLayout::UNDEFINED, extent: vk::Extent2D { width: 4, height: 2 }, linear_filter: true };
        match Effect::new(&gpu.device, &gpu.memory, &bundle(), 0, &target(&targets.images, &gpu, Some(unknown)), gpu.queue) {
            Err(Error::Unsupported(why)) => assert!(why.contains("layout"), "{why}"),
            other => panic!("esperava Unsupported, veio {:?}", other.map(|_| ())),
        }
        gpu.destroy_targets(targets);
        gpu.device.destroy_image(depth.image, None);
        gpu.device.free_memory(depth.memory, None);
    }
}
