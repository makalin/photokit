use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use photokit_ai::JobRequest;
use photokit_core::batch::{self, BatchSpec};
use photokit_core::codecs::{self, EncodeOptions};
use photokit_core::config::Config;
use photokit_core::rename::{self, RenameOptions};
use photokit_core::{naming, ops, preset, Op};
use serde::Deserialize;
use serde_json::{json, Value};
use tiny_http::{Method, Request};

use crate::gallery::{self, GalleryItem};
use crate::util::{self, bytes, error, header, json, Resp};
use crate::{assets, State};

pub fn handle(state: &Arc<State>, mut req: Request) {
    let (path, query) = util::split_url(req.url());
    let resp = if path.starts_with("/api/") {
        let authorized = util::header_value(&req, "X-PhotoKit-Token").as_deref() == Some(&state.token)
            || query.get("t") == Some(&state.token);
        if !authorized {
            error(403, "missing or invalid PhotoKit token; reload the page")
        } else {
            route(state, &mut req, &path, &query).unwrap_or_else(|e| error(400, format!("{e:#}")))
        }
    } else if req.method() == &Method::Get {
        match assets::get(&path) {
            Some((data, mime)) if mime.starts_with("text/html") => {
                let html = String::from_utf8_lossy(&data).replace("__PHOTOKIT_TOKEN__", &state.token);
                bytes(html.into_bytes(), mime)
            }
            Some((data, mime)) => bytes(data, mime),
            None => error(404, "not found"),
        }
    } else {
        error(405, "method not allowed")
    };
    let _ = req.respond(resp);
}

type Query = std::collections::HashMap<String, String>;

fn route(state: &Arc<State>, req: &mut Request, path: &str, q: &Query) -> Result<Resp> {
    let method = req.method().clone();
    let parts: Vec<&str> = path.trim_start_matches("/api/").split('/').collect();
    Ok(match (method, parts.as_slice()) {
        (Method::Get, ["info"]) => json(&json!({
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "home": dirs::home_dir(),
            "pictures": dirs::picture_dir(),
            "config_path": photokit_core::config::config_path(),
            "data_dir": photokit_core::config::data_dir(),
            "formats": codecs::OutputFormat::ALL.iter().map(|f| f.to_string()).collect::<Vec<_>>(),
        })),

        // ---------- file system ----------
        (Method::Get, ["fs", "list"]) => fs_list(q)?,
        (Method::Get, ["fs", "thumb"]) => {
            let p = PathBuf::from(q.get("path").context("path is required")?);
            let size = q.get("size").and_then(|s| s.parse().ok()).unwrap_or(256u32).clamp(32, 1024);
            bytes(state.thumbs.get_or_make(&p, size)?, "image/jpeg").with_header(header("Cache-Control", "max-age=60"))
        }

        // ---------- sources ----------
        (Method::Post, ["source", "open"]) => {
            #[derive(Deserialize)]
            struct B {
                path: PathBuf,
            }
            let b: B = util::read_json(req)?;
            json(&state.sources.open_path(b.path)?.info())
        }
        (Method::Post, ["source", "upload"]) => {
            let name = util::header_value(req, "X-Filename")
                .map(|n| util::percent_decode(&n))
                .unwrap_or_else(|| "pasted.png".into());
            let body = util::read_body(req, 512 * 1024 * 1024)?;
            json(&state.sources.add_bytes(&naming::sanitize(&name), None, body)?.info())
        }
        (Method::Get, ["source", id]) => {
            let src = state.sources.get(id)?;
            let max = q.get("max").and_then(|s| s.parse().ok()).unwrap_or(2048u32);
            let (data, mime) = util::encode_preview(&ops::thumbnail(&src.decoded.image, max), 88)?;
            bytes(data, mime)
        }

        // ---------- processing ----------
        (Method::Post, ["render"]) => {
            // Edited image (without final encode) for the editor canvas.
            let b: ProcessBody = util::read_json(req)?;
            let src = state.sources.get(&b.source)?;
            let img = ops::apply_ops(
                ops::thumbnail(&src.decoded.image, b.max.unwrap_or(1600)),
                &scale_ops(&b.ops, &src, b.max.unwrap_or(1600)),
            )?;
            let (data, mime) = util::encode_preview(&img, 88)?;
            bytes(data, mime)
        }
        (Method::Post, ["process"]) => process(state, req)?,
        (Method::Get, ["presets"]) => json(&preset::all()),

        // ---------- rename ----------
        (Method::Post, ["rename", action @ ("preview" | "apply")]) => {
            #[derive(Deserialize)]
            struct B {
                #[serde(flatten)]
                files: FileSet,
                #[serde(default)]
                options: RenameOptions,
            }
            let b: B = util::read_json(req)?;
            let files = b.files.resolve()?;
            let plan = rename::plan(&files, &b.options)?;
            let renamed = if *action == "apply" { rename::apply(&plan)? } else { 0 };
            json(&json!({ "items": plan, "renamed": renamed }))
        }

        // ---------- batch ----------
        (Method::Post, ["batch", action @ ("plan" | "run")]) => {
            #[derive(Deserialize)]
            struct B {
                #[serde(flatten)]
                files: FileSet,
                spec: BatchSpec,
            }
            let b: B = util::read_json(req)?;
            let files = b.files.resolve()?;
            if *action == "plan" {
                let outs = batch::plan_outputs(&files, &b.spec);
                let items: Vec<Value> = files.iter().zip(outs).map(|(i, o)| json!({ "from": i, "to": o })).collect();
                json(&json!({ "items": items }))
            } else {
                json(&json!({ "job": start_batch(state, files, b.spec) }))
            }
        }
        (Method::Get, ["jobs", id]) => {
            let job = state.jobs.get(id).context("unknown job")?;
            let info = job.info.lock().unwrap().clone();
            json(&info)
        }
        (Method::Post, ["jobs", id, "cancel"]) => {
            let job = state.jobs.get(id).context("unknown job")?;
            job.cancel.store(true, Ordering::Relaxed);
            json(&json!({ "ok": true }))
        }

        // ---------- AI ----------
        (Method::Get, ["ai", "status"]) => ai_status(state),
        (Method::Post, ["ai", "generate"]) => ai_generate(state, req)?,
        (Method::Post, ["ai", "enhance"]) => {
            #[derive(Deserialize)]
            struct B {
                prompt: String,
            }
            let b: B = util::read_json(req)?;
            let cfg = state.config.read().unwrap().ollama.clone();
            json(&json!({ "prompt": photokit_ai::ollama::Ollama::new(&cfg).enhance_prompt(&b.prompt)? }))
        }
        (Method::Post, ["ai", "assist"]) => {
            #[derive(Deserialize)]
            struct B {
                text: String,
            }
            let b: B = util::read_json(req)?;
            let cfg = state.config.read().unwrap().ollama.clone();
            json(&photokit_ai::ollama::Ollama::new(&cfg).assist(&b.text)?)
        }
        (Method::Get, ["gallery"]) => json(&gallery::list()),
        (Method::Get, ["gallery", id, "image"]) => {
            let item = gallery::get(id)?;
            let data = std::fs::read(&item.file)?;
            let mime = image::guess_format(&data).map(|f| f.to_mime_type()).unwrap_or("image/png");
            bytes(data, mime)
        }
        (Method::Post, ["gallery", id, "open"]) => {
            let item = gallery::get(id)?;
            json(&state.sources.open_path(item.file)?.info())
        }
        (Method::Delete, ["gallery", id]) => {
            gallery::delete(id)?;
            json(&json!({ "ok": true }))
        }

        // ---------- config ----------
        (Method::Get, ["config"]) => json(&*state.config.read().unwrap()),
        (Method::Post, ["config"]) => {
            let cfg: Config = util::read_json(req)?;
            let path = cfg.save()?;
            *state.config.write().unwrap() = cfg;
            json(&json!({ "ok": true, "path": path }))
        }

        _ => error(404, format!("no route for {path}")),
    })
}

/// A folder (optionally recursive) or an explicit file list.
#[derive(Deserialize)]
struct FileSet {
    dir: Option<PathBuf>,
    #[serde(default)]
    files: Vec<PathBuf>,
    #[serde(default)]
    recursive: bool,
}

impl FileSet {
    fn resolve(self) -> Result<Vec<PathBuf>> {
        if !self.files.is_empty() {
            return Ok(self.files);
        }
        let dir = self.dir.context("choose a folder or files")?;
        let files = rename::collect_images(&dir, self.recursive)?;
        if files.is_empty() {
            bail!("no supported images in {}", dir.display());
        }
        Ok(files)
    }
}

fn fs_list(q: &Query) -> Result<Resp> {
    let dir = match q.get("path").filter(|p| !p.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => dirs::picture_dir().or_else(dirs::home_dir).context("no home directory")?,
    };
    let mut dirs_out = Vec::new();
    let mut files = Vec::new();
    for e in std::fs::read_dir(&dir).with_context(|| format!("cannot open {}", dir.display()))?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            dirs_out.push(name);
        } else if codecs::is_supported_input(&e.path()) {
            let modified =
                meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
            files.push(json!({ "name": name, "path": e.path(), "size": meta.len(), "modified": modified }));
        }
    }
    dirs_out.sort_by_key(|n| n.to_lowercase());
    files.sort_by_key(|f| f["name"].as_str().unwrap_or("").to_lowercase());
    Ok(json(&json!({
        "path": dir,
        "parent": dir.parent(),
        "sep": std::path::MAIN_SEPARATOR.to_string(),
        "dirs": dirs_out,
        "files": files,
        "roots": util::roots(),
    })))
}

#[derive(Deserialize)]
struct ProcessBody {
    source: String,
    #[serde(default)]
    ops: Vec<Op>,
    #[serde(default)]
    encode: EncodeOptions,
    target_kb: Option<u64>,
    /// preview | download | save
    #[serde(default)]
    action: String,
    /// For `save`: folder to write to (default: next to the source).
    output_dir: Option<PathBuf>,
    name: Option<String>,
    #[serde(default)]
    overwrite: bool,
    max: Option<u32>,
}

/// Crop coordinates arrive in full-resolution pixels; scale them for a preview.
fn scale_ops(list: &[Op], src: &crate::sources::Source, max: u32) -> Vec<Op> {
    let (w, h) = (src.decoded.image.width(), src.decoded.image.height());
    let s = (max as f64 / w.max(h) as f64).min(1.0);
    list.iter()
        .filter(|o| !matches!(o, Op::Resize(_)))
        .map(|o| match o {
            Op::Crop { x, y, width, height } if s < 1.0 => Op::Crop {
                x: (*x as f64 * s) as u32,
                y: (*y as f64 * s) as u32,
                width: ((*width as f64 * s) as u32).max(1),
                height: ((*height as f64 * s) as u32).max(1),
            },
            o => o.clone(),
        })
        .collect()
}

fn process(state: &Arc<State>, req: &mut Request) -> Result<Resp> {
    let b: ProcessBody = util::read_json(req)?;
    let src = state.sources.get(&b.source)?;
    let (enc, w, h) = batch::process(&src.decoded, &b.ops, &b.encode, b.target_kb.map(|k| k * 1024))?;
    let meta = |r: Resp| {
        r.with_header(header("X-Width", &w.to_string()))
            .with_header(header("X-Height", &h.to_string()))
            .with_header(header("X-Bytes", &enc.data.len().to_string()))
            .with_header(header("X-Quality", &enc.quality.to_string()))
            .with_header(header("Access-Control-Expose-Headers", "X-Width, X-Height, X-Bytes, X-Quality"))
    };
    let stem = b
        .name
        .as_deref()
        .map(naming::sanitize)
        .unwrap_or_else(|| format!("{}-edited", src.name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&src.name)));
    let filename = format!("{stem}.{}", enc.format.extension());
    Ok(match b.action.as_str() {
        "save" => {
            let dir = b
                .output_dir
                .or_else(|| src.path.as_ref().and_then(|p| p.parent().map(PathBuf::from)))
                .unwrap_or_else(|| photokit_core::config::data_dir().join("exports"));
            let target = dir.join(&filename);
            let target = if b.overwrite { target } else { naming::unique_path(&target) };
            naming::write_atomic(&target, &enc.data)?;
            json(&json!({ "path": target, "bytes": enc.data.len(), "width": w, "height": h }))
        }
        "download" => {
            let mime = enc.format.mime();
            let data = enc.data.clone();
            meta(bytes(data, mime))
                .with_header(header("Content-Disposition", &format!("attachment; filename=\"{filename}\"")))
        }
        _ => {
            let mime = enc.format.mime();
            // Not every webview decodes AVIF; show a lossless rendering, keep the real size headers.
            if enc.format == codecs::OutputFormat::Avif {
                let img = ops::apply_ops(src.decoded.image.clone(), &b.ops)?;
                let (data, mime) = util::encode_preview(&img, 92)?;
                return Ok(meta(bytes(data, mime)));
            }
            meta(bytes(enc.data.clone(), mime))
        }
    })
}

fn start_batch(state: &Arc<State>, files: Vec<PathBuf>, spec: BatchSpec) -> String {
    let job = state.jobs.create("batch", files.len());
    let id = job.info.lock().unwrap().id.clone();
    std::thread::spawn(move || {
        let report = batch::run(&files, &spec, &job.cancel, |item| {
            job.update(|i| {
                i.done += 1;
                i.fraction = Some(i.done as f32 / i.total.max(1) as f32);
                i.recent.push(serde_json::to_value(item).unwrap_or_default());
                if i.recent.len() > 500 {
                    i.recent.drain(..100);
                }
            });
        });
        job.finish(serde_json::to_value(report).map_err(Into::into));
    });
    id
}

fn ai_status(state: &Arc<State>) -> Resp {
    let cfg = state.config.read().unwrap().clone();
    let handles: Vec<_> = photokit_ai::backends(&cfg)
        .into_iter()
        .map(|b| {
            std::thread::spawn(move || {
                let health = b.health();
                let (caps, models) = if health.ok {
                    (b.discover_capabilities().ok(), b.list_models().unwrap_or_default())
                } else {
                    (None, Vec::new())
                };
                json!({ "id": b.id(), "name": b.name(), "health": health, "capabilities": caps, "models": models })
            })
        })
        .collect();
    let ollama_cfg = cfg.ollama.clone();
    let ollama = std::thread::spawn(move || photokit_ai::ollama::Ollama::new(&ollama_cfg).health());
    let backends: Vec<Value> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
    json(&json!({ "backends": backends, "ollama": ollama.join().ok(), "default": cfg.ai.default_backend }))
}

fn ai_generate(state: &Arc<State>, req: &mut Request) -> Result<Resp> {
    #[derive(Deserialize)]
    struct B {
        backend: Option<String>,
        request: JobRequest,
        source: Option<String>,
        /// Base64 PNG, white = area to change.
        mask: Option<String>,
    }
    let b: B = util::read_json(req)?;
    let cfg = state.config.read().unwrap().clone();
    let backend = photokit_ai::backend(&cfg, b.backend.as_deref())?;
    let mut job_req = b.request;
    if let Some(id) = &b.source {
        job_req.source = Some(state.sources.get(id)?.bytes.to_vec());
    }
    if let Some(m) = &b.mask {
        let m = m.split_once(',').map(|(_, d)| d).unwrap_or(m);
        job_req.mask = Some(B64.decode(m).context("invalid mask data")?);
    }
    photokit_ai::validate_common(&job_req)?;

    let job = state.jobs.create("ai", 1);
    let id = job.info.lock().unwrap().id.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<Value> {
            let images = photokit_ai::run_job(backend.as_ref(), &job_req, &job.cancel, |p| {
                job.update(|i| {
                    i.fraction = p.fraction;
                    i.message = Some(format!("{:?}", p.state).to_lowercase());
                });
            })?;
            let mut saved = Vec::new();
            for (n, img) in images.into_iter().enumerate() {
                let data = img.data;
                let (width, height) = image::load_from_memory(&data).map(|i| (i.width(), i.height())).unwrap_or((0, 0));
                let created = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let item = GalleryItem {
                    id: format!("{created}-{n}"),
                    created,
                    backend: backend.id().into(),
                    operation: job_req.operation.key().into(),
                    prompt: job_req.prompt.clone(),
                    negative_prompt: job_req.negative_prompt.clone(),
                    model: job_req.model.clone(),
                    seed: img.seed,
                    width,
                    height,
                    file: PathBuf::new(),
                };
                saved.push(gallery::save(item, &data)?);
            }
            Ok(json!({ "images": saved }))
        })();
        job.update(|i| i.done = 1);
        job.finish(result);
    });
    Ok(json(&json!({ "job": id })))
}
