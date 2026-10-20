use std::io::Cursor;
use std::path::Path;

use anyhow::{Context, Result};
use image::{DynamicImage, ImageDecoder, ImageReader};

use crate::OutputFormat;

/// A decoded image plus the metadata PhotoKit cares about.
pub struct Decoded {
    pub image: DynamicImage,
    pub format: Option<OutputFormat>,
    pub icc_profile: Option<Vec<u8>>,
    /// Size of the encoded source in bytes.
    pub source_bytes: u64,
}

pub fn decode_file(path: &Path) -> Result<Decoded> {
    let data = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    decode_bytes(&data).with_context(|| format!("cannot decode {}", path.display()))
}

/// Decode, applying EXIF orientation so pixels are always upright.
pub fn decode_bytes(data: &[u8]) -> Result<Decoded> {
    let reader = ImageReader::new(Cursor::new(data)).with_guessed_format()?;
    let format = reader.format().and_then(OutputFormat::from_image_format);
    let mut decoder = reader.into_decoder().context("unrecognized image format")?;
    let icc_profile = decoder.icc_profile().ok().flatten();
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder)?;
    if let Some(o) = orientation {
        image.apply_orientation(o);
    }
    Ok(Decoded { image, format, icc_profile, source_bytes: data.len() as u64 })
}

/// Read only the header to get dimensions; much faster than a full decode.
pub fn probe_dimensions(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}
