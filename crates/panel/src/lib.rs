//! O modelo da interface gráfica: o que mostrar e o que cada ação faz, sem saber desenhar.
//!
//! A interface (hoje GTK4, em outro crate) só apresenta isto e repassa os cliques; assim a lógica é
//! testável sem display, e trocar de toolkit não obriga a reescrever nada daqui.

pub mod control;
pub mod depth_note;
pub mod home;
pub mod macro_defs;
pub mod model;

pub use home::{EffectGroup, macros_text, GameEntry, Home, HomeSnapshot, PresetInfo, TechniqueView, start_analysis};
pub use depth_note::{DepthChoice, DepthClear, DepthNote};
pub use control::{Control, Kind, controls_of};
pub use model::{Model, Selection, Snapshot};
