pub mod app;
pub mod clipboard;
pub mod config;
pub mod error;
pub mod front;
pub mod history;
pub mod hotkey;
mod indicator;
pub mod insert;
mod key_listener;
pub mod permissions;
pub mod tray;
pub mod update;
pub mod word_win;

pub use app::run;
pub use error::{Error, Result};
