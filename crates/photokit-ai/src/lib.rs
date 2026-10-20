//! The AI job contract shared by every backend, plus the bundled adapters.
//!
//! The UI and CLI never assume an operation is available: they call
//! [`Backend::discover_capabilities`] first and only offer what it reports.

mod comfyui;
mod http;
pub mod ollama;
mod worker;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use photokit_core::config::Config;
use serde::{Deserialize, Serialize};

pub use comfyui::ComfyUi;
pub use worker::LocalWorker;

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("{backend} is not reachable at {url}: {reason}")]
    Unavailable { backend: String, url: String, reason: String },
    #[error("{0} is not supported by this backend")]
    Unsupported(Operation),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    TextToImage,
    ImageToImage,
    Inpaint,
    Outpaint,
    Upscale,
    RemoveBackground,
}

impl Operation {
    pub const ALL: [Operation; 6] =
        [Self::TextToImage, Self::ImageToImage, Self::Inpaint, Self::Outpaint, Self::Upscale, Self::RemoveBackground];

    pub fn key(self) -> &'static str {
        match self {
            Self::TextToImage => "text_to_image",
            Self::ImageToImage => "image_to_image",
            Self::Inpaint => "inpaint",
            Self::Outpaint => "outpaint",
            Self::Upscale => "upscale",
            Self::RemoveBackground => "remove_background",
        }
    }

    pub fn needs_source(self) -> bool {
        self != Self::TextToImage
    }

    pub fn needs_prompt(self) -> bool {
        matches!(self, Self::TextToImage | Self::ImageToImage | Self::Inpaint | Self::Outpaint)
    }
}

impl std::fmt::Display for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.key())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct JobRequest {
    pub operation: Operation,
    pub prompt: String,
    pub negative_prompt: String,
    pub model: Option<String>,
    pub width: u32,
    pub height: u32,
    pub steps: u32,
    pub cfg: f32,
    pub seed: Option<u64>,
    /// Image-to-image strength: 0 keeps the source, 1 ignores it.
    pub denoise: f32,
    pub batch: u32,
    /// Outpaint padding: left, top, right, bottom.
    pub pad: [u32; 4],
    /// Upscale factor applied to the source dimensions.
    pub scale: f32,
    /// Encoded source image (any supported format).
    #[serde(skip)]
    pub source: Option<Vec<u8>>,
    /// Encoded mask; white marks the area to change.
    #[serde(skip)]
    pub mask: Option<Vec<u8>>,
}

impl Default for JobRequest {
    fn default() -> Self {
        Self {
            operation: Operation::TextToImage,
            prompt: String::new(),
            negative_prompt: "blurry, low quality, watermark, text".into(),
            model: None,
            width: 1024,
            height: 1024,
            steps: 24,
            cfg: 7.0,
            seed: None,
            denoise: 0.6,
            batch: 1,
            pad: [0, 0, 0, 0],
            scale: 2.0,
            source: None,
            mask: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    /// `checkpoint`, `upscale`, `llm`, …
    pub kind: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub operations: Vec<Operation>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub ok: bool,
    pub url: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progress {
    pub state: JobState,
    /// 0..1 when the backend reports it.
    pub fraction: Option<f32>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GeneratedImage {
    pub data: Vec<u8>,
    pub seed: Option<u64>,
}

/// The contract every AI adapter implements.
pub trait Backend: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn health(&self) -> Health;
    fn discover_capabilities(&self) -> Result<Capabilities>;
    fn list_models(&self) -> Result<Vec<ModelInfo>>;

    fn validate_job(&self, req: &JobRequest) -> Result<()> {
        validate_common(req)?;
        if !self.discover_capabilities()?.operations.contains(&req.operation) {
            return Err(AiError::Unsupported(req.operation).into());
        }
        Ok(())
    }

    /// Start a job; returns the backend's job id.
    fn submit_job(&self, req: &JobRequest) -> Result<String>;
    fn get_progress(&self, job: &str) -> Result<Progress>;
    fn cancel_job(&self, job: &str) -> Result<()>;
    fn collect_results(&self, job: &str) -> Result<Vec<GeneratedImage>>;
}

pub fn validate_common(req: &JobRequest) -> Result<()> {
    if req.operation.needs_prompt() && req.prompt.trim().is_empty() {
        bail!("a prompt is required for {}", req.operation);
    }
    if req.operation.needs_source() && req.source.is_none() {
        bail!("{} needs a source image", req.operation);
    }
    if req.operation == Operation::Inpaint && req.mask.is_none() {
        bail!("inpainting needs a mask image");
    }
    if !(64..=4096).contains(&req.width) || !(64..=4096).contains(&req.height) {
        bail!("width and height must be between 64 and 4096");
    }
    if !(1..=200).contains(&req.steps) {
        bail!("steps must be between 1 and 200");
    }
    if req.operation == Operation::Outpaint && req.pad.iter().all(|p| *p == 0) {
        bail!("outpainting needs padding on at least one side");
    }
    Ok(())
}

/// Submit and wait, reporting progress, until the job finishes or `cancel` is set.
pub fn run_job(
    backend: &dyn Backend,
    req: &JobRequest,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(&Progress),
) -> Result<Vec<GeneratedImage>> {
    let mut req = req.clone();
    let seed = *req.seed.get_or_insert_with(random_seed);
    backend.validate_job(&req)?;
    let id = backend.submit_job(&req)?;
    let started = Instant::now();
    let mut last_state = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = backend.cancel_job(&id);
            bail!("cancelled");
        }
        let p = backend.get_progress(&id)?;
        if last_state != Some(p.state) || p.fraction.is_some() {
            on_progress(&p);
            last_state = Some(p.state);
        }
        match p.state {
            JobState::Completed => {
                let mut images = backend.collect_results(&id)?;
                for (i, img) in images.iter_mut().enumerate() {
                    img.seed.get_or_insert(seed + i as u64);
                    finalize(&req, img)?;
                }
                return Ok(images);
            }
            JobState::Failed => bail!("{} job failed: {}", backend.name(), p.message.unwrap_or_default()),
            JobState::Cancelled => bail!("job was cancelled by the backend"),
            _ => {}
        }
        if started.elapsed() > Duration::from_secs(60 * 30) {
            let _ = backend.cancel_job(&id);
            bail!("job timed out after 30 minutes");
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// Backend-independent post-processing so every client gets the same result:
/// upscales match the requested factor, and inpainting is blended back into
/// the full-resolution source so untouched areas keep their original pixels.
fn finalize(req: &JobRequest, img: &mut GeneratedImage) -> Result<()> {
    use photokit_core::codecs::{decode_bytes, encode, EncodeOptions, OutputFormat};
    use photokit_core::ops::resize_exact;
    let Some(src) = req.source.as_deref() else { return Ok(()) };
    let png = EncodeOptions { format: OutputFormat::Png, png_level: 0, ..Default::default() };
    match req.operation {
        Operation::Upscale => {
            let source = decode_bytes(src)?.image;
            let out = decode_bytes(&img.data)?.image;
            let (tw, th) = (
                (source.width() as f32 * req.scale).round() as u32,
                (source.height() as f32 * req.scale).round() as u32,
            );
            if (out.width(), out.height()) != (tw, th) && tw > 0 && th > 0 {
                img.data = encode(&resize_exact(&out, tw, th)?, &png, None)?;
            }
        }
        Operation::Inpaint => {
            let Some(mask) = req.mask.as_deref() else { return Ok(()) };
            let source = decode_bytes(src)?.image;
            let opaque = !source.color().has_alpha();
            let source = source.to_rgba8();
            let out = decode_bytes(&img.data)?.image;
            let (w, h) = source.dimensions();
            // Feather at working size (cheap), then scale the mask up smoothly.
            let mask = decode_bytes(mask)?.image;
            let small = resize_exact(&mask, out.width(), out.height())?.to_luma8();
            let soft = image::DynamicImage::ImageLuma8(image::imageops::blur(&small, 3.0));
            let mask = resize_exact(&soft, w, h)?.to_luma8();
            let out = resize_exact(&out, w, h)?.to_rgba8();
            let blended = image::RgbaImage::from_fn(w, h, |x, y| {
                let a = mask.get_pixel(x, y).0[0] as u32;
                let (s, o) = (source.get_pixel(x, y).0, out.get_pixel(x, y).0);
                image::Rgba(std::array::from_fn(|i| ((o[i] as u32 * a + s[i] as u32 * (255 - a) + 127) / 255) as u8))
            });
            let blended = image::DynamicImage::ImageRgba8(blended);
            let blended = if opaque { image::DynamicImage::ImageRgb8(blended.to_rgb8()) } else { blended };
            img.data = encode(&blended, &png, None)?;
        }
        _ => {}
    }
    Ok(())
}

/// All configured backends, in preference order (default backend first).
pub fn backends(cfg: &Config) -> Vec<Box<dyn Backend>> {
    let mut list: Vec<Box<dyn Backend>> =
        vec![Box::new(ComfyUi::new(&cfg.comfyui)), Box::new(LocalWorker::new(&cfg.worker.url))];
    list.sort_by_key(|b| b.id() != cfg.ai.default_backend);
    list
}

pub fn backend(cfg: &Config, id: Option<&str>) -> Result<Box<dyn Backend>> {
    let id = id.unwrap_or(&cfg.ai.default_backend);
    backends(cfg)
        .into_iter()
        .find(|b| b.id() == id)
        .ok_or_else(|| anyhow::anyhow!("unknown backend '{id}' (available: comfyui, worker)"))
}

/// Prepare a source image for diffusion: fit inside `max_side`, round to a
/// multiple of 8 and re-encode as PNG. Returns the PNG and its dimensions.
pub fn prepare_image(data: &[u8], max_side: u32) -> Result<(Vec<u8>, u32, u32)> {
    let img = photokit_core::codecs::decode_bytes(data)?.image;
    let (w, h) = (img.width() as f64, img.height() as f64);
    let s = (max_side as f64 / w.max(h)).min(1.0);
    let tw = (((w * s) as u32) / 8 * 8).max(8);
    let th = (((h * s) as u32) / 8 * 8).max(8);
    let img =
        if (tw, th) == (img.width(), img.height()) { img } else { photokit_core::ops::resize_exact(&img, tw, th)? };
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok((out, tw, th))
}

/// Resize `mask` to exactly `w`×`h` and encode as PNG.
pub fn prepare_mask(data: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let img = photokit_core::codecs::decode_bytes(data)?.image;
    let img = if (img.width(), img.height()) == (w, h) { img } else { photokit_core::ops::resize_exact(&img, w, h)? };
    let mut out = Vec::new();
    image::DynamicImage::ImageLuma8(img.to_luma8())
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)?;
    Ok(out)
}

/// 128 bits from the OS-seeded hasher keys; suitable for session tokens.
pub fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(random_seed());
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

pub fn random_seed() -> u64 {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    // splitmix64 so consecutive calls differ a lot
    let mut z = nanos.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) % 1_000_000_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A mock backend proving `run_job` drives the contract correctly.
    struct Mock {
        polls: Mutex<u32>,
    }

    impl Backend for Mock {
        fn id(&self) -> &'static str {
            "mock"
        }
        fn name(&self) -> &'static str {
            "Mock"
        }
        fn health(&self) -> Health {
            Health { ok: true, url: String::new(), message: "ok".into() }
        }
        fn discover_capabilities(&self) -> Result<Capabilities> {
            Ok(Capabilities { operations: vec![Operation::TextToImage], notes: vec![] })
        }
        fn list_models(&self) -> Result<Vec<ModelInfo>> {
            Ok(vec![])
        }
        fn submit_job(&self, _: &JobRequest) -> Result<String> {
            Ok("job1".into())
        }
        fn get_progress(&self, _: &str) -> Result<Progress> {
            let mut p = self.polls.lock().unwrap();
            *p += 1;
            let state = if *p >= 2 { JobState::Completed } else { JobState::Running };
            Ok(Progress { state, fraction: None, message: None })
        }
        fn cancel_job(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn collect_results(&self, _: &str) -> Result<Vec<GeneratedImage>> {
            Ok(vec![GeneratedImage { data: vec![1, 2, 3], seed: Some(1) }])
        }
    }

    #[test]
    fn run_job_completes_and_rejects_unsupported() {
        let m = Mock { polls: Mutex::new(0) };
        let req = JobRequest { prompt: "a cat".into(), ..Default::default() };
        let out = run_job(&m, &req, &AtomicBool::new(false), |_| {}).unwrap();
        assert_eq!(out.len(), 1);

        let bad = JobRequest { operation: Operation::Upscale, source: Some(vec![0]), ..Default::default() };
        let err = run_job(&m, &bad, &AtomicBool::new(false), |_| {}).unwrap_err();
        assert!(err.to_string().contains("not supported"));
    }

    #[test]
    fn missing_prompt_is_rejected() {
        assert!(validate_common(&JobRequest::default()).is_err());
    }
}
