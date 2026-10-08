//! External plugins. Design and roadmap: `docs/plugins/README.md`.
//!
//! All plugin code lives in this module; the core only holds a `PluginManager`
//! in `AppState` and calls into it at a few touch points (`docs/plugins/ROADMAP.md`).
mod api;
mod config;
mod dto;
mod events;
mod manager;
#[cfg(test)]
mod test_support;
mod ui;

pub use api::router;
pub use config::{deserialize_plugins, PluginConfig};
pub use events::publish_playback;
pub use manager::{PluginManager, UiTab};
pub use ui::router as ui_router;
