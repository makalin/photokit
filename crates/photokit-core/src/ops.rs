use anyhow::{bail, Result};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    /// Fit inside the box, keeping aspect ratio.
    #[default]
    Contain,
    /// Fill the box, keeping aspect ratio, cropping the overflow (centered).
    Cover,
    /// Stretch to the exact box.
    Fill,
}

impl std::str::FromStr for Fit {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "contain" => Ok(Self::Contain),
            "cover" => Ok(Self::Cover),
            "fill" => Ok(Self::Fill),
            o => Err(format!("unknown fit mode '{o}' (contain, cover, fill)")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ResizeSpec {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Scale factor in percent; used when neither width nor height is set.
    pub percent: Option<f32>,
    pub fit: Fit,
    /// Allow output larger than the input.
    pub upscale: bool,
}

impl ResizeSpec {
    pub fn is_noop(&self) -> bool {
        self.width.is_none() && self.height.is_none() && self.percent.is_none()
    }

    /// Target canvas size and, for `cover`, the scaled size before cropping.
    pub fn plan(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let (wf, hf) = (w as f64, h as f64);
        let (sw, sh) = match (self.width, self.height, self.percent) {
            (Some(tw), Some(th), _) => match self.fit {
                Fit::Fill => (tw as f64, th as f64),
                Fit::Contain => {
                    let s = (tw as f64 / wf).min(th as f64 / hf);
                    (wf * s, hf * s)
                }
                Fit::Cover => {
                    let s = (tw as f64 / wf).max(th as f64 / hf);
                    (wf * s, hf * s)
                }
            },
            (Some(tw), None, _) => (tw as f64, hf * tw as f64 / wf),
            (None, Some(th), _) => (wf * th as f64 / hf, th as f64),
            (None, None, Some(p)) => (wf * p as f64 / 100.0, hf * p as f64 / 100.0),
            (None, None, None) => (wf, hf),
        };
        let (mut sw, mut sh) = (sw, sh);
        let (mut cw, mut ch) = match (self.fit, self.width, self.height) {
            (Fit::Cover, Some(tw), Some(th)) => (tw as f64, th as f64),
            _ => (sw, sh),
        };
        if !self.upscale && self.fit != Fit::Fill && (sw > wf || sh > hf) {
            // Never enlarge: scale the whole plan down so the source is used 1:1.
            let k = (wf / sw).min(hf / sh);
            sw *= k;
            sh *= k;
            cw = (cw * k).min(sw);
            ch = (ch * k).min(sh);
        }
        let r = |v: f64| v.round().max(1.0) as u32;
        let (sw, sh) = (r(sw), r(sh));
        let (cw, ch) = (r(cw).min(sw), r(ch).min(sh));
        (cw, ch, sw, sh)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Flip {
    Horizontal,
    Vertical,
}

/// One editing step. The pipeline is an ordered list of these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    Resize(ResizeSpec),
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Clockwise rotation in degrees: multiples of 90 are lossless.
    Rotate {
        degrees: f32,
    },
    Flip {
        axis: Flip,
    },
    /// Small-angle rotation that crops to remove the empty corners.
    Straighten {
        degrees: f32,
    },
    Grayscale,
}

pub fn apply_ops(mut img: DynamicImage, ops: &[Op]) -> Result<DynamicImage> {
    for op in ops {
        img = apply(img, op)?;
    }
    Ok(img)
}

fn apply(img: DynamicImage, op: &Op) -> Result<DynamicImage> {
    Ok(match op {
        Op::Resize(spec) => resize(&img, spec)?,
        Op::Crop { x, y, width, height } => {
            let (w, h) = img.dimensions();
            if *x >= w || *y >= h || *width == 0 || *height == 0 {
                bail!("crop {width}x{height}+{x}+{y} is outside the {w}x{h} image");
            }
            img.crop_imm(*x, *y, (*width).min(w - x), (*height).min(h - y))
        }
        Op::Rotate { degrees } => {
            let d = degrees.rem_euclid(360.0);
            let near = |t: f32| (d - t).abs() < 0.01;
            if near(0.0) || near(360.0) {
                img
            } else if near(90.0) {
                img.rotate90()
            } else if near(180.0) {
                img.rotate180()
            } else if near(270.0) {
                img.rotate270()
            } else {
                rotate_free(&img, *degrees, false)
            }
        }
        Op::Flip { axis: Flip::Horizontal } => img.fliph(),
        Op::Flip { axis: Flip::Vertical } => img.flipv(),
        Op::Straighten { degrees } => rotate_free(&img, *degrees, true),
        Op::Grayscale => img.grayscale(),
    })
}

/// SIMD resize via `fast_image_resize`; alpha is premultiplied automatically.
pub fn resize(img: &DynamicImage, spec: &ResizeSpec) -> Result<DynamicImage> {
    if spec.is_noop() {
        return Ok(img.clone());
    }
    let (w, h) = img.dimensions();
    let (cw, ch, sw, sh) = spec.plan(w, h);
    let scaled = if (sw, sh) == (w, h) { img.clone() } else { resize_exact(img, sw, sh)? };
    if (cw, ch) == (sw, sh) {
        return Ok(scaled);
    }
    let x = (sw - cw) / 2;
    let y = (sh - ch) / 2;
    Ok(scaled.crop_imm(x, y, cw, ch))
}

pub fn resize_exact(img: &DynamicImage, w: u32, h: u32) -> Result<DynamicImage> {
    // fast_image_resize handles 8-bit layouts; normalize anything exotic first.
    let src = match img {
        DynamicImage::ImageLuma8(_)
        | DynamicImage::ImageLumaA8(_)
        | DynamicImage::ImageRgb8(_)
        | DynamicImage::ImageRgba8(_) => img.clone(),
        _ if img.color().has_alpha() => DynamicImage::ImageRgba8(img.to_rgba8()),
        _ => DynamicImage::ImageRgb8(img.to_rgb8()),
    };
    let mut dst = DynamicImage::new(w, h, src.color());
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new().resize(&src, &mut dst, &opts)?;
    Ok(dst)
}

/// Fast preview-sized copy (bilinear) that never upscales.
pub fn thumbnail(img: &DynamicImage, max_side: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= max_side {
        return img.clone();
    }
    let s = max_side as f64 / w.max(h) as f64;
    let (tw, th) = (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1));
    let src = DynamicImage::ImageRgba8(img.to_rgba8());
    let mut dst = DynamicImage::new(tw, th, src.color());
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
    match Resizer::new().resize(&src, &mut dst, &opts) {
        Ok(()) => dst,
        Err(_) => img.thumbnail(max_side, max_side),
    }
}

/// Rotate by an arbitrary angle with bilinear sampling.
///
/// With `crop`, the result is the largest rectangle of the original aspect
/// ratio that contains no empty corners; otherwise the canvas grows and the
/// corners are transparent.
fn rotate_free(img: &DynamicImage, degrees: f32, crop: bool) -> DynamicImage {
    let src = img.to_rgba8();
    let (w, h) = (src.width() as f64, src.height() as f64);
    let t = (degrees as f64).to_radians();
    let (s, c) = (t.sin().abs(), t.cos().abs());
    let (ow, oh) = if crop {
        let k = (w / (w * c + h * s)).min(h / (w * s + h * c));
        (w * k, h * k)
    } else {
        (w * c + h * s, w * s + h * c)
    };
    let (ow, oh) = ((ow.floor() as u32).max(1), (oh.floor() as u32).max(1));
    let (sin, cos) = t.sin_cos();
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (ocx, ocy) = (ow as f64 / 2.0, oh as f64 / 2.0);
    let out = RgbaImage::from_fn(ow, oh, |x, y| {
        let dx = x as f64 + 0.5 - ocx;
        let dy = y as f64 + 0.5 - ocy;
        // inverse rotation back into source space
        let sx = dx * cos + dy * sin + cx - 0.5;
        let sy = -dx * sin + dy * cos + cy - 0.5;
        bilinear(&src, sx, sy)
    });
    DynamicImage::ImageRgba8(out)
}

fn bilinear(img: &RgbaImage, x: f64, y: f64) -> Rgba<u8> {
    let (w, h) = (img.width() as i64, img.height() as i64);
    if x < -0.5 || y < -0.5 || x > w as f64 - 0.5 || y > h as f64 - 0.5 {
        return Rgba([0, 0, 0, 0]);
    }
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = (x - x0 as f64, y - y0 as f64);
    let px = |xx: i64, yy: i64| img.get_pixel(xx.clamp(0, w - 1) as u32, yy.clamp(0, h - 1) as u32).0;
    let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    let mut out = [0u8; 4];
    for i in 0..4 {
        let top = a[i] as f64 * (1.0 - fx) + b[i] as f64 * fx;
        let bot = c[i] as f64 * (1.0 - fx) + d[i] as f64 * fx;
        out[i] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    Rgba(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: u32, h: u32) -> DynamicImage {
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 0])))
    }

    fn spec(width: Option<u32>, height: Option<u32>, fit: Fit) -> ResizeSpec {
        ResizeSpec { width, height, fit, ..Default::default() }
    }

    #[test]
    fn width_only_keeps_aspect() {
        let out = resize(&img(400, 200), &spec(Some(100), None, Fit::Contain)).unwrap();
        assert_eq!(out.dimensions(), (100, 50));
    }

    #[test]
    fn contain_cover_fill() {
        let src = img(400, 200);
        assert_eq!(resize(&src, &spec(Some(100), Some(100), Fit::Contain)).unwrap().dimensions(), (100, 50));
        assert_eq!(resize(&src, &spec(Some(100), Some(100), Fit::Cover)).unwrap().dimensions(), (100, 100));
        assert_eq!(resize(&src, &spec(Some(100), Some(100), Fit::Fill)).unwrap().dimensions(), (100, 100));
    }

    #[test]
    fn no_upscale_by_default() {
        let src = img(100, 50);
        assert_eq!(resize(&src, &spec(Some(400), None, Fit::Contain)).unwrap().dimensions(), (100, 50));
        let up = ResizeSpec { upscale: true, ..spec(Some(400), None, Fit::Contain) };
        assert_eq!(resize(&src, &up).unwrap().dimensions(), (400, 200));
    }

    #[test]
    fn percent() {
        let s = ResizeSpec { percent: Some(50.0), ..Default::default() };
        assert_eq!(resize(&img(300, 100), &s).unwrap().dimensions(), (150, 50));
    }

    #[test]
    fn rotate_and_crop_ops() {
        let ops = vec![
            Op::Rotate { degrees: 90.0 },
            Op::Crop { x: 10, y: 10, width: 20, height: 30 },
            Op::Flip { axis: Flip::Horizontal },
        ];
        let out = apply_ops(img(200, 100), &ops).unwrap();
        assert_eq!(out.dimensions(), (20, 30));
    }

    #[test]
    fn straighten_keeps_aspect_and_shrinks() {
        let out = apply_ops(img(400, 300), &[Op::Straighten { degrees: 5.0 }]).unwrap();
        let (w, h) = out.dimensions();
        assert!(w < 400 && h < 300);
        assert!(((w as f64 / h as f64) - 4.0 / 3.0).abs() < 0.02);
    }

    #[test]
    fn ops_deserialize_from_json() {
        let ops: Vec<Op> = serde_json::from_str(
            r#"[{"op":"resize","width":800,"fit":"cover","height":600},{"op":"rotate","degrees":90}]"#,
        )
        .unwrap();
        assert_eq!(ops.len(), 2);
    }
}
