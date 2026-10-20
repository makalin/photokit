use std::io::Cursor;

use anyhow::{bail, Context, Result};
use image::codecs::avif::AvifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::OutputFormat;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EncodeOptions {
    pub format: OutputFormat,
    /// 1-100, used by lossy formats.
    pub quality: u8,
    /// WebP only: encode losslessly.
    pub lossless: bool,
    /// PNG only: oxipng effort 0 (off) to 6.
    pub png_level: u8,
    /// Keep the source ICC profile where the encoder supports it.
    pub keep_icc: bool,
    /// Background used when flattening alpha for formats without it.
    pub background: [u8; 3],
    /// AVIF encoder speed 1 (slow, small) to 10 (fast).
    pub avif_speed: u8,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            format: OutputFormat::Jpeg,
            quality: 85,
            lossless: false,
            png_level: 2,
            keep_icc: false,
            background: [255, 255, 255],
            avif_speed: 8,
        }
    }
}

pub struct Encoded {
    pub data: Vec<u8>,
    pub format: OutputFormat,
    /// The quality actually used (differs from the request with a size target).
    pub quality: u8,
}

pub fn encode(img: &DynamicImage, opts: &EncodeOptions, icc: Option<&[u8]>) -> Result<Vec<u8>> {
    let icc = if opts.keep_icc { icc.map(<[u8]>::to_vec) } else { None };
    let q = opts.quality.clamp(1, 100);
    let mut buf = Vec::new();
    let (w, h) = (img.width(), img.height());
    match opts.format {
        OutputFormat::Jpeg => {
            let rgb = flatten(img, opts.background);
            let mut enc = JpegEncoder::new_with_quality(&mut buf, q);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p);
            }
            enc.write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)?;
        }
        OutputFormat::Png => {
            {
                let mut enc = PngEncoder::new_with_quality(&mut buf, CompressionType::Fast, FilterType::Adaptive);
                if let Some(p) = icc {
                    let _ = enc.set_icc_profile(p);
                }
                if img.color().has_alpha() {
                    enc.write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)?;
                } else {
                    enc.write_image(img.to_rgb8().as_raw(), w, h, ExtendedColorType::Rgb8)?;
                }
            }
            if opts.png_level > 0 {
                let mut o = oxipng::Options::from_preset(opts.png_level.min(6));
                o.strip = oxipng::StripChunks::Safe;
                buf = oxipng::optimize_from_memory(&buf, &o).context("PNG optimization failed")?;
            }
        }
        OutputFormat::Webp if opts.lossless => {
            let mut enc = WebPEncoder::new_lossless(&mut buf);
            if let Some(p) = icc {
                let _ = enc.set_icc_profile(p);
            }
            enc.write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)?;
        }
        OutputFormat::Webp => {
            let data = if img.color().has_alpha() {
                let rgba = img.to_rgba8();
                webp::Encoder::from_rgba(rgba.as_raw(), w, h).encode(q as f32).to_vec()
            } else {
                let rgb = img.to_rgb8();
                webp::Encoder::from_rgb(rgb.as_raw(), w, h).encode(q as f32).to_vec()
            };
            buf = data;
        }
        OutputFormat::Avif => {
            let enc = AvifEncoder::new_with_speed_quality(&mut buf, opts.avif_speed.clamp(1, 10), q);
            enc.write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)?;
        }
        OutputFormat::Gif => {
            img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Gif)?;
        }
        OutputFormat::Bmp => {
            DynamicImage::ImageRgb8(flatten(img, opts.background))
                .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Bmp)?;
        }
        OutputFormat::Tiff => {
            let img = if img.color().has_alpha() {
                DynamicImage::ImageRgba8(img.to_rgba8())
            } else {
                DynamicImage::ImageRgb8(img.to_rgb8())
            };
            img.write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Tiff)?;
        }
    }
    if buf.is_empty() {
        bail!("encoder produced no data");
    }
    Ok(buf)
}

/// Encode, binary-searching the quality so the result fits `target_bytes`.
///
/// Lossless formats ignore the target and are encoded once. If even the
/// lowest quality is too big, the smallest result is returned.
pub fn encode_to_target(
    img: &DynamicImage,
    opts: &EncodeOptions,
    icc: Option<&[u8]>,
    target_bytes: Option<u64>,
) -> Result<Encoded> {
    let Some(target) = target_bytes.filter(|_| opts.format.is_lossy() && !opts.lossless) else {
        let data = encode(img, opts, icc)?;
        return Ok(Encoded { data, format: opts.format, quality: opts.quality });
    };

    let mut o = opts.clone();
    let (mut lo, mut hi) = (5u8, opts.quality.clamp(5, 100));
    let mut best: Option<Encoded> = None;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        o.quality = mid;
        let data = encode(img, &o, icc)?;
        if data.len() as u64 <= target {
            best = Some(Encoded { data, format: o.format, quality: mid });
            lo = mid + 1;
        } else {
            if mid == 5 {
                best.get_or_insert(Encoded { data, format: o.format, quality: mid });
            }
            if mid == 0 {
                break;
            }
            hi = mid - 1;
        }
    }
    best.context("could not encode image")
}

fn flatten(img: &DynamicImage, bg: [u8; 3]) -> RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let p = rgba.get_pixel(x, y).0;
        let a = p[3] as u32;
        let mix = |c: u8, b: u8| ((c as u32 * a + b as u32 * (255 - a) + 127) / 255) as u8;
        Rgb([mix(p[0], bg[0]), mix(p[1], bg[1]), mix(p[2], bg[2])])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbaImage;

    fn sample() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_fn(64, 48, |x, y| {
            image::Rgba([(x * 4) as u8, (y * 5) as u8, 128, if x < 32 { 255 } else { 100 }])
        }))
    }

    #[test]
    fn every_format_round_trips_dimensions() {
        let img = sample();
        for format in OutputFormat::ALL {
            let opts = EncodeOptions { format, ..Default::default() };
            let data = encode(&img, &opts, None).unwrap();
            if format == OutputFormat::Avif {
                continue; // AVIF decoding is not enabled
            }
            let back = image::load_from_memory(&data).unwrap();
            assert_eq!((back.width(), back.height()), (64, 48), "{format}");
        }
    }

    #[test]
    fn target_size_is_respected() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_fn(400, 300, |x, y| {
            Rgb([(x ^ y) as u8, (x * y) as u8, (x + y) as u8])
        }));
        let opts = EncodeOptions { quality: 95, ..Default::default() };
        let full = encode(&img, &opts, None).unwrap().len() as u64;
        let target = full / 2;
        let out = encode_to_target(&img, &opts, None, Some(target)).unwrap();
        assert!(out.data.len() as u64 <= target);
        assert!(out.quality < 95);
    }
}
