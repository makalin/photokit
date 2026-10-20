//! User configuration stored at `<config dir>/photokit/config.toml`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ComfyConfig {
    pub url: String,
    /// Extra workflow templates; files here override the built-in ones by name.
    pub workflow_dir: Option<PathBuf>,
    /// Default checkpoint name as ComfyUI lists it.
    pub checkpoint: Option<String>,
    pub upscale_model: Option<String>,
}

impl Default for ComfyConfig {
    fn default() -> Self {
        Self { url: "http://127.0.0.1:8188".into(), workflow_dir: None, checkpoint: None, upscale_model: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OllamaConfig {
    pub url: String,
    pub model: String,
}

impl Default for OllamaConfig {
    fn default() -> Self {
        Self { url: "http://127.0.0.1:11434".into(), model: "llama3.2".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkerConfig {
    pub url: String,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self { url: "http://127.0.0.1:7879".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// `comfyui` or `worker`.
    pub default_backend: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self { default_backend: "comfyui".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self { port: 7878 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    pub format: String,
    pub quality: u8,
}

impl Default for Defaults {
    fn default() -> Self {
        Self { format: "webp".into(), quality: 85 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub comfyui: ComfyConfig,
    pub ollama: OllamaConfig,
    pub worker: WorkerConfig,
    pub ai: AiConfig,
    pub server: ServerConfig,
    pub defaults: Defaults,
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PHOTOKIT_CONFIG_DIR") {
        return Some(PathBuf::from(p));
    }
    dirs::config_dir().map(|d| d.join("photokit"))
}

/// Where generated images and other app data are stored.
pub fn data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("PHOTOKIT_DATA_DIR") {
        return PathBuf::from(p);
    }
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("photokit")
}

pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

impl Config {
    /// Load the config, falling back to defaults when the file is missing.
    pub fn load() -> Result<Self> {
        let Some(path) = config_path().filter(|p| p.exists()) else {
            return Ok(Self::default());
        };
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path().context("no config directory on this platform")?;
        crate::naming::write_atomic(&path, toml::to_string_pretty(self)?.as_bytes())?;
        Ok(path)
    }

    /// Set a dotted key such as `comfyui.url`.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let mut doc = toml::Value::try_from(&*self)?;
        let mut cur = &mut doc;
        let parts: Vec<&str> = key.split('.').collect();
        for part in &parts[..parts.len() - 1] {
            cur = cur.get_mut(*part).with_context(|| format!("unknown config section '{part}'"))?;
        }
        let last = parts[parts.len() - 1];
        let table = cur.as_table_mut().context("not a config section")?;
        let parsed = match table.get(last) {
            Some(toml::Value::Integer(_)) => toml::Value::Integer(value.parse().context("expected a number")?),
            Some(toml::Value::Boolean(_)) => toml::Value::Boolean(value.parse().context("expected true/false")?),
            _ => toml::Value::String(value.to_string()),
        };
        table.insert(last.to_string(), parsed);
        *self = doc.try_into().context("invalid value")?;
        Ok(())
    }
}
