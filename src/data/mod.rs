pub(crate) mod types;
pub(crate) mod history;
pub(crate) mod cursor_history;
pub(crate) mod cursor_store;
pub(crate) mod io;
pub(crate) mod preview;
pub(crate) mod titles;

#[cfg(test)]
mod tests;

pub use history::load_all_sessions;
#[allow(unused_imports)] // retained for callers/tests that still want Claude-only
pub use history::load_sessions;
pub use preview::{load_chain_preview, load_cursor_preview, load_preview};
pub use titles::{load_custom_title, save_custom_title};
pub use types::{AgentBackend, PreviewMessage, SessionInfo, SessionMeta};
