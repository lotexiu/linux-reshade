//! Captura de tela: no present em que o daemon pediu uma, copia a imagem do swapchain para a memória do processador (antes e
//! depois dos efeitos) e deixa os pixels crus no estado compartilhado, de onde o daemon faz o PNG (ver `ipc::shot`).
//!
//! A cópia entra na fila entre o jogo e o efeito por semáforos: a cópia "sem efeitos" espera o quadro do jogo, o efeito espera a
//! cópia, e a cópia "com efeitos" espera o efeito; o present espera a última.

use crate::log::log;
use crate::state::DeviceData;
use ash::vk::{self, Handle};
use ipc::shot::{Kind, Order, RawShot, ShotRequest};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Quanto tempo se espera a GPU terminar a cópia (o quadro inteiro do jogo precisa ter terminado).
const COPY_TIMEOUT_NS: u64 = 2_000_000_000;

/// Último pedido tratado e quando o arquivo de pedido foi olhado pela última vez (o present roda centenas de vezes por segundo).
static STATE: Mutex<Option<(u64, Instant)>> = Mutex::new(None);

/// Há um pedido novo? O primeiro olhar só anota o número que já existe (um pedido de antes de o jogo abrir não vale).
pub fn poll() -> Option<ShotRequest> {
    let mut state = STATE.lock().unwrap();
    if state.is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(40)) {
        return None;
    }
    let request = ipc::shot::read_request();
    let handled = state.map(|(n, _)| n);
    let now = Instant::now();
    match (request, handled) {
        (Some(r), None) => {
            *state = Some((r.number, now));
            None
        }
        (None, None) => {
            *state = Some((0, now));
            None
        }
        (Some(r), Some(h)) if r.number > h => {
            *state = Some((r.number, now));
            Some(r)
        }
        (_, Some(h)) => {
            *state = Some((h, now));
            None
        }
    }
}

/// A imagem do swapchain que vai ser apresentada.
#[derive(Clone, Copy)]
pub struct Target {
    pub image: vk::Image,
    pub extent: vk::Extent2D,
    pub format: vk::Format,
}

fn order_of(format: vk::Format) -> Option<Order> {
    match format {
        vk::Format::B8G8R8A8_UNORM | vk::Format::B8G8R8A8_SRGB => Some(Order::Bgra),
        vk::Format::R8G8B8A8_UNORM | vk::Format::R8G8B8A8_SRGB => Some(Order::Rgba),
        vk::Format::A2R10G10B10_UNORM_PACK32 => Some(Order::A2R10G10B10),
        vk::Format::A2B10G10R10_UNORM_PACK32 => Some(Order::A2B10G10R10),
        _ => None,
    }
}

/// Uma cópia em andamento: os recursos dela e o semáforo que o próximo passo espera.
pub struct Capture {
    pub semaphore: vk::Semaphore,
    pool: vk::CommandPool,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    fence: vk::Fence,
    target: Target,
    size: u64,
    /// O número do pedido (as duas capturas de um mesmo pedido têm o mesmo).
    number: u64,
}

fn host_memory_type(data: &DeviceData, bits: u32) -> Option<u32> {
    let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
    (0..data.memory.memory_type_count).find(|i| bits & (1 << i) != 0 && data.memory.memory_types[*i as usize].property_flags.contains(wanted))
}

/// Enfileira a cópia da imagem (esperando `waits`). `None` (e o motivo no log) se não deu.
///
/// # Safety
/// `queue` é a fila do present e `target.image` está em `PRESENT_SRC_KHR`.
pub unsafe fn capture(data: &DeviceData, queue: vk::Queue, family: u32, target: Target, waits: &[vk::Semaphore], number: u64) -> Option<Capture> {
    if order_of(target.format).is_none() {
        log!("captura de tela: formato {} não suportado (8 e 10 bits por canal)", target.format.as_raw());
        return None;
    }
    let device = &data.device;
    let size = u64::from(target.extent.width) * u64::from(target.extent.height) * 4;
    unsafe {
        let mut made = Capture { semaphore: vk::Semaphore::null(), pool: vk::CommandPool::null(), buffer: vk::Buffer::null(), memory: vk::DeviceMemory::null(), fence: vk::Fence::null(), target, size, number };
        let result = (|| -> Result<(), String> {
            let fail = |what: &str, e: vk::Result| format!("{what}: erro {}", e.as_raw());
            made.buffer = device.create_buffer(&vk::BufferCreateInfo::default().size(size).usage(vk::BufferUsageFlags::TRANSFER_DST), None).map_err(|e| fail("buffer", e))?;
            let req = device.get_buffer_memory_requirements(made.buffer);
            let type_index = host_memory_type(data, req.memory_type_bits).ok_or("no CPU-visible memory")?;
            made.memory = device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index), None).map_err(|e| fail("memory", e))?;
            device.bind_buffer_memory(made.buffer, made.memory, 0).map_err(|e| fail("bind", e))?;
            made.pool = device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(family).flags(vk::CommandPoolCreateFlags::TRANSIENT), None).map_err(|e| fail("pool", e))?;
            let cb = device.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(made.pool).command_buffer_count(1)).map_err(|e| fail("command buffer", e))?[0];
            let init = (data.set_loader_data)(device.handle(), cb.as_raw() as *mut std::ffi::c_void);
            if init != vk::Result::SUCCESS {
                return Err(format!("loader data: erro {}", init.as_raw()));
            }
            made.fence = device.create_fence(&vk::FenceCreateInfo::default(), None).map_err(|e| fail("fence", e))?;
            made.semaphore = device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).map_err(|e| fail("semaphore", e))?;

            device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)).map_err(|e| fail("begin", e))?;
            let range = vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, base_mip_level: 0, level_count: 1, base_array_layer: 0, layer_count: 1 };
            let barrier = |old, new, src, dst| {
                vk::ImageMemoryBarrier::default()
                    .image(target.image)
                    .old_layout(old)
                    .new_layout(new)
                    .src_access_mask(src)
                    .dst_access_mask(dst)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .subresource_range(range)
            };
            let all = vk::PipelineStageFlags::ALL_COMMANDS;
            device.cmd_pipeline_barrier(cb, all, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[], &[barrier(vk::ImageLayout::PRESENT_SRC_KHR, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::AccessFlags::MEMORY_WRITE, vk::AccessFlags::TRANSFER_READ)]);
            let region = vk::BufferImageCopy::default()
                .image_subresource(vk::ImageSubresourceLayers { aspect_mask: vk::ImageAspectFlags::COLOR, mip_level: 0, base_array_layer: 0, layer_count: 1 })
                .image_extent(vk::Extent3D { width: target.extent.width, height: target.extent.height, depth: 1 });
            device.cmd_copy_image_to_buffer(cb, target.image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, made.buffer, &[region]);
            device.cmd_pipeline_barrier(cb, vk::PipelineStageFlags::TRANSFER, all, vk::DependencyFlags::empty(), &[], &[], &[barrier(vk::ImageLayout::TRANSFER_SRC_OPTIMAL, vk::ImageLayout::PRESENT_SRC_KHR, vk::AccessFlags::TRANSFER_READ, vk::AccessFlags::MEMORY_READ)]);
            device.end_command_buffer(cb).map_err(|e| fail("end", e))?;

            let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
            let cbs = [cb];
            let signal = [made.semaphore];
            let submit = vk::SubmitInfo::default().wait_semaphores(waits).wait_dst_stage_mask(&stages).command_buffers(&cbs).signal_semaphores(&signal);
            device.queue_submit(queue, &[submit], made.fence).map_err(|e| fail("submit", e))
        })();
        match result {
            Ok(()) => Some(made),
            Err(why) => {
                log!("captura de tela falhou: {why}");
                // Nada foi enviado (ou o envio falhou): os recursos saem agora. O semáforo, se existe, nunca foi usado.
                destroy(device, &mut made, true);
                None
            }
        }
    }
}

unsafe fn destroy(device: &ash::Device, c: &mut Capture, with_semaphore: bool) {
    unsafe {
        if c.fence != vk::Fence::null() {
            device.destroy_fence(std::mem::take(&mut c.fence), None);
        }
        if c.pool != vk::CommandPool::null() {
            device.destroy_command_pool(std::mem::take(&mut c.pool), None);
        }
        if c.buffer != vk::Buffer::null() {
            device.destroy_buffer(std::mem::take(&mut c.buffer), None);
        }
        if c.memory != vk::DeviceMemory::null() {
            device.free_memory(std::mem::take(&mut c.memory), None);
        }
        if with_semaphore && c.semaphore != vk::Semaphore::null() {
            device.destroy_semaphore(std::mem::take(&mut c.semaphore), None);
        }
    }
}

/// Espera a cópia, grava os pixels crus e libera tudo. O semáforo (que o present ainda espera) é aposentado, não destruído.
///
/// # Safety
/// `capture` veio de [`capture`] com o mesmo dispositivo.
pub unsafe fn finish(data: &DeviceData, mut capture: Capture, kind: Kind) {
    let device = &data.device;
    unsafe {
        let done = device.wait_for_fences(&[capture.fence], true, COPY_TIMEOUT_NS).is_ok();
        if done {
            match device.map_memory(capture.memory, 0, capture.size, vk::MemoryMapFlags::empty()) {
                Ok(ptr) => {
                    let pixels = std::slice::from_raw_parts(ptr.cast::<u8>(), capture.size as usize).to_vec();
                    device.unmap_memory(capture.memory);
                    let shot = RawShot {
                        game: crate::status::game_id(),
                        kind,
                        width: capture.target.extent.width,
                        height: capture.target.extent.height,
                        order: order_of(capture.target.format).unwrap_or(Order::Bgra),
                        pixels,
                    };
                    match ipc::shot::write_raw_in(&ipc::state_dir(), &shot, u128::from(capture.number)) {
                        Ok(path) => log!("captura de tela ({kind:?}) gravada: {}", path.display()),
                        Err(e) => log!("captura de tela: não consegui gravar: {e}"),
                    }
                }
                Err(e) => log!("captura de tela: não consegui ler a memória: erro {}", e.as_raw()),
            }
        } else {
            log!("captura de tela: a GPU não terminou a cópia a tempo");
        }
        let semaphore = std::mem::take(&mut capture.semaphore);
        // Se a espera falhou a GPU pode ainda usar o buffer: ele vai embora só com a fence sinalizada (do contrário vaza, de propósito).
        if done {
            destroy(device, &mut capture, false);
        }
        if semaphore != vk::Semaphore::null() {
            crate::effect::retire_semaphores(device, std::iter::once(semaphore));
        }
    }
}

/// Descarta a cópia sem gravar nada (o quadro sem efeitos que ninguém pediu): espera a GPU e libera tudo.
///
/// # Safety
/// Como [`finish`].
pub unsafe fn finish_discard(data: &DeviceData, mut capture: Capture) {
    let device = &data.device;
    unsafe {
        let done = device.wait_for_fences(&[capture.fence], true, COPY_TIMEOUT_NS).is_ok();
        let semaphore = std::mem::take(&mut capture.semaphore);
        if done {
            destroy(device, &mut capture, false);
        }
        if semaphore != vk::Semaphore::null() {
            crate::effect::retire_semaphores(device, std::iter::once(semaphore));
        }
    }
}
