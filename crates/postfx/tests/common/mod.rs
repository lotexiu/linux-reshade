//! Contexto Vulkan de software (lavapipe) e utilitários para os testes do postfx: criar imagens,
//! preenchê-las com um padrão conhecido e ler os pixels de volta. Sem janela nem swapchain.

#![allow(dead_code)]

use ash::vk;

pub const W: u32 = 8;
pub const H: u32 = 4;
pub const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
/// Layout em que as imagens "chegam" e voltam (num swapchain seria PRESENT_SRC_KHR).
pub const REST: vk::ImageLayout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;

pub struct Gpu {
    _entry: ash::Entry,
    pub instance: ash::Instance,
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub family: u32,
    pub memory: vk::PhysicalDeviceMemoryProperties,
}

pub fn range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 }
}

fn layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 }
}

/// Imagens-alvo com a memória delas.
pub struct Targets {
    pub images: Vec<vk::Image>,
    memories: Vec<vk::DeviceMemory>,
}

impl Gpu {
    /// `None` se não houver Vulkan ou dispositivo de software.
    pub fn software() -> Option<Self> {
        unsafe {
            let entry = ash::Entry::load().ok()?;
            let app = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 0, 0));
            let instance = entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app), None).ok()?;
            let physical = instance
                .enumerate_physical_devices()
                .ok()?
                .into_iter()
                .find(|&p| {
                    let kind = instance.get_physical_device_properties(p).device_type;
                    // LINUX_RESHADE_TEST_HARDWARE=1: usa a GPU de verdade (para ver o que o driver dela recusa).
                    if std::env::var_os("LINUX_RESHADE_TEST_HARDWARE").is_some_and(|v| v != "0") {
                        kind == vk::PhysicalDeviceType::DISCRETE_GPU
                    } else {
                        kind == vk::PhysicalDeviceType::CPU
                    }
                })?;
            let family = instance
                .get_physical_device_queue_family_properties(physical)
                .iter()
                .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))? as u32;
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
            // Os mesmos recursos opcionais que a layer liga no dispositivo do jogo.
            let have = instance.get_physical_device_features(physical);
            let features = vk::PhysicalDeviceFeatures::default()
                .shader_image_gather_extended(have.shader_image_gather_extended == vk::TRUE)
                .shader_storage_image_extended_formats(have.shader_storage_image_extended_formats == vk::TRUE)
                .shader_storage_image_read_without_format(have.shader_storage_image_read_without_format == vk::TRUE)
                .shader_storage_image_write_without_format(have.shader_storage_image_write_without_format == vk::TRUE);
            let device = instance.create_device(physical, &vk::DeviceCreateInfo::default().queue_create_infos(&queues).enabled_features(&features), None).ok()?;
            let queue = device.get_device_queue(family, 0);
            let memory = instance.get_physical_device_memory_properties(physical);
            Some(Self { _entry: entry, instance, device, queue, family, memory })
        }
    }

    pub unsafe fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> u32 {
        (0..self.memory.memory_type_count)
            .find(|&i| bits & (1 << i) != 0 && self.memory.memory_types[i as usize].property_flags.contains(flags))
            .expect("tipo de memória")
    }

    /// Grava e executa um command buffer descartável, esperando terminar.
    pub unsafe fn run(&self, record: impl FnOnce(vk::CommandBuffer)) {
        unsafe {
            let pool = self.device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(self.family), None).unwrap();
            let cb = self
                .device
                .allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1))
                .unwrap()[0];
            self.device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default()).unwrap();
            record(cb);
            self.device.end_command_buffer(cb).unwrap();
            let cbs = [cb];
            self.device.queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&cbs)], vk::Fence::null()).unwrap();
            self.device.queue_wait_idle(self.queue).unwrap();
            self.device.destroy_command_pool(pool, None);
        }
    }

    /// Submete o command buffer do efeito para a imagem `index` e espera terminar.
    pub unsafe fn submit(&self, cb: vk::CommandBuffer) {
        unsafe {
            let cbs = [cb];
            self.device.queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&cbs)], vk::Fence::null()).unwrap();
            self.device.queue_wait_idle(self.queue).unwrap();
        }
    }

    /// Vários command buffers num único submit, na ordem (como a layer faz com uma cadeia).
    pub unsafe fn submit_all(&self, cbs: &[vk::CommandBuffer]) {
        unsafe {
            self.device.queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(cbs)], vk::Fence::null()).unwrap();
            self.device.queue_wait_idle(self.queue).unwrap();
        }
    }

    unsafe fn host_buffer(&self, size: u64) -> (vk::Buffer, vk::DeviceMemory) {
        unsafe {
            let buffer = self
                .device
                .create_buffer(&vk::BufferCreateInfo::default().size(size).usage(vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST), None)
                .unwrap();
            let req = self.device.get_buffer_memory_requirements(buffer);
            let flags = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
            let mem = self
                .device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(self.memory_type(req.memory_type_bits, flags)), None)
                .unwrap();
            self.device.bind_buffer_memory(buffer, mem, 0).unwrap();
            (buffer, mem)
        }
    }

    fn transition(&self, cb: vk::CommandBuffer, image: vk::Image, old: vk::ImageLayout, new: vk::ImageLayout) {
        let b = vk::ImageMemoryBarrier::default()
            .image(image)
            .old_layout(old)
            .new_layout(new)
            .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .subresource_range(range());
        unsafe {
            self.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::ALL_COMMANDS, vk::PipelineStageFlags::ALL_COMMANDS, vk::DependencyFlags::empty(), &[], &[], &[b]);
        }
    }

    /// `n` imagens `W`x`H` com o uso que um swapchain com o uso extra do efeito teria, preenchidas
    /// com `pixel(x, y)` e deixadas no layout `REST`.
    pub unsafe fn targets(&self, n: usize, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Targets {
        unsafe { self.targets_in(FORMAT, n, pixel) }
    }

    /// Como [`targets`](Self::targets), num formato dado (os bytes de 8 bits são os mesmos em UNORM e sRGB).
    pub unsafe fn targets_in(&self, format: vk::Format, n: usize, pixel: impl Fn(u32, u32) -> [u8; 4]) -> Targets {
        let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
        let size = (W * H * 4) as u64;
        let (staging, staging_mem) = unsafe { self.host_buffer(size) };
        unsafe {
            let ptr = self.device.map_memory(staging_mem, 0, size, vk::MemoryMapFlags::empty()).unwrap() as *mut u8;
            for y in 0..H {
                for x in 0..W {
                    ptr.add(((y * W + x) * 4) as usize).copy_from_nonoverlapping(pixel(x, y).as_ptr(), 4);
                }
            }
            self.device.unmap_memory(staging_mem);
        }
        let region = vk::BufferImageCopy::default().image_subresource(layers()).image_extent(vk::Extent3D { width: W, height: H, depth: 1 });
        let (mut images, mut memories) = (Vec::new(), Vec::new());
        for _ in 0..n {
            unsafe {
                let info = vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(format)
                    .extent(vk::Extent3D { width: W, height: H, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(usage)
                    .initial_layout(vk::ImageLayout::UNDEFINED);
                let image = self.device.create_image(&info, None).unwrap();
                let req = self.device.get_image_memory_requirements(image);
                let mem = self
                    .device
                    .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(self.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::empty())), None)
                    .unwrap();
                self.device.bind_image_memory(image, mem, 0).unwrap();
                self.run(|cb| {
                    self.transition(cb, image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                    self.device.cmd_copy_buffer_to_image(cb, staging, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);
                    self.transition(cb, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, REST);
                });
                images.push(image);
                memories.push(mem);
            }
        }
        unsafe {
            self.device.destroy_buffer(staging, None);
            self.device.free_memory(staging_mem, None);
        }
        Targets { images, memories }
    }

    /// Lê de volta os pixels RGBA de `image` (que deve estar no layout `REST`).
    pub unsafe fn read(&self, image: vk::Image) -> Vec<[u8; 4]> {
        let size = (W * H * 4) as u64;
        let (buffer, mem) = unsafe { self.host_buffer(size) };
        let region = vk::BufferImageCopy::default().image_subresource(layers()).image_extent(vk::Extent3D { width: W, height: H, depth: 1 });
        unsafe {
            self.run(|cb| self.device.cmd_copy_image_to_buffer(cb, image, REST, buffer, &[region]));
            let ptr = self.device.map_memory(mem, 0, size, vk::MemoryMapFlags::empty()).unwrap() as *const u8;
            let pixels = (0..(W * H) as usize).map(|i| [*ptr.add(i * 4), *ptr.add(i * 4 + 1), *ptr.add(i * 4 + 2), *ptr.add(i * 4 + 3)]).collect();
            self.device.unmap_memory(mem);
            self.device.destroy_buffer(buffer, None);
            self.device.free_memory(mem, None);
            pixels
        }
    }

    pub unsafe fn destroy_targets(&self, targets: Targets) {
        unsafe {
            for (image, mem) in targets.images.into_iter().zip(targets.memories) {
                self.device.destroy_image(image, None);
                self.device.free_memory(mem, None);
            }
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

/// Entrada padrão dos testes: um degradê previsível, com alfa 200.
pub fn input(x: u32, y: u32) -> [u8; 4] {
    [(x * 30) as u8, (y * 60) as u8, 100, 200]
}

pub fn assert_pixel(got: [u8; 4], want: [u8; 4], tolerance: u8, what: &str) {
    for c in 0..4 {
        assert!(got[c].abs_diff(want[c]) <= tolerance, "{what}: veio {got:?}, esperado {want:?} (tolerância {tolerance})");
    }
}

pub const DEPTH_FORMAT: vk::Format = vk::Format::D32_SFLOAT;

pub struct DepthImage {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
}

/// Uma imagem de profundidade `width`x`height` cheia de `value`, no layout `DEPTH_STENCIL_ATTACHMENT_OPTIMAL`,
/// como um jogo a deixaria (com `SAMPLED` no uso, que a layer acrescenta).
pub unsafe fn depth_image(gpu: &Gpu, width: u32, height: u32, value: f32) -> DepthImage {
    unsafe { depth_image_with(gpu, DEPTH_FORMAT, width, height, value, None) }
}

/// Como `depth_image`, mas com um padrão por pixel (`(x, y) -> profundidade`) no lugar do valor único: uma imagem plana
/// esconde erros que só aparecem quando a profundidade varia (autofoco, ordenação).
pub unsafe fn depth_image_with(gpu: &Gpu, format: vk::Format, width: u32, height: u32, value: f32, pattern: Option<&dyn Fn(u32, u32) -> f32>) -> DepthImage {
    unsafe {
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = gpu.device.create_image(&info, None).unwrap();
        let req = gpu.device.get_image_memory_requirements(image);
        let memory = gpu
            .device
            .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(gpu.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::empty())), None)
            .unwrap();
        gpu.device.bind_image_memory(image, memory, 0).unwrap();
        let range = vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::DEPTH, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 };
        let barrier = |old, new| {
            vk::ImageMemoryBarrier::default()
                .image(image)
                .old_layout(old)
                .new_layout(new)
                .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(range)
        };
        let staging = pattern.map(|f| {
            let bytes = u64::from(width) * u64::from(height) * 4;
            let buffer = gpu.device.create_buffer(&vk::BufferCreateInfo::default().size(bytes).usage(vk::BufferUsageFlags::TRANSFER_SRC), None).unwrap();
            let req = gpu.device.get_buffer_memory_requirements(buffer);
            let mem = gpu
                .device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(gpu.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT)), None)
                .unwrap();
            gpu.device.bind_buffer_memory(buffer, mem, 0).unwrap();
            let ptr = gpu.device.map_memory(mem, 0, bytes, vk::MemoryMapFlags::empty()).unwrap().cast::<f32>();
            for y in 0..height {
                for x in 0..width {
                    ptr.add((y * width + x) as usize).write(f(x, y));
                }
            }
            gpu.device.unmap_memory(mem);
            (buffer, mem)
        });
        gpu.run(|cb| {
            let all = vk::PipelineStageFlags::ALL_COMMANDS;
            gpu.device.cmd_pipeline_barrier(cb, all, all, vk::DependencyFlags::empty(), &[], &[], &[barrier(vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL)]);
            gpu.device.cmd_clear_depth_stencil_image(cb, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &vk::ClearDepthStencilValue { depth: value, stencil: 0 }, &[range]);
            if let Some((buffer, _)) = &staging {
                let copy = vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::DEPTH, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                    .image_extent(vk::Extent3D { width, height, depth: 1 });
                gpu.device.cmd_copy_buffer_to_image(cb, *buffer, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[copy]);
            }
            gpu.device.cmd_pipeline_barrier(cb, all, all, vk::DependencyFlags::empty(), &[], &[], &[barrier(vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)]);
        });
        if let Some((buffer, mem)) = staging {
            gpu.device.destroy_buffer(buffer, None);
            gpu.device.free_memory(mem, None);
        }
        DepthImage { image, memory }
    }
}


impl Gpu {
    /// Uma imagem alvo `w`x`h` (no formato `FORMAT`, layout `REST`) preenchida por `pixel`.
    pub unsafe fn target_sized(&self, w: u32, h: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) -> (vk::Image, vk::DeviceMemory) {
        let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
        let size = (w * h * 4) as u64;
        unsafe {
            let (staging, staging_mem) = self.host_buffer(size);
            let ptr = self.device.map_memory(staging_mem, 0, size, vk::MemoryMapFlags::empty()).unwrap() as *mut u8;
            for y in 0..h {
                for x in 0..w {
                    ptr.add(((y * w + x) * 4) as usize).copy_from_nonoverlapping(pixel(x, y).as_ptr(), 4);
                }
            }
            self.device.unmap_memory(staging_mem);
            let info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(FORMAT)
                .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = self.device.create_image(&info, None).unwrap();
            let req = self.device.get_image_memory_requirements(image);
            let mem = self
                .device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(self.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::empty())), None)
                .unwrap();
            self.device.bind_image_memory(image, mem, 0).unwrap();
            let region = vk::BufferImageCopy::default().image_subresource(layers()).image_extent(vk::Extent3D { width: w, height: h, depth: 1 });
            self.run(|cb| {
                self.transition(cb, image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                self.device.cmd_copy_buffer_to_image(cb, staging, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);
                self.transition(cb, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, REST);
            });
            self.device.destroy_buffer(staging, None);
            self.device.free_memory(staging_mem, None);
            (image, mem)
        }
    }

    /// Lê os pixels RGBA de uma imagem `w`x`h` (layout `REST`).
    pub unsafe fn read_sized(&self, image: vk::Image, w: u32, h: u32) -> Vec<[u8; 4]> {
        let size = (w * h * 4) as u64;
        unsafe {
            let (buffer, mem) = self.host_buffer(size);
            let region = vk::BufferImageCopy::default().image_subresource(layers()).image_extent(vk::Extent3D { width: w, height: h, depth: 1 });
            self.run(|cb| self.device.cmd_copy_image_to_buffer(cb, image, REST, buffer, &[region]));
            let ptr = self.device.map_memory(mem, 0, size, vk::MemoryMapFlags::empty()).unwrap() as *const u8;
            let pixels = (0..(w * h) as usize).map(|i| [*ptr.add(i * 4), *ptr.add(i * 4 + 1), *ptr.add(i * 4 + 2), *ptr.add(i * 4 + 3)]).collect();
            self.device.unmap_memory(mem);
            self.device.destroy_buffer(buffer, None);
            self.device.free_memory(mem, None);
            pixels
        }
    }
}

impl Gpu {
    /// Os bytes crus de uma imagem `w`x`h` com `bytes_per` por pixel, a partir do layout de leitura de shader.
    pub unsafe fn read_image_raw(&self, image: vk::Image, w: u32, h: u32, bytes_per: usize) -> Vec<u8> {
        let size = (w as usize * h as usize * bytes_per) as u64;
        unsafe {
            let (buffer, mem) = self.host_buffer(size);
            let region = vk::BufferImageCopy::default().image_subresource(layers()).image_extent(vk::Extent3D { width: w, height: h, depth: 1 });
            self.run(|cb| {
                let to = |old, new| {
                    vk::ImageMemoryBarrier::default()
                        .image(image)
                        .old_layout(old)
                        .new_layout(new)
                        .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                        .dst_access_mask(vk::AccessFlags::MEMORY_READ)
                        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                        .subresource_range(range())
                };
                let all = vk::PipelineStageFlags::ALL_COMMANDS;
                self.device.cmd_pipeline_barrier(cb, all, all, vk::DependencyFlags::empty(), &[], &[], &[to(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL, vk::ImageLayout::TRANSFER_SRC_OPTIMAL)]);
                self.device.cmd_copy_image_to_buffer(cb, image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, buffer, &[region]);
                self.device.cmd_pipeline_barrier(cb, all, all, vk::DependencyFlags::empty(), &[], &[], &[to(vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)]);
            });
            let ptr = self.device.map_memory(mem, 0, size, vk::MemoryMapFlags::empty()).unwrap() as *const u8;
            let bytes = std::slice::from_raw_parts(ptr, size as usize).to_vec();
            self.device.unmap_memory(mem);
            self.device.destroy_buffer(buffer, None);
            self.device.free_memory(mem, None);
            bytes
        }
    }
}
