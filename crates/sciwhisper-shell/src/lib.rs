pub mod app;
pub mod clipboard;
pub mod config;
pub mod error;
pub mod front;
pub mod history;
pub mod hotkey;
pub mod insert;
mod key_listener;
pub mod permissions;
pub mod tray;
pub mod word_win;

pub use app::run;
pub use error::{Error, Result};
