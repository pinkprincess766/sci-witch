//! Everything that happens to a downloaded update **after** it is on disk:
//! unpacking it into a staging directory, checking that what came out looks
//! like a SciWhisper installation, and putting it in place so that a failure
//! leaves the old version working.
//!
//! Discovery and download live in `sciwhisper-shell::update`. They are a
//! different problem with a different threat model — that layer decides what
//! to trust from the network; this one decides what to trust from an archive
//! that has already been verified against its manifest.
//!
//! There is no GUI, no recogniser and no network here on purpose: the helper
//! that replaces a running Windows installation links this crate and nothing
//! else, so "the helper cannot phone home" is a fact the compiler enforces.

pub mod helper;
pub mod install;
pub mod staging;

mod error;

pub use error::{Error, Result};
