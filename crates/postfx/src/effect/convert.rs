//! Conversões do vocabulário do bundle para o do Vulkan. Funções puras, testáveis sem GPU.

use ash::vk;
use fx_bundle::manifest::{Address, BlendFactor, BlendOp, Filter, TextureFormat, Topology};

pub fn texture_format(f: TextureFormat) -> vk::Format {
    match f {
        TextureFormat::R8 => vk::Format::R8_UNORM,
        TextureFormat::R16f => vk::Format::R16_SFLOAT,
        TextureFormat::R16 => vk::Format::R16_UNORM,
        TextureFormat::R32f => vk::Format::R32_SFLOAT,
        TextureFormat::R32u => vk::Format::R32_UINT,
        TextureFormat::R32i => vk::Format::R32_SINT,
        TextureFormat::Rg8 => vk::Format::R8G8_UNORM,
        TextureFormat::Rg16f => vk::Format::R16G16_SFLOAT,
        TextureFormat::Rg16 => vk::Format::R16G16_UNORM,
        TextureFormat::Rg32f => vk::Format::R32G32_SFLOAT,
        // Sem formato declarado, o ReShade usa RGBA8.
        TextureFormat::Rgba8 | TextureFormat::Unknown => vk::Format::R8G8B8A8_UNORM,
        TextureFormat::Rgba16f => vk::Format::R16G16B16A16_SFLOAT,
        TextureFormat::Rgba16 => vk::Format::R16G16B16A16_UNORM,
        TextureFormat::Rgba32f => vk::Format::R32G32B32A32_SFLOAT,
        TextureFormat::Rgba32u => vk::Format::R32G32B32A32_UINT,
        TextureFormat::Rgba32i => vk::Format::R32G32B32A32_SINT,
        TextureFormat::Rgb10a2 => vk::Format::A2B10G10R10_UNORM_PACK32,
        TextureFormat::Rg11b10f => vk::Format::B10G11R11_UFLOAT_PACK32,
    }
}

/// Formatos sRGB de 8 bits por canal (os que um swapchain costuma usar).
pub fn is_srgb(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::R8G8B8A8_SRGB
            | vk::Format::B8G8R8A8_SRGB
            | vk::Format::A8B8G8R8_SRGB_PACK32
            | vk::Format::R8G8B8_SRGB
            | vk::Format::B8G8R8_SRGB
            | vk::Format::R8G8_SRGB
            | vk::Format::R8_SRGB
    )
}

/// A versão sRGB de um formato de 8 bits por canal; `None` para formatos sem ela (float, 10 bits...). O
/// ReShade ignora `SRGBWriteEnable`/`SRGBTexture` nesses casos, e nós também.
pub fn srgb_counterpart(format: vk::Format) -> Option<vk::Format> {
    Some(match format {
        vk::Format::R8G8B8A8_UNORM => vk::Format::R8G8B8A8_SRGB,
        vk::Format::B8G8R8A8_UNORM => vk::Format::B8G8R8A8_SRGB,
        vk::Format::A8B8G8R8_UNORM_PACK32 => vk::Format::A8B8G8R8_SRGB_PACK32,
        vk::Format::R8G8_UNORM => vk::Format::R8G8_SRGB,
        vk::Format::R8_UNORM => vk::Format::R8_SRGB,
        _ => return None,
    })
}

/// Os pixels RGBA8 de uma imagem de arquivo no formato da textura: `R8` fica com o canal vermelho e `Rg8` com
/// vermelho e verde. `None` para formatos que um arquivo de 8 bits não preenche (float, 16 bits...).
pub fn pack_pixels(format: TextureFormat, rgba: &[u8]) -> Option<Vec<u8>> {
    match format {
        TextureFormat::Rgba8 | TextureFormat::Unknown => Some(rgba.to_vec()),
        TextureFormat::Rg8 => Some(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1]]).collect()),
        TextureFormat::R8 => Some(rgba.chunks_exact(4).map(|p| p[0]).collect()),
        _ => None,
    }
}

pub fn filter(f: Filter) -> vk::Filter {
    match f {
        Filter::Point => vk::Filter::NEAREST,
        Filter::Linear => vk::Filter::LINEAR,
    }
}

pub fn mipmap_mode(f: Filter) -> vk::SamplerMipmapMode {
    match f {
        Filter::Point => vk::SamplerMipmapMode::NEAREST,
        Filter::Linear => vk::SamplerMipmapMode::LINEAR,
    }
}

pub fn address(a: Address) -> vk::SamplerAddressMode {
    match a {
        Address::Wrap => vk::SamplerAddressMode::REPEAT,
        Address::Mirror => vk::SamplerAddressMode::MIRRORED_REPEAT,
        Address::Clamp => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        Address::Border => vk::SamplerAddressMode::CLAMP_TO_BORDER,
    }
}

pub fn blend_factor(f: BlendFactor) -> vk::BlendFactor {
    match f {
        BlendFactor::Zero => vk::BlendFactor::ZERO,
        BlendFactor::One => vk::BlendFactor::ONE,
        BlendFactor::SourceColor => vk::BlendFactor::SRC_COLOR,
        BlendFactor::OneMinusSourceColor => vk::BlendFactor::ONE_MINUS_SRC_COLOR,
        BlendFactor::DestColor => vk::BlendFactor::DST_COLOR,
        BlendFactor::OneMinusDestColor => vk::BlendFactor::ONE_MINUS_DST_COLOR,
        BlendFactor::SourceAlpha => vk::BlendFactor::SRC_ALPHA,
        BlendFactor::OneMinusSourceAlpha => vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        BlendFactor::DestAlpha => vk::BlendFactor::DST_ALPHA,
        BlendFactor::OneMinusDestAlpha => vk::BlendFactor::ONE_MINUS_DST_ALPHA,
    }
}

pub fn blend_op(o: BlendOp) -> vk::BlendOp {
    match o {
        BlendOp::Add => vk::BlendOp::ADD,
        BlendOp::Subtract => vk::BlendOp::SUBTRACT,
        BlendOp::ReverseSubtract => vk::BlendOp::REVERSE_SUBTRACT,
        BlendOp::Min => vk::BlendOp::MIN,
        BlendOp::Max => vk::BlendOp::MAX,
    }
}

pub fn topology(t: Topology) -> vk::PrimitiveTopology {
    match t {
        Topology::PointList => vk::PrimitiveTopology::POINT_LIST,
        Topology::LineList => vk::PrimitiveTopology::LINE_LIST,
        Topology::LineStrip => vk::PrimitiveTopology::LINE_STRIP,
        Topology::TriangleList => vk::PrimitiveTopology::TRIANGLE_LIST,
        Topology::TriangleStrip => vk::PrimitiveTopology::TRIANGLE_STRIP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_texture_format_defaults_to_rgba8() {
        assert_eq!(texture_format(TextureFormat::Unknown), vk::Format::R8G8B8A8_UNORM);
        assert_eq!(texture_format(TextureFormat::Rgba16f), vk::Format::R16G16B16A16_SFLOAT);
    }

    #[test]
    fn srgb_detection_only_flags_srgb_formats() {
        assert!(is_srgb(vk::Format::B8G8R8A8_SRGB));
        assert!(is_srgb(vk::Format::R8G8B8A8_SRGB));
        assert!(!is_srgb(vk::Format::B8G8R8A8_UNORM));
        assert!(!is_srgb(vk::Format::R16G16B16A16_SFLOAT));
    }

    #[test]
    fn sampler_state_maps_one_to_one() {
        assert_eq!(address(Address::Border), vk::SamplerAddressMode::CLAMP_TO_BORDER);
        assert_eq!(filter(Filter::Point), vk::Filter::NEAREST);
        assert_eq!(blend_factor(BlendFactor::OneMinusDestColor), vk::BlendFactor::ONE_MINUS_DST_COLOR);
        assert_eq!(blend_op(BlendOp::ReverseSubtract), vk::BlendOp::REVERSE_SUBTRACT);
        assert_eq!(topology(Topology::TriangleStrip), vk::PrimitiveTopology::TRIANGLE_STRIP);
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    #[test]
    fn file_pixels_are_packed_to_the_texture_format() {
        let rgba = [1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(pack_pixels(TextureFormat::Rgba8, &rgba).unwrap(), rgba);
        assert_eq!(pack_pixels(TextureFormat::Unknown, &rgba).unwrap(), rgba);
        assert_eq!(pack_pixels(TextureFormat::Rg8, &rgba).unwrap(), [1, 2, 5, 6]);
        assert_eq!(pack_pixels(TextureFormat::R8, &rgba).unwrap(), [1, 5]);
        assert!(pack_pixels(TextureFormat::Rgba16f, &rgba).is_none());
    }

    #[test]
    fn only_eight_bit_unorm_formats_have_a_srgb_twin() {
        assert_eq!(srgb_counterpart(vk::Format::R8G8B8A8_UNORM), Some(vk::Format::R8G8B8A8_SRGB));
        assert_eq!(srgb_counterpart(vk::Format::A2B10G10R10_UNORM_PACK32), None);
        assert_eq!(srgb_counterpart(vk::Format::R16G16B16A16_SFLOAT), None);
    }
}
