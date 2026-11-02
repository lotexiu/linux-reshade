//! Layer Vulkan implícita do linux-reshade.
//!
//! - [`proc_addr`]: negociação com o loader e as tabelas `vkGet*ProcAddr`
//! - [`instance`], [`device`], [`swapchain`], [`present`]: um módulo por grupo de comandos
//! - [`effect`]: ciclo de vida do efeito (postfx) de cada swapchain; [`bundle`]: o bundle executado
//! - [`state`]: dados por instance/device; [`control`]: estado escrito pelo app de controle

mod crash;
mod bundle;
mod control;
mod depth;
mod device;
mod effect;
mod instance;
mod log;
mod params;
mod present;
mod proc_addr;
mod screenshot;
mod state;
mod stats;
mod status;
mod swapchain;
