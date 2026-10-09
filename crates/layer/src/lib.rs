//! Layer Vulkan implícita do linux-reshade.
//!
//! - [`proc_addr`]: negociação com o loader e as tabelas `vkGet*ProcAddr`
//! - [`instance`], [`device`], [`swapchain`], [`present`]: um módulo por grupo de comandos
//! - [`state`]: dados por instance/device; [`control`]: estado escrito pelo app de controle

mod control;
mod device;
mod instance;
mod log;
mod present;
mod proc_addr;
mod state;
mod swapchain;
