//! Export presets: a resize spec plus output settings, stored as TOML.

use std::path::Path;

use anyhow::{Context, Result};
use photokit_codecs::{EncodeOptions, OutputFormat};
use serde::{Deserialize, Serialize};

use crate::ops::{Op, ResizeSpec};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputSpec {
    /// `None` keeps the source format.
    pub format: Option<OutputFormat>,
    pub quality: Option<u8>,
    pub lossless: bool,
    pub png_level: Option<u8>,
    pub target_kb: Option<u64>,
    pub keep_metadata: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub resize: Option<ResizeSpec>,
    pub output: OutputSpec,
    /// Output file name template (see `naming::render`).
    pub name_pattern: Option<String>,
}

impl Preset {
    pub fn ops(&self) -> Vec<Op> {
        self.resize.clone().filter(|r| !r.is_noop()).map(Op::Resize).into_iter().collect()
    }

    pub fn encode_options(&self, fallback: OutputFormat) -> EncodeOptions {
        let d = EncodeOptions::default();
        EncodeOptions {
            format: self.output.format.unwrap_or(fallback),
            quality: self.output.quality.unwrap_or(d.quality),
            lossless: self.output.lossless,
            png_level: self.output.png_level.unwrap_or(d.png_level),
            keep_icc: self.output.keep_metadata,
            ..d
        }
    }
}

const BUILTIN: &[&str] = &[
    include_str!("../../../presets/web.toml"),
    include_str!("../../../presets/social.toml"),
    include_str!("../../../presets/story.toml"),
    include_str!("../../../presets/thumbnail.toml"),
    include_str!("../../../presets/email.toml"),
    include_str!("../../../presets/archive.toml"),
];

/// Built-in presets followed by user presets from the config directory.
/// A user preset with the same name replaces the built-in one.
pub fn all() -> Vec<Preset> {
    let mut out: Vec<Preset> = BUILTIN.iter().filter_map(|s| toml::from_str(s).ok()).collect();
    if let Some(dir) = crate::config::config_dir().map(|d| d.join("presets")) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                if let Ok(p) = load(&e.path()) {
                    out.retain(|b| b.name != p.name);
                    out.push(p);
                }
            }
        }
    }
    out
}

pub fn find(name: &str) -> Option<Preset> {
    all().into_iter().find(|p| p.name.eq_ignore_ascii_case(name))
}

pub fn load(path: &Path) -> Result<Preset> {
    let text = std::fs::read_to_string(path)?;
    let mut p: Preset = toml::from_str(&text).with_context(|| format!("invalid preset {}", path.display()))?;
    if p.name.is_empty() {
        p.name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    #[test]
    fn builtins_parse() {
        let all = super::all();
        assert!(all.len() >= 6);
        let web = super::find("web").unwrap();
        assert_eq!(web.resize.unwrap().width, Some(1920));
    }
}
