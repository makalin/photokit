//! Generated images saved under `<data dir>/gallery` with JSON sidecars.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryItem {
    pub id: String,
    pub created: u64,
    pub backend: String,
    pub operation: String,
    pub prompt: String,
    pub negative_prompt: String,
    pub model: Option<String>,
    pub seed: Option<u64>,
    pub width: u32,
    pub height: u32,
    pub file: PathBuf,
}

pub fn dir() -> PathBuf {
    photokit_core::config::data_dir().join("gallery")
}

pub fn save(mut item: GalleryItem, data: &[u8]) -> Result<GalleryItem> {
    let dir = dir();
    std::fs::create_dir_all(&dir)?;
    let ext = image::guess_format(data).ok().and_then(|f| f.extensions_str().first().copied()).unwrap_or("png");
    item.file = dir.join(format!("{}.{ext}", item.id));
    photokit_core::naming::write_atomic(&item.file, data)?;
    photokit_core::naming::write_atomic(&dir.join(format!("{}.json", item.id)), &serde_json::to_vec_pretty(&item)?)?;
    Ok(item)
}

pub fn list() -> Vec<GalleryItem> {
    let mut items: Vec<GalleryItem> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .collect();
    items.sort_by(|a, b| b.created.cmp(&a.created));
    items
}

pub fn get(id: &str) -> Result<GalleryItem> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        anyhow::bail!("invalid gallery id");
    }
    let data = std::fs::read(dir().join(format!("{id}.json"))).context("gallery item not found")?;
    Ok(serde_json::from_slice(&data)?)
}

pub fn delete(id: &str) -> Result<()> {
    let item = get(id)?;
    let _ = std::fs::remove_file(&item.file);
    std::fs::remove_file(dir().join(format!("{id}.json")))?;
    Ok(())
}
