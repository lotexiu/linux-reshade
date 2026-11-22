//! A medição do tempo de GPU de cada etapa da cadeia (só quando pedida).

use super::*;

impl GpuTimer {
    /// `None` se a fila não tem carimbos ou algo falhou (a medição simplesmente não existe).
    pub(super) unsafe fn new(data: &DeviceData, family: u32, images: usize, names: Vec<String>) -> Option<Self> {
        let instance = &data.instance.instance;
        let queue_props = unsafe { instance.get_physical_device_queue_family_properties(data.physical_device) };
        if queue_props.get(family as usize).is_none_or(|p| p.timestamp_valid_bits == 0) {
            return None;
        }
        let period = unsafe { instance.get_physical_device_properties(data.physical_device) }.limits.timestamp_period;
        let boundaries = names.len() + 1;
        let device = &data.device;
        unsafe {
            let pool = device.create_query_pool(&vk::QueryPoolCreateInfo::default().query_type(vk::QueryType::TIMESTAMP).query_count((images * boundaries) as u32), None).ok()?;
            let command_pool = match device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(family), None) {
                Ok(p) => p,
                Err(_) => {
                    device.destroy_query_pool(pool, None);
                    return None;
                }
            };
            let mut timer = Self { pool, command_pool, stamps: Vec::new(), written: vec![false; images], period, steps: names.into_iter().map(|n| (n, 0.0)).collect() };
            for image in 0..images {
                let cbs = device.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(command_pool).command_buffer_count(boundaries as u32)).ok();
                let Some(cbs) = cbs else {
                    timer.destroy(device);
                    return None;
                };
                for (boundary, cb) in cbs.iter().enumerate() {
                    let init = (data.set_loader_data)(device.handle(), cb.as_raw() as *mut std::ffi::c_void);
                    if init != vk::Result::SUCCESS || device.begin_command_buffer(*cb, &vk::CommandBufferBeginInfo::default()).is_err() {
                        timer.stamps.push(cbs.clone());
                        timer.destroy(device);
                        return None;
                    }
                    let query = (image * boundaries + boundary) as u32;
                    if boundary == 0 {
                        device.cmd_reset_query_pool(*cb, pool, (image * boundaries) as u32, boundaries as u32);
                    }
                    device.cmd_write_timestamp(*cb, vk::PipelineStageFlags::BOTTOM_OF_PIPE, pool, query);
                    if device.end_command_buffer(*cb).is_err() {
                        timer.stamps.push(cbs.clone());
                        timer.destroy(device);
                        return None;
                    }
                }
                timer.stamps.push(cbs);
            }
            Some(timer)
        }
    }

    /// Lê a medição que a imagem gravou da última vez que foi usada (a fence dela já foi esperada) e a soma às médias.
    pub(super) unsafe fn collect(&mut self, device: &ash::Device, image: usize) {
        if !self.written.get(image).copied().unwrap_or(false) {
            return;
        }
        let boundaries = self.steps.len() + 1;
        let mut ticks = vec![0u64; boundaries];
        let read = unsafe { device.get_query_pool_results(self.pool, (image * boundaries) as u32, &mut ticks, vk::QueryResultFlags::TYPE_64) };
        if read.is_ok() {
            for (i, (_, smoothed)) in self.steps.iter_mut().enumerate() {
                let us = ticks[i + 1].saturating_sub(ticks[i]) as f32 * self.period / 1000.0;
                *smoothed = if *smoothed == 0.0 { us } else { *smoothed * 0.9 + us * 0.1 };
            }
        }
    }

    pub(super) unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            device.destroy_command_pool(self.command_pool, None);
            device.destroy_query_pool(self.pool, None);
        }
        self.stamps.clear();
    }
}
