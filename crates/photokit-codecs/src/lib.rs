//! Image decoding, encoding and optimization adapters.
//!
//! Everything format-specific lives here so the processing pipeline in
//! `photokit-core` can work purely on decoded pixels.

mod decode;
mod encode;
mod format;

pub use decode::{decode_bytes, decode_file, probe_dimensions, Decoded};
pub use encode::{encode, encode_to_target, EncodeOptions, Encoded};
pub use format::{is_supported_input, OutputFormat, INPUT_EXTENSIONS};
