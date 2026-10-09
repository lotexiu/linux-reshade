//! Roda o passe de inversão de verdade, numa GPU de software (lavapipe), sem janela nem swapchain.
//! Se não houver um dispositivo Vulkan do tipo CPU, o teste avisa e é ignorado.

use ash::vk;
use postfx::{InvertPass, Target};

const W: u32 = 8;
const H: u32 = 4;
const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;
const REST: vk::ImageLayout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL;

struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
}

impl Gpu {
    /// `None` se não houver Vulkan ou dispositivo de software.
    fn software() -> Option<Self> {
        unsafe {
            let entry = ash::Entry::load().ok()?;
            let app = vk::ApplicationInfo::default().api_version(vk::make_api_version(0, 1, 0, 0));
            let instance = entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app), None).ok()?;
            let physical = instance
                .enumerate_physical_devices()
                .ok()?
                .into_iter()
                .find(|&p| instance.get_physical_device_properties(p).device_type == vk::PhysicalDeviceType::CPU)?;
            let family = instance
                .get_physical_device_queue_family_properties(physical)
                .iter()
                .position(|q| q.queue_flags.contains(vk::QueueFlags::GRAPHICS))? as u32;
            let priorities = [1.0];
            let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
            let device = instance.create_device(physical, &vk::DeviceCreateInfo::default().queue_create_infos(&queues), None).ok()?;
            let queue = device.get_device_queue(family, 0);
            let memory = instance.get_physical_device_memory_properties(physical);
            Some(Self { _entry: entry, instance, device, queue, family, memory })
        }
    }

    unsafe fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> u32 {
        (0..self.memory.memory_type_count)
            .find(|&i| bits & (1 << i) != 0 && self.memory.memory_types[i as usize].property_flags.contains(flags))
            .expect("tipo de memória")
    }

    /// Grava e executa um command buffer descartável, esperando terminar.
    unsafe fn run(&self, record: impl FnOnce(vk::CommandBuffer)) {
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

    /// Buffer visível ao host com `size` bytes.
    unsafe fn host_buffer(&self, size: u64) -> (vk::Buffer, vk::DeviceMemory) {
        unsafe {
            let buffer = self
                .device
                .create_buffer(
                    &vk::BufferCreateInfo::default().size(size).usage(vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST),
                    None,
                )
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
}

fn range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 }
}

fn layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 }
}

fn transition(gpu: &Gpu, cb: vk::CommandBuffer, image: vk::Image, old: vk::ImageLayout, new: vk::ImageLayout) {
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
        gpu.device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::ALL_COMMANDS, vk::PipelineStageFlags::ALL_COMMANDS, vk::DependencyFlags::empty(), &[], &[], &[b]);
    }
}

/// Pixel de entrada (x, y).
fn input(x: u32, y: u32) -> [u8; 4] {
    [(x * 30) as u8, (y * 60) as u8, 100, 200]
}

#[test]
fn inverts_every_image_in_place() {
    let Some(gpu) = Gpu::software() else {
        eprintln!("sem dispositivo Vulkan de software (lavapipe); teste ignorado");
        return;
    };
    unsafe {
        // Alvos: o que o jogo teria renderizado (duas imagens, como num swapchain com 2 buffers).
        let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
        let mut images = Vec::new();
        let mut memories = Vec::new();
        for _ in 0..2 {
            let info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(FORMAT)
                .extent(vk::Extent3D { width: W, height: H, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = gpu.device.create_image(&info, None).unwrap();
            let req = gpu.device.get_image_memory_requirements(image);
            let mem = gpu
                .device
                .allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(gpu.memory_type(req.memory_type_bits, vk::MemoryPropertyFlags::empty())), None)
                .unwrap();
            gpu.device.bind_image_memory(image, mem, 0).unwrap();
            images.push(image);
            memories.push(mem);
        }

        // Preenche as imagens com o padrão conhecido, via buffer.
        let size = (W * H * 4) as u64;
        let (staging, staging_mem) = gpu.host_buffer(size);
        let ptr = gpu.device.map_memory(staging_mem, 0, size, vk::MemoryMapFlags::empty()).unwrap() as *mut u8;
        for y in 0..H {
            for x in 0..W {
                let o = ((y * W + x) * 4) as usize;
                ptr.add(o).copy_from_nonoverlapping(input(x, y).as_ptr(), 4);
            }
        }
        gpu.device.unmap_memory(staging_mem);
        let region = vk::BufferImageCopy::default().image_subresource(layers()).image_extent(vk::Extent3D { width: W, height: H, depth: 1 });
        for &image in &images {
            gpu.run(|cb| {
                transition(&gpu, cb, image, vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL);
                gpu.device.cmd_copy_buffer_to_image(cb, staging, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &[region]);
                transition(&gpu, cb, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, REST);
            });
        }

        // O passe sob teste.
        let target = Target { images: &images, format: FORMAT, extent: vk::Extent2D { width: W, height: H }, rest_layout: REST, queue_family: gpu.family, init_command_buffer: None };
        let mut pass = InvertPass::new(&gpu.device, &gpu.memory, &target).expect("criar o passe");
        for i in 0..images.len() {
            let cbs = [pass.command_buffer(i)];
            gpu.device.queue_submit(gpu.queue, &[vk::SubmitInfo::default().command_buffers(&cbs)], vk::Fence::null()).unwrap();
        }
        gpu.device.queue_wait_idle(gpu.queue).unwrap();

        // Lê de volta e confere cada pixel de cada imagem (tolerância de 1 por arredondamento).
        for &image in &images {
            gpu.run(|cb| gpu.device.cmd_copy_image_to_buffer(cb, image, REST, staging, &[region]));
            let ptr = gpu.device.map_memory(staging_mem, 0, size, vk::MemoryMapFlags::empty()).unwrap() as *const u8;
            for y in 0..H {
                for x in 0..W {
                    let o = ((y * W + x) * 4) as usize;
                    let got = [*ptr.add(o), *ptr.add(o + 1), *ptr.add(o + 2), *ptr.add(o + 3)];
                    let src = input(x, y);
                    let want = [255 - src[0], 255 - src[1], 255 - src[2], src[3]]; // alfa preservado
                    for c in 0..4 {
                        assert!(got[c].abs_diff(want[c]) <= 1, "pixel ({x},{y}) canal {c}: veio {got:?}, esperado {want:?}");
                    }
                }
            }
            gpu.device.unmap_memory(staging_mem);
        }

        // Limpeza (também exercita o destroy; chamar duas vezes não pode quebrar).
        pass.destroy(&gpu.device);
        pass.destroy(&gpu.device);
        gpu.device.destroy_buffer(staging, None);
        gpu.device.free_memory(staging_mem, None);
        for (image, mem) in images.into_iter().zip(memories) {
            gpu.device.destroy_image(image, None);
            gpu.device.free_memory(mem, None);
        }
        gpu.device.destroy_device(None);
        gpu.instance.destroy_instance(None);
    }
}
