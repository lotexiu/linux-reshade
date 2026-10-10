//! `SRGBWriteEnable` e `SRGBTexture` num alvo de 8 bits: a escrita codifica para sRGB e a leitura decodifica,
//! como o ReShade faz com as views sRGB do back buffer. Aqui o frame é UNORM e a conversão vem de imagens
//! sRGB auxiliares (cópia de bytes entre formatos compatíveis).

mod common;

use ash::vk;
use common::*;
use fx_bundle::Bundle;
use postfx::effect::Target;
use postfx::Effect;
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

/// O bundle `Invert` com o texto trocado no manifesto.
fn patched(name: &str, from: &str, to: &str) -> Bundle {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/Invert");
    let dst = std::env::temp_dir().join(format!("srgb-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&dst).unwrap();
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        std::fs::copy(entry.path(), dst.join(entry.file_name())).unwrap();
    }
    let manifest = std::fs::read_to_string(dst.join("manifest.json")).unwrap();
    assert!(manifest.contains(from), "{from}");
    std::fs::write(dst.join("manifest.json"), manifest.replace(from, to)).unwrap();
    let bundle = Bundle::load(&dst).unwrap();
    let _ = std::fs::remove_dir_all(&dst);
    bundle
}

fn encode(linear: f32) -> f32 {
    if linear <= 0.003_130_8 { linear * 12.92 } else { 1.055 * linear.powf(1.0 / 2.4) - 0.055 }
}

fn decode(srgb: f32) -> f32 {
    if srgb <= 0.040_45 { srgb / 12.92 } else { ((srgb + 0.055) / 1.055).powf(2.4) }
}

fn byte(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn run(gpu: &Gpu, bundle: &Bundle) -> Vec<[u8; 4]> {
    unsafe {
        let targets = gpu.targets(1, input);
        let images: &'static [vk::Image] = Box::leak(targets.images.clone().into_boxed_slice());
        let target = Target { images, format: FORMAT, extent: vk::Extent2D { width: W, height: H }, rest_layout: REST, queue_family: gpu.family, init_command_buffer: None };
        let mut effect = Effect::new(&gpu.device, &gpu.memory, bundle, 0, &target, gpu.queue).expect("criar o efeito");
        gpu.submit(effect.command_buffer(0));
        let pixels = gpu.read(targets.images[0]);
        effect.destroy(&gpu.device);
        gpu.destroy_targets(targets);
        pixels
    }
}

#[test]
fn srgb_write_encodes_what_the_shader_outputs() {
    let gpu = gpu_or_skip!();
    let bundle = patched("write", "\"srgb_write\":false", "\"srgb_write\":true");
    for (n, got) in run(&gpu, &bundle).into_iter().enumerate() {
        let s = input(n as u32 % W, n as u32 / W);
        // O shader devolve 1 - cor; a GPU grava esse valor codificado em sRGB.
        let want = |c: u8| byte(encode(1.0 - c as f32 / 255.0));
        assert_pixel(got, [want(s[0]), want(s[1]), want(s[2]), s[3]], 2, &format!("pixel {n}"));
    }
}

#[test]
fn srgb_read_decodes_what_the_shader_samples() {
    let gpu = gpu_or_skip!();
    let bundle = patched("read", "\"srgb\":false", "\"srgb\":true");
    for (n, got) in run(&gpu, &bundle).into_iter().enumerate() {
        let s = input(n as u32 % W, n as u32 / W);
        // O shader recebe o frame decodificado para linear e devolve 1 - isso, sem codificar.
        let want = |c: u8| byte(1.0 - decode(c as f32 / 255.0));
        assert_pixel(got, [want(s[0]), want(s[1]), want(s[2]), s[3]], 2, &format!("pixel {n}"));
    }
}

#[test]
fn without_either_flag_the_values_pass_through_unchanged() {
    let gpu = gpu_or_skip!();
    let bundle = Bundle::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/Invert")).unwrap();
    for (n, got) in run(&gpu, &bundle).into_iter().enumerate() {
        let s = input(n as u32 % W, n as u32 / W);
        assert_pixel(got, [255 - s[0], 255 - s[1], 255 - s[2], s[3]], 1, &format!("pixel {n}"));
    }
}
