//! Core primitives shared by every layer of the backend.
//!
//! Nothing in here knows about the modem, AT commands or any business module:
//! `core` is dependency-free infrastructure (JSON codec, error model, task
//! vocabulary, runtime helpers, crypto helpers).

pub mod error;
pub mod json;
pub mod runtime;
pub mod sha1;
pub mod task;
