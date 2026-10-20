//! Parallel processing of many files: resize/convert/compress plus output naming.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use photokit_codecs::{self as codecs, Decoded, EncodeOptions, Encoded, OutputFormat};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::naming::{self, NameContext};
use crate::ops::{apply_ops, Op};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchSpec {
    pub ops: Vec<Op>,
    /// `None` keeps each file's source format.
    pub format: Option<OutputFormat>,
    pub encode: EncodeOptions,
    pub target_bytes: Option<u64>,
    /// `None` writes next to each source file.
    pub output_dir: Option<PathBuf>,
    /// Output name template without extension (see `naming::render`).
    pub name_pattern: String,
    pub start: u64,
    pub overwrite: bool,
    pub dry_run: bool,
}

impl Default for BatchSpec {
    fn default() -> Self {
        Self {
            ops: Vec::new(),
            format: None,
            encode: EncodeOptions::default(),
            target_bytes: None,
            output_dir: None,
            name_pattern: "{name}".into(),
            start: 1,
            overwrite: false,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ItemResult {
    pub input: PathBuf,
    pub output: PathBuf,
    pub ok: bool,
    pub skipped: bool,
    pub error: Option<String>,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchReport {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub cancelled: bool,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub elapsed_ms: u128,
    pub items: Vec<ItemResult>,
}

/// Output paths for every input, decided up front so counters are stable and
/// two inputs never write the same file.
pub fn plan_outputs(files: &[PathBuf], spec: &BatchSpec) -> Vec<PathBuf> {
    let dims = naming::needs_dimensions(&spec.name_pattern);
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    files
        .iter()
        .enumerate()
        .map(|(i, input)| {
            let mut ctx = NameContext::for_path(input, spec.start + i as u64);
            if dims {
                // Name reflects the source size; output size is unknown before processing.
                if let Some((w, h)) = codecs::probe_dimensions(input) {
                    ctx.width = Some(w);
                    ctx.height = Some(h);
                }
            }
            let format = spec.format.or_else(|| OutputFormat::from_path(input)).unwrap_or(OutputFormat::Png);
            let stem = naming::render(&spec.name_pattern, &ctx);
            let dir = spec.output_dir.clone().unwrap_or_else(|| input.parent().unwrap_or(Path::new(".")).to_path_buf());
            let wanted = dir.join(format!("{stem}.{}", format.extension()));
            let path = if spec.overwrite {
                naming::unique_path_with(&wanted, |p| claimed.contains(p))
            } else {
                naming::unique_path_with(&wanted, |p| claimed.contains(p) || p.exists())
            };
            claimed.insert(path.clone());
            path
        })
        .collect()
}

/// Decode, apply ops and encode one image in memory.
pub fn process(
    decoded: &Decoded,
    ops: &[Op],
    encode: &EncodeOptions,
    target: Option<u64>,
) -> Result<(Encoded, u32, u32)> {
    let img = apply_ops(decoded.image.clone(), ops)?;
    let (w, h) = (img.width(), img.height());
    let enc = codecs::encode_to_target(&img, encode, decoded.icc_profile.as_deref(), target)?;
    Ok((enc, w, h))
}

pub fn process_file(input: &Path, output: &Path, spec: &BatchSpec) -> Result<ItemResult> {
    let decoded = codecs::decode_file(input)?;
    let format = spec.format.or(decoded.format).unwrap_or(OutputFormat::Png);
    let encode = EncodeOptions { format, ..spec.encode.clone() };
    let (enc, width, height) = process(&decoded, &spec.ops, &encode, spec.target_bytes)?;
    if !spec.dry_run {
        naming::write_atomic(output, &enc.data).with_context(|| format!("cannot write {}", output.display()))?;
    }
    Ok(ItemResult {
        input: input.to_path_buf(),
        output: output.to_path_buf(),
        ok: true,
        skipped: false,
        error: None,
        input_bytes: decoded.source_bytes,
        output_bytes: enc.data.len() as u64,
        width,
        height,
    })
}

/// Process files in parallel. `on_item` is called from worker threads as each
/// file finishes; setting `cancel` stops scheduling new files.
pub fn run(
    files: &[PathBuf],
    spec: &BatchSpec,
    cancel: &AtomicBool,
    on_item: impl Fn(&ItemResult) + Sync,
) -> BatchReport {
    let start = Instant::now();
    let outputs = plan_outputs(files, spec);
    let mut items: Vec<ItemResult> = files
        .par_iter()
        .zip(outputs.par_iter())
        .map(|(input, output)| {
            let item = if cancel.load(Ordering::Relaxed) {
                ItemResult { input: input.clone(), output: output.clone(), skipped: true, ..Default::default() }
            } else {
                process_file(input, output, spec).unwrap_or_else(|e| ItemResult {
                    input: input.clone(),
                    output: output.clone(),
                    error: Some(format!("{e:#}")),
                    ..Default::default()
                })
            };
            on_item(&item);
            item
        })
        .collect();
    items.sort_by(|a, b| a.input.cmp(&b.input));
    BatchReport {
        total: files.len(),
        succeeded: items.iter().filter(|i| i.ok).count(),
        failed: items.iter().filter(|i| i.error.is_some()).count(),
        cancelled: cancel.load(Ordering::Relaxed),
        input_bytes: items.iter().filter(|i| i.ok).map(|i| i.input_bytes).sum(),
        output_bytes: items.iter().map(|i| i.output_bytes).sum(),
        elapsed_ms: start.elapsed().as_millis(),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::ResizeSpec;

    #[test]
    fn batch_resizes_converts_and_names() {
        let d = std::env::temp_dir().join(format!("pk-batch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..4 {
            image::RgbImage::from_pixel(300 + i * 10, 200, image::Rgb([i as u8 * 40, 80, 160]))
                .save(d.join(format!("img{i}.png")))
                .unwrap();
        }
        std::fs::write(d.join("broken.jpg"), b"not an image").unwrap();
        let files = crate::rename::collect_images(&d, false).unwrap();
        let spec = BatchSpec {
            ops: vec![Op::Resize(ResizeSpec { width: Some(100), ..Default::default() })],
            format: Some(OutputFormat::Webp),
            output_dir: Some(d.join("out")),
            name_pattern: "photo-{n:2}".into(),
            ..Default::default()
        };
        let report = run(&files, &spec, &AtomicBool::new(false), |_| {});
        assert_eq!(report.total, 5);
        assert_eq!(report.succeeded, 4);
        assert_eq!(report.failed, 1);
        let out = image::open(d.join("out/photo-02.webp")).unwrap();
        assert_eq!(out.width(), 100);
        std::fs::remove_dir_all(d).unwrap();
    }
}
