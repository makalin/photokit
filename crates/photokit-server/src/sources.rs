//! Images opened in the editor, kept decoded in memory between requests.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use photokit_core::codecs::{self, Decoded};
use serde::Serialize;

pub struct Source {
    pub id: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub bytes: Arc<Vec<u8>>,
    pub decoded: Decoded,
}

#[derive(Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub format: Option<String>,
    pub bytes: u64,
    pub has_alpha: bool,
    pub has_icc: bool,
}

impl Source {
    pub fn info(&self) -> SourceInfo {
        SourceInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            path: self.path.clone(),
            width: self.decoded.image.width(),
            height: self.decoded.image.height(),
            format: self.decoded.format.map(|f| f.to_string()),
            bytes: self.decoded.source_bytes,
            has_alpha: self.decoded.image.color().has_alpha(),
            has_icc: self.decoded.icc_profile.is_some(),
        }
    }
}

/// Keeps the most recently opened images; older ones are dropped.
#[derive(Default)]
pub struct Sources {
    list: Mutex<VecDeque<Arc<Source>>>,
}

const KEEP: usize = 12;

impl Sources {
    pub fn add_bytes(&self, name: &str, path: Option<PathBuf>, bytes: Vec<u8>) -> Result<Arc<Source>> {
        let decoded = codecs::decode_bytes(&bytes).with_context(|| format!("cannot open {name}"))?;
        let src = Arc::new(Source {
            id: format!("s{:x}", photokit_ai::random_seed()),
            name: name.to_string(),
            path,
            bytes: Arc::new(bytes),
            decoded,
        });
        let mut list = self.list.lock().unwrap();
        list.push_front(src.clone());
        list.truncate(KEEP);
        Ok(src)
    }

    pub fn open_path(&self, path: PathBuf) -> Result<Arc<Source>> {
        let bytes = std::fs::read(&path).with_context(|| format!("cannot read {}", path.display()))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
        self.add_bytes(&name, Some(path), bytes)
    }

    pub fn get(&self, id: &str) -> Result<Arc<Source>> {
        self.list
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .cloned()
            .context("image is no longer open; please load it again")
    }
}
