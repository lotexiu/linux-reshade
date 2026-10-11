//! Aplicativo de teste da layer: renderização dinâmica com limpezas de profundidade DENTRO do passe (`vkCmdClearAttachments`),
//! como o DXVK faz, num swapchain "headless". Serve para rodar com a validation layer e a layer ligada
//! (`scripts/dev.sh smoke-dynamic`): confere que partir o passe numa limpeza (fechar, copiar a profundidade, reabrir)
//! não gera erros de validação.

use ash::{ext, khr, vk};
use std::ffi::CStr;

const COLOR: vk::Format = vk::Format::B8G8R8A8_UNORM;
const DEPTH: vk::Format = vk::Format::D32_SFLOAT_S8_UINT;
const SIZE: vk::Extent2D = vk::Extent2D { width: 256, height: 256 };
const FRAMES: usize = 150;

/// `DYN_MSAA=4`: renderiza com MSAA (cor e profundidade com 4 amostras, cor resolvida no swapchain), como um jogo em qualidade máxima.
fn samples() -> vk::SampleCountFlags {
    match std::env::var("DYN_MSAA").ok().as_deref() {
        Some("2") => vk::SampleCountFlags::TYPE_2,
        Some("4") => vk::SampleCountFlags::TYPE_4,
        Some("8") => vk::SampleCountFlags::TYPE_8,
        _ => vk::SampleCountFlags::TYPE_1,
    }
}

fn spv(bytes: &[u8]) -> Vec<u32> {
    ash::util::read_spv(&mut std::io::Cursor::new(bytes)).expect("SPIR-V")
}

fn main() {
    unsafe { run() }
}

unsafe fn run() {
    unsafe {
        let entry = ash::Entry::load().expect("loader Vulkan");
        let app = vk::ApplicationInfo::default().application_name(c"dynclear").api_version(if std::env::var_os("DYN_API14").is_some() { vk::make_api_version(0, 1, 4, 0) } else { vk::make_api_version(0, 1, 3, 0) });
        let extensions = [khr::surface::NAME.as_ptr(), ext::headless_surface::NAME.as_ptr()];
        let instance = entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app).enabled_extension_names(&extensions), None).expect("instância");
        let hardware = std::env::var_os("DYN_HARDWARE").is_some_and(|v| v != "0");
        let physical = instance
            .enumerate_physical_devices()
            .unwrap()
            .into_iter()
            .find(|&p| {
                let kind = instance.get_physical_device_properties(p).device_type;
                if hardware { kind == vk::PhysicalDeviceType::DISCRETE_GPU } else { kind == vk::PhysicalDeviceType::CPU }
            })
            .expect("dispositivo");
        let surface_fn = khr::surface::Instance::new(&entry, &instance);
        let headless = ext::headless_surface::Instance::new(&entry, &instance);
        let surface = headless.create_headless_surface(&vk::HeadlessSurfaceCreateInfoEXT::default(), None).expect("superfície headless");
        let family = instance
            .get_physical_device_queue_family_properties(physical)
            .iter()
            .enumerate()
            .position(|(i, q)| q.queue_flags.contains(vk::QueueFlags::GRAPHICS) && surface_fn.get_physical_device_surface_support(physical, i as u32, surface).unwrap_or(false))
            .expect("fila gráfica") as u32;
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
        let mut v13 = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true);
        let device_extensions = [khr::swapchain::NAME.as_ptr()];
        let device = instance
            .create_device(physical, &vk::DeviceCreateInfo::default().queue_create_infos(&queues).enabled_extension_names(&device_extensions).push_next(&mut v13), None)
            .expect("dispositivo lógico");
        let queue = device.get_device_queue(family, 0);
        let memory = instance.get_physical_device_memory_properties(physical);
        let swapchain_fn = khr::swapchain::Device::new(&instance, &device);

        // Swapchain.
        let caps = surface_fn.get_physical_device_surface_capabilities(physical, surface).unwrap();
        let swapchain = swapchain_fn
            .create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(surface)
                    .min_image_count(caps.min_image_count.max(2))
                    .image_format(COLOR)
                    .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                    .image_extent(SIZE)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
                    .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
                    .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                    .present_mode(vk::PresentModeKHR::FIFO)
                    .clipped(true),
                None,
            )
            .expect("swapchain");
        let images = swapchain_fn.get_swapchain_images(swapchain).unwrap();
        let color_views: Vec<vk::ImageView> = images
            .iter()
            .map(|&image| {
                device
                    .create_image_view(&vk::ImageViewCreateInfo::default().image(image).view_type(vk::ImageViewType::TYPE_2D).format(COLOR).subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, level_count: 1, layer_count: 1, ..Default::default() }), None)
                    .unwrap()
            })
            .collect();

        // Profundidade (como um jogo a cria: só `DEPTH_STENCIL_ATTACHMENT`; a layer acrescenta o resto).
        let depth_image = device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(DEPTH)
                    .extent(vk::Extent3D { width: SIZE.width, height: SIZE.height, depth: 1 })
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(samples())
                    .tiling(vk::ImageTiling::OPTIMAL)
                    .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
                    .initial_layout(vk::ImageLayout::UNDEFINED),
                None,
            )
            .unwrap();
        let req = device.get_image_memory_requirements(depth_image);
        let type_index = (0..memory.memory_type_count).find(|&i| req.memory_type_bits & (1 << i) != 0 && memory.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)).unwrap_or(0);
        let depth_memory = device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index), None).unwrap();
        device.bind_image_memory(depth_image, depth_memory, 0).unwrap();
        let depth_view = device
            .create_image_view(&vk::ImageViewCreateInfo::default().image(depth_image).view_type(vk::ImageViewType::TYPE_2D).format(DEPTH).subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL, level_count: 1, layer_count: 1, ..Default::default() }), None)
            .unwrap();

        // Cor MSAA (só com `DYN_MSAA`): resolve para a imagem do swapchain.
        let msaa = samples() != vk::SampleCountFlags::TYPE_1;
        let (msaa_image, msaa_memory, msaa_view) = if msaa {
            let image = device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(COLOR)
                        .extent(vk::Extent3D { width: SIZE.width, height: SIZE.height, depth: 1 })
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(samples())
                        .tiling(vk::ImageTiling::OPTIMAL)
                        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
                        .initial_layout(vk::ImageLayout::UNDEFINED),
                    None,
                )
                .unwrap();
            let req = device.get_image_memory_requirements(image);
            let type_index = (0..memory.memory_type_count).find(|&i| req.memory_type_bits & (1 << i) != 0 && memory.memory_types[i as usize].property_flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)).unwrap_or(0);
            let mem = device.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(type_index), None).unwrap();
            device.bind_image_memory(image, mem, 0).unwrap();
            let view = device
                .create_image_view(&vk::ImageViewCreateInfo::default().image(image).view_type(vk::ImageViewType::TYPE_2D).format(COLOR).subresource_range(vk::ImageSubresourceRange { aspect_mask: vk::ImageAspectFlags::COLOR, level_count: 1, layer_count: 1, ..Default::default() }), None)
                .unwrap();
            (image, mem, view)
        } else {
            (vk::Image::null(), vk::DeviceMemory::null(), vk::ImageView::null())
        };

        // Pipeline.
        let vert = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&spv(include_bytes!("data/tri.vert.spv"))), None).unwrap();
        let frag = device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&spv(include_bytes!("data/tri.frag.spv"))), None).unwrap();
        let layout = device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None).unwrap();
        let main: &CStr = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::VERTEX).module(vert).name(main),
            vk::PipelineShaderStageCreateInfo::default().stage(vk::ShaderStageFlags::FRAGMENT).module(frag).name(main),
        ];
        let blend_attachments = [vk::PipelineColorBlendAttachmentState::default().color_write_mask(vk::ColorComponentFlags::RGBA)];
        let color_formats = [COLOR];
        let mut rendering = vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&color_formats).depth_attachment_format(DEPTH);
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let pipeline = device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[vk::GraphicsPipelineCreateInfo::default()
                    .stages(&stages)
                    .vertex_input_state(&vk::PipelineVertexInputStateCreateInfo::default())
                    .input_assembly_state(&vk::PipelineInputAssemblyStateCreateInfo::default().topology(vk::PrimitiveTopology::TRIANGLE_LIST))
                    .viewport_state(&vk::PipelineViewportStateCreateInfo::default().viewport_count(1).scissor_count(1))
                    .rasterization_state(&vk::PipelineRasterizationStateCreateInfo::default().polygon_mode(vk::PolygonMode::FILL).cull_mode(vk::CullModeFlags::NONE).line_width(1.0))
                    .multisample_state(&vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(samples()))
                    .depth_stencil_state(&vk::PipelineDepthStencilStateCreateInfo::default().depth_test_enable(true).depth_write_enable(true).depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL))
                    .color_blend_state(&vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attachments))
                    .dynamic_state(&vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic))
                    .layout(layout)
                    .push_next(&mut rendering)],
                None,
            )
            .map_err(|(_, e)| e)
            .expect("pipeline")[0];

        let pool = device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(family).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER), None).unwrap();
        let cb = device.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(pool).command_buffer_count(1)).unwrap()[0];
        let acquire = device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).unwrap();
        // Um semáforo de "pronto" por imagem do swapchain: a apresentação o segura até a imagem voltar.
        let render_done: Vec<vk::Semaphore> = images.iter().map(|_| device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).unwrap()).collect();
        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None).unwrap();

        let barrier = |image: vk::Image, aspect: vk::ImageAspectFlags, old: vk::ImageLayout, new: vk::ImageLayout| {
            vk::ImageMemoryBarrier::default()
                .image(image)
                .old_layout(old)
                .new_layout(new)
                .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .subresource_range(vk::ImageSubresourceRange { aspect_mask: aspect, level_count: 1, layer_count: 1, ..Default::default() })
        };
        let area = vk::Rect2D { offset: vk::Offset2D::default(), extent: SIZE };
        let viewport = vk::Viewport { x: 0.0, y: 0.0, width: SIZE.width as f32, height: SIZE.height as f32, min_depth: 0.0, max_depth: 1.0 };
        let all = vk::PipelineStageFlags::ALL_COMMANDS;

        for frame in 0..FRAMES {
            let (index, _) = swapchain_fn.acquire_next_image(swapchain, u64::MAX, acquire, vk::Fence::null()).expect("acquire");
            let color = images[index as usize];
            device.reset_command_buffer(cb, vk::CommandBufferResetFlags::empty()).unwrap();
            device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default()).unwrap();
            let depth_old = if frame == 0 { vk::ImageLayout::UNDEFINED } else { vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL };
            device.cmd_pipeline_barrier(
                cb,
                all,
                all,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier(color, vk::ImageAspectFlags::COLOR, vk::ImageLayout::UNDEFINED, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL), barrier(depth_image, vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL, depth_old, vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)],
            );
            if msaa {
                let old = if frame == 0 { vk::ImageLayout::UNDEFINED } else { vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL };
                device.cmd_pipeline_barrier(cb, all, all, vk::DependencyFlags::empty(), &[], &[], &[barrier(msaa_image, vk::ImageAspectFlags::COLOR, old, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)]);
            }
            let mut color_info = vk::RenderingAttachmentInfo::default()
                .image_view(if msaa { msaa_view } else { color_views[index as usize] })
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(if msaa { vk::AttachmentStoreOp::DONT_CARE } else { vk::AttachmentStoreOp::STORE })
                .clear_value(vk::ClearValue { color: vk::ClearColorValue { float32: [0.0, 0.0, 0.0, 1.0] } });
            if msaa {
                color_info = color_info.resolve_mode(vk::ResolveModeFlags::AVERAGE).resolve_image_view(color_views[index as usize]).resolve_image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
            }
            let color_attachment = [color_info];
            let depth_attachment = vk::RenderingAttachmentInfo::default()
                .image_view(depth_view)
                .image_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue { depth_stencil: vk::ClearDepthStencilValue { depth: 0.0, stencil: 0 } });
            device.cmd_begin_rendering(cb, &vk::RenderingInfo::default().render_area(area).layer_count(1).color_attachments(&color_attachment).depth_attachment(&depth_attachment));
            device.cmd_bind_pipeline(cb, vk::PipelineBindPoint::GRAPHICS, pipeline);
            device.cmd_set_viewport(cb, 0, &[viewport]);
            device.cmd_set_scissor(cb, 0, &[area]);
            let clear_depth = [vk::ClearAttachment { aspect_mask: vk::ImageAspectFlags::DEPTH, color_attachment: 0, clear_value: vk::ClearValue { depth_stencil: vk::ClearDepthStencilValue { depth: 0.0, stencil: 0 } } }];
            let rect = [vk::ClearRect { rect: area, base_array_layer: 0, layer_count: 1 }];
            // Três segmentos separados por limpezas de profundidade dentro do passe: 5, 3 e 1 desenhos.
            for _ in 0..5 {
                device.cmd_draw(cb, 3, 1, 0, 0);
            }
            device.cmd_clear_attachments(cb, &clear_depth, &rect);
            for _ in 0..3 {
                device.cmd_draw(cb, 3, 1, 0, 0);
            }
            device.cmd_clear_attachments(cb, &clear_depth, &rect);
            device.cmd_draw(cb, 3, 1, 0, 0);
            device.cmd_end_rendering(cb);
            device.cmd_pipeline_barrier(cb, all, all, vk::DependencyFlags::empty(), &[], &[], &[barrier(color, vk::ImageAspectFlags::COLOR, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::ImageLayout::PRESENT_SRC_KHR)]);
            device.end_command_buffer(cb).unwrap();

            let waits = [acquire];
            let stages_wait = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
            let cbs = [cb];
            let signals = [render_done[index as usize]];
            device.queue_submit(queue, &[vk::SubmitInfo::default().wait_semaphores(&waits).wait_dst_stage_mask(&stages_wait).command_buffers(&cbs).signal_semaphores(&signals)], fence).unwrap();
            let swapchains = [swapchain];
            let indices = [index];
            let _ = swapchain_fn.queue_present(queue, &vk::PresentInfoKHR::default().wait_semaphores(&signals).swapchains(&swapchains).image_indices(&indices));
            device.wait_for_fences(&[fence], true, u64::MAX).unwrap();
            device.reset_fences(&[fence]).unwrap();
        }
        device.device_wait_idle().unwrap();
        println!("dynamic_clear: {FRAMES} quadros");

        device.destroy_fence(fence, None);
        for s in render_done {
            device.destroy_semaphore(s, None);
        }
        device.destroy_semaphore(acquire, None);
        device.destroy_command_pool(pool, None);
        device.destroy_pipeline(pipeline, None);
        device.destroy_pipeline_layout(layout, None);
        device.destroy_shader_module(frag, None);
        device.destroy_shader_module(vert, None);
        if msaa {
            device.destroy_image_view(msaa_view, None);
            device.destroy_image(msaa_image, None);
            device.free_memory(msaa_memory, None);
        }
        device.destroy_image_view(depth_view, None);
        device.destroy_image(depth_image, None);
        device.free_memory(depth_memory, None);
        for v in color_views {
            device.destroy_image_view(v, None);
        }
        swapchain_fn.destroy_swapchain(swapchain, None);
        device.destroy_device(None);
        surface_fn.destroy_surface(surface, None);
        instance.destroy_instance(None);
    }
}
