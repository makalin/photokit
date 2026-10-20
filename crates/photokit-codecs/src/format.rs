use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Extensions PhotoKit can decode.
pub const INPUT_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif", "tiff"];

pub fn is_supported_input(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| INPUT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Jpeg,
    Png,
    Webp,
    Avif,
    Gif,
    Bmp,
    Tiff,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 7] =
        [Self::Jpeg, Self::Png, Self::Webp, Self::Avif, Self::Gif, Self::Bmp, Self::Tiff];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
        }
    }

    pub fn mime(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Webp => "image/webp",
            Self::Avif => "image/avif",
            Self::Gif => "image/gif",
            Self::Bmp => "image/bmp",
            Self::Tiff => "image/tiff",
        }
    }

    /// Formats where a quality setting changes the output.
    pub fn is_lossy(self) -> bool {
        matches!(self, Self::Jpeg | Self::Webp | Self::Avif)
    }

    pub fn supports_alpha(self) -> bool {
        !matches!(self, Self::Jpeg | Self::Bmp)
    }

    pub fn from_image_format(f: image::ImageFormat) -> Option<Self> {
        use image::ImageFormat as F;
        Some(match f {
            F::Jpeg => Self::Jpeg,
            F::Png => Self::Png,
            F::WebP => Self::Webp,
            F::Avif => Self::Avif,
            F::Gif => Self::Gif,
            F::Bmp => Self::Bmp,
            F::Tiff => Self::Tiff,
            _ => return None,
        })
    }

    pub fn from_path(path: &Path) -> Option<Self> {
        path.extension()?.to_str()?.parse().ok()
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Webp => "webp",
            Self::Avif => "avif",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
        })
    }
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Self::Jpeg,
            "png" => Self::Png,
            "webp" => Self::Webp,
            "avif" => Self::Avif,
            "gif" => Self::Gif,
            "bmp" => Self::Bmp,
            "tif" | "tiff" => Self::Tiff,
            other => return Err(format!("unsupported output format '{other}'")),
        })
    }
}
