//! Executa bundles reais (compilados pelo fx2spv; ver `scripts/dev.sh fx-golden`) numa GPU de
//! software e confere os pixels. Cobre: um passe simples, dois passes com textura intermediária,
//! escrita de uniforms e a recusa de alvos que a v1 não suporta.

mod common;

use ash::vk;
use common::*;
use fx_bundle::Bundle;
use postfx::effect::{FrameInfo, Target};
use postfx::{Effect, Error};
use std::path::Path;

fn bundle(name: &str) -> Bundle {
    Bundle::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).expect("carregar o bundle de teste")
}

fn target(images: &[vk::Image], gpu: &Gpu) -> Target<'static> {
    // O `'static` é seguro porque os testes mantêm `images` vivo até o fim; evita emprestar `images` na struct.
    let images: &'static [vk::Image] = Box::leak(images.to_vec().into_boxed_slice());
    Target { images, format: FORMAT, extent: vk::Extent2D { width: W, height: H }, rest_layout: REST, queue_family: gpu.family, init_command_buffer: None }
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

#[test]
fn invert_flips_every_pixel_of_every_image() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(2, input);
        let mut effect = Effect::new(&gpu.device, &gpu.memory, &bundle("Invert"), 0, &target(&targets.images, &gpu), gpu.queue).expect("criar o efeito");
        for (i, &image) in targets.images.iter().enumerate() {
            gpu.submit(effect.command_buffer(i));
            for (n, got) in gpu.read(image).into_iter().enumerate() {
                let (x, y) = (n as u32 % W, n as u32 / W);
                let s = input(x, y);
                assert_pixel(got, [255 - s[0], 255 - s[1], 255 - s[2], s[3]], 1, &format!("imagem {i} pixel ({x},{y})"));
            }
        }
        effect.destroy(&gpu.device);
        effect.destroy(&gpu.device); // idempotente
        gpu.destroy_targets(targets);
    }
}

#[test]
fn two_passes_go_through_the_intermediate_texture() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(2, input);
        let mut effect = Effect::new(&gpu.device, &gpu.memory, &bundle("TwoPass"), 0, &target(&targets.images, &gpu), gpu.queue).expect("criar o efeito");
        // Duas imagens em sequência: a textura intermediária é compartilhada entre elas.
        for (i, &image) in targets.images.iter().enumerate() {
            gpu.submit(effect.command_buffer(i));
            for (n, got) in gpu.read(image).into_iter().enumerate() {
                let (x, y) = (n as u32 % W, n as u32 / W);
                let s = input(x, y);
                // metade do frame (quantizada em 8 bits na textura) + 0,25 (64/255); alfa 1.
                let f = |v: u8| ((v as f32 * 0.5).round() + 63.75).min(255.0) as u8;
                assert_pixel(got, [f(s[0]), f(s[1]), f(s[2]), 255], 2, &format!("imagem {i} pixel ({x},{y})"));
            }
        }
        effect.destroy(&gpu.device);
        gpu.destroy_targets(targets);
    }
}

fn sepia(c: [u8; 4], strength: f32) -> [u8; 4] {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let gray = 0.299 * r + 0.587 * g + 0.114 * b;
    let tone = [gray * 1.2, gray, gray * 0.8];
    let mix = |orig: f32, t: f32| ((orig + (t - orig) * strength).clamp(0.0, 1.0) * 255.0).round() as u8;
    [mix(r, tone[0]), mix(g, tone[1]), mix(b, tone[2]), c[3]]
}

#[test]
fn uniform_default_and_overwrite_change_the_result() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(1, input);
        let mut effect = Effect::new(&gpu.device, &gpu.memory, &bundle("Sepia"), 0, &target(&targets.images, &gpu), gpu.queue).expect("criar o efeito");

        // Valor padrão do manifesto (Strength = 1.0).
        gpu.submit(effect.command_buffer(0));
        for (n, got) in gpu.read(targets.images[0]).into_iter().enumerate() {
            let (x, y) = (n as u32 % W, n as u32 / W);
            assert_pixel(got, sepia(input(x, y), 1.0), 2, &format!("padrão ({x},{y})"));
        }

        // Sobrescreve o uniform com 0.0: o efeito passa a devolver o frame que recebeu. Como o frame
        // agora é a saída do passo anterior, comparamos contra ela mesma passando de novo.
        let before = gpu.read(targets.images[0]);
        effect.write_uniforms(0, 0, &0.0f32.to_le_bytes());
        gpu.submit(effect.command_buffer(0));
        for (n, got) in gpu.read(targets.images[0]).into_iter().enumerate() {
            assert_pixel(got, before[n], 1, &format!("Strength 0 mantém o pixel {n}"));
        }
        effect.destroy(&gpu.device);
        gpu.destroy_targets(targets);
    }
}

#[test]
fn unknown_technique_is_an_error() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(1, input);
        let r = Effect::new(&gpu.device, &gpu.memory, &bundle("Invert"), 7, &target(&targets.images, &gpu), gpu.queue);
        assert!(matches!(r, Err(Error::Invalid(_))));
        gpu.destroy_targets(targets);
    }
}

#[test]
fn runtime_sources_feed_the_uniforms_each_frame() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(1, input);
        let mut effect = Effect::new(&gpu.device, &gpu.memory, &bundle("Timer"), 0, &target(&targets.images, &gpu), gpu.queue).expect("criar o efeito");

        // O `mousepoint` não é alimentado pelo runtime: tem de aparecer como ignorado.
        assert_eq!(effect.ignored_sources().len(), 1, "{:?}", effect.ignored_sources());
        assert!(effect.ignored_sources()[0].contains("mousepoint"));

        // Cada frame traz seus valores: o resultado muda com eles.
        for (time_ms, count, frame_ms) in [(250.0f32, 51u32, 51.0f32), (1000.0, 255, 255.0), (500.0, 0, 0.0)] {
            effect.update_sources(0, &FrameInfo { time_ms, frame_time_ms: frame_ms, frame_count: count, date: [0.0; 4] });
            gpu.submit(effect.command_buffer(0));
            let expected = [
                (time_ms / 1000.0 * 255.0).round() as u8,
                (count as f32 / 255.0 * 255.0).round() as u8,
                (frame_ms / 255.0 * 255.0).round() as u8,
            ];
            for (n, got) in gpu.read(targets.images[0]).into_iter().enumerate() {
                assert_pixel(got, [expected[0], expected[1], expected[2], 255], 2, &format!("t={time_ms} pixel {n}"));
            }
        }
        effect.destroy(&gpu.device);
        gpu.destroy_targets(targets);
    }
}

#[test]
fn mipmaps_are_generated_after_the_pass_that_writes_the_texture() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(2, input);
        let mut effect = Effect::new(&gpu.device, &gpu.memory, &bundle("Mip"), 0, &target(&targets.images, &gpu), gpu.queue).expect("criar o efeito");
        // O nível 3 tem 1x1: a média do frame (r = média de x*30 = 105, g = média de y*60 = 90, b = 100).
        for (i, &image) in targets.images.iter().enumerate() {
            gpu.submit(effect.command_buffer(i));
            for (n, got) in gpu.read(image).into_iter().enumerate() {
                assert_pixel(got, [105, 90, 100, 255], 3, &format!("imagem {i} pixel {n}"));
            }
        }
        effect.destroy(&gpu.device);
        gpu.destroy_targets(targets);
    }
}

/// O bundle `Tex` com a imagem 4x2 (`pixel(x, y)`) gravada como o app de controle faria; `edit` troca texto no manifesto.
fn bundle_with_image(name: &str, edit: impl Fn(String) -> String) -> Bundle {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/Tex");
    let dst = std::env::temp_dir().join(format!("tex-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&dst).unwrap();
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
        }
    }
    let manifest = std::fs::read_to_string(dst.join("manifest.json")).unwrap();
    std::fs::write(dst.join("manifest.json"), edit(manifest)).unwrap();
    let rgba = (0..8u32).flat_map(|n| [(n * 30) as u8, (255 - n * 30) as u8, (n * 10) as u8, 255]).collect();
    let image = fx_bundle::images::TextureImage { width: 4, height: 2, rgba };
    fx_bundle::images::write(&dst, 0, &image).unwrap();
    let bundle = Bundle::load(&dst).unwrap();
    let _ = std::fs::remove_dir_all(&dst);
    bundle
}

fn run_bundle(gpu: &Gpu, bundle: &Bundle) -> Vec<[u8; 4]> {
    unsafe {
        let targets = gpu.targets(1, input);
        let mut effect = Effect::new(&gpu.device, &gpu.memory, bundle, 0, &target(&targets.images, gpu), gpu.queue).expect("criar o efeito");
        gpu.submit(effect.command_buffer(0));
        let pixels = gpu.read(targets.images[0]);
        effect.destroy(&gpu.device);
        gpu.destroy_targets(targets);
        pixels
    }
}

#[test]
fn a_texture_from_a_file_shows_the_image_the_bundle_carries() {
    let gpu = gpu_or_skip!();
    let bundle = bundle_with_image("plain", |m| m);
    for (n, got) in run_bundle(&gpu, &bundle).into_iter().enumerate() {
        let (x, y) = (n as u32 % W, n as u32 / W);
        let texel = (y / 2) * 4 + x / 2; // 8x4 amostrando 4x2 sem filtro
        assert_pixel(got, [(texel * 30) as u8, (255 - texel * 30) as u8, (texel * 10) as u8, 255], 1, &format!("pixel ({x},{y})"));
    }
}

#[test]
fn a_srgb_sampler_decodes_the_file_texture() {
    let gpu = gpu_or_skip!();
    let bundle = bundle_with_image("srgb", |m| m.replacen("\"srgb\":false", "\"srgb\":true", 1));
    let decode = |c: u32| {
        let v = c as f32 / 255.0;
        let l = if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        (l * 255.0).round() as u8
    };
    for (n, got) in run_bundle(&gpu, &bundle).into_iter().enumerate() {
        let (x, y) = (n as u32 % W, n as u32 / W);
        let texel = (y / 2) * 4 + x / 2;
        assert_pixel(got, [decode(texel * 30), decode(255 - texel * 30), decode(texel * 10), 255], 2, &format!("pixel ({x},{y})"));
    }
}

#[test]
fn a_file_texture_without_its_image_is_refused_explicitly() {
    let gpu = gpu_or_skip!();
    unsafe {
        let targets = gpu.targets(1, input);
        match Effect::new(&gpu.device, &gpu.memory, &bundle("Tex"), 0, &target(&targets.images, &gpu), gpu.queue) {
            Err(Error::Unsupported(why)) => assert!(why.contains("img.png"), "{why}"),
            other => panic!("esperava Unsupported, veio {:?}", other.map(|_| ())),
        }
        gpu.destroy_targets(targets);
    }
}
