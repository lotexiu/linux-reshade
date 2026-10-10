//! A cadeia: dois efeitos em ordem trocada dão imagens diferentes, e cada uma bate com o cálculo na CPU.

mod common;

use ash::vk;
use common::*;
use fx_bundle::Bundle;
use postfx::effect::Target;
use postfx::{Chain, Error, Step};
use std::path::Path;

fn bundle(name: &str) -> Bundle {
    Bundle::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).expect("carregar o bundle de teste")
}

fn target(images: &[vk::Image], gpu: &Gpu) -> Target<'static> {
    let images: &'static [vk::Image] = Box::leak(images.to_vec().into_boxed_slice());
    Target { images, format: FORMAT, extent: vk::Extent2D { width: W, height: H }, rest_layout: REST, queue_family: gpu.family, init_command_buffer: None, depth: None }
}

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

fn sepia(c: [u8; 4]) -> [u8; 4] {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let gray = 0.299 * r + 0.587 * g + 0.114 * b;
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [to8(gray * 1.2), to8(gray), to8(gray * 0.8), c[3]]
}

fn invert(c: [u8; 4]) -> [u8; 4] {
    [255 - c[0], 255 - c[1], 255 - c[2], c[3]]
}

/// Roda a cadeia sobre a imagem 0 e devolve os pixels.
unsafe fn run(gpu: &Gpu, steps: &[Step<'_>]) -> Vec<[u8; 4]> {
    unsafe {
        let targets = gpu.targets(1, input);
        let mut chain = Chain::new(&gpu.device, &gpu.memory, steps, &target(&targets.images, gpu), gpu.queue).expect("criar a cadeia");
        assert_eq!(chain.len(), steps.len());
        gpu.submit_all(&chain.command_buffers(0));
        let pixels = gpu.read(targets.images[0]);
        chain.destroy(&gpu.device);
        chain.destroy(&gpu.device); // idempotente
        gpu.destroy_targets(targets);
        pixels
    }
}

#[test]
fn swapping_the_order_changes_the_image() {
    let gpu = gpu_or_skip!();
    let (sepia_fx, invert_fx) = (bundle("Sepia"), bundle("Invert"));
    unsafe {
        let a = run(&gpu, &[Step::named(&sepia_fx, "Sepia").unwrap(), Step::named(&invert_fx, "Invert").unwrap()]);
        let b = run(&gpu, &[Step::named(&invert_fx, "Invert").unwrap(), Step::named(&sepia_fx, "Sepia").unwrap()]);
        assert_ne!(a, b, "a ordem tem de importar");
        for (n, (got_a, got_b)) in a.iter().zip(&b).enumerate() {
            let s = input(n as u32 % W, n as u32 / W);
            assert_pixel(*got_a, invert(sepia(s)), 3, &format!("sépia e depois inverter, pixel {n}"));
            assert_pixel(*got_b, sepia(invert(s)), 3, &format!("inverter e depois sépia, pixel {n}"));
        }
    }
}

#[test]
fn the_same_effect_twice_composes() {
    let gpu = gpu_or_skip!();
    let invert_fx = bundle("Invert");
    unsafe {
        let step = Step::named(&invert_fx, "Invert").unwrap();
        let twice = run(&gpu, &[step, step]);
        for (n, got) in twice.iter().enumerate() {
            assert_pixel(*got, input(n as u32 % W, n as u32 / W), 1, &format!("inverter duas vezes devolve o original, pixel {n}"));
        }
        let once = run(&gpu, &[step]);
        assert_ne!(once, twice);
    }
}

#[test]
fn an_empty_chain_leaves_the_image_alone() {
    let gpu = gpu_or_skip!();
    unsafe {
        let pixels = run(&gpu, &[]);
        for (n, got) in pixels.iter().enumerate() {
            assert_pixel(*got, input(n as u32 % W, n as u32 / W), 0, &format!("pixel {n}"));
        }
    }
}

#[test]
fn a_failing_step_is_named_and_nothing_leaks() {
    let gpu = gpu_or_skip!();
    let invert_fx = bundle("Invert");
    unsafe {
        let targets = gpu.targets(1, input);
        let steps = [Step { bundle: &invert_fx, technique: 0 }, Step { bundle: &invert_fx, technique: 9 }];
        match Chain::new(&gpu.device, &gpu.memory, &steps, &target(&targets.images, &gpu), gpu.queue) {
            Err(Error::Invalid(why)) => assert!(why.starts_with("etapa 1:"), "{why}"),
            other => panic!("esperava Invalid na etapa 1, veio {:?}", other.map(|_| ())),
        }
        gpu.destroy_targets(targets);
    }
}

#[test]
fn techniques_are_found_by_name() {
    let invert_fx = bundle("Invert");
    assert_eq!(postfx::chain::technique_index(&invert_fx, "Invert"), Some(0));
    assert!(Step::named(&invert_fx, "Nao existe").is_none());
}

/// Um bundle de teste que pede mipmaps na textura do frame, o que o runtime ainda não faz.
fn mipmap_bundle() -> Bundle {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/Invert");
    let dst = std::env::temp_dir().join(format!("chain-mip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&dst).unwrap();
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        std::fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
    }
    let manifest = std::fs::read_to_string(dst.join("manifest.json")).unwrap().replace("\"levels\":1", "\"levels\":4");
    std::fs::write(dst.join("manifest.json"), manifest).unwrap();
    let bundle = Bundle::load(&dst).unwrap();
    let _ = std::fs::remove_dir_all(&dst);
    bundle
}

#[test]
fn a_lenient_chain_skips_what_the_runtime_cannot_do_and_keeps_the_rest() {
    let gpu = gpu_or_skip!();
    let (sepia_fx, invert_fx, mip_fx) = (bundle("Sepia"), bundle("Invert"), mipmap_bundle());
    unsafe {
        let targets = gpu.targets(1, input);
        let steps = [
            Step::named(&invert_fx, "Invert").unwrap(),
            Step { bundle: &mip_fx, technique: 0 },
            Step::named(&sepia_fx, "Sepia").unwrap(),
        ];
        // A cadeia estrita recusa o conjunto; a leniente pula só a etapa 1.
        assert!(matches!(Chain::new(&gpu.device, &gpu.memory, &steps, &target(&targets.images, &gpu), gpu.queue), Err(Error::Unsupported(_))));
        let (mut chain, skipped) = Chain::new_lenient(&gpu.device, &gpu.memory, &steps, &target(&targets.images, &gpu), gpu.queue).unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].step, 1);
        assert!(skipped[0].why.contains("mipmaps"), "{}", skipped[0].why);
        assert_eq!((chain.len(), chain.origin(0), chain.origin(1)), (2, Some(0), Some(2)));

        gpu.submit_all(&chain.command_buffers(0));
        for (n, got) in gpu.read(targets.images[0]).into_iter().enumerate() {
            let s = input(n as u32 % W, n as u32 / W);
            assert_pixel(got, sepia(invert(s)), 3, &format!("inverter e depois sépia, pixel {n}"));
        }
        chain.destroy(&gpu.device);
        gpu.destroy_targets(targets);
    }
}
