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
