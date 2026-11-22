//! Cópias, blits e limpezas de imagens de cor, que também entram na observação.

use super::*;

pub(super) unsafe extern "system" fn cmd_blit_image(cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, dst: vk::Image, dst_layout: vk::ImageLayout, count: u32, p_regions: *const vk::ImageBlit, filter: vk::Filter) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, dst, crate::depth::screen::Kind::Blit);
        (data.device.fp_v1_0().cmd_blit_image)(cmd, src, src_layout, dst, dst_layout, count, p_regions, filter);
    }
}

pub(super) unsafe extern "system" fn cmd_copy_image(cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, dst: vk::Image, dst_layout: vk::ImageLayout, count: u32, p_regions: *const vk::ImageCopy) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, dst, crate::depth::screen::Kind::Copy);
        (data.device.fp_v1_0().cmd_copy_image)(cmd, src, src_layout, dst, dst_layout, count, p_regions);
    }
}

pub(super) unsafe extern "system" fn cmd_resolve_image(cmd: vk::CommandBuffer, src: vk::Image, src_layout: vk::ImageLayout, dst: vk::Image, dst_layout: vk::ImageLayout, count: u32, p_regions: *const vk::ImageResolve) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, dst, crate::depth::screen::Kind::Resolve);
        (data.device.fp_v1_0().cmd_resolve_image)(cmd, src, src_layout, dst, dst_layout, count, p_regions);
    }
}

pub(super) unsafe extern "system" fn cmd_clear_color_image(cmd: vk::CommandBuffer, image: vk::Image, layout: vk::ImageLayout, p_color: *const vk::ClearColorValue, count: u32, p_ranges: *const vk::ImageSubresourceRange) {
    unsafe {
        let Some(data) = data_of(cmd.as_raw()) else { return };
        screen_event(&data, cmd, image, crate::depth::screen::Kind::Clear);
        (data.device.fp_v1_0().cmd_clear_color_image)(cmd, image, layout, p_color, count, p_ranges);
    }
}
