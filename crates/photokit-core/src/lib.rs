//! The PhotoKit processing engine shared by the CLI, web server and desktop app.

pub mod batch;
pub mod config;
pub mod naming;
pub mod ops;
pub mod preset;
pub mod rename;

pub use photokit_codecs as codecs;
pub use photokit_codecs::{EncodeOptions, OutputFormat};

pub use ops::{apply_ops, Fit, Flip, Op, ResizeSpec};
