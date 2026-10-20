//! `photokit` — fast image tools and local AI from the command line.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use photokit_ai::{JobRequest, Operation};
use photokit_core::batch::{self, BatchReport, BatchSpec};
use photokit_core::codecs::{self, EncodeOptions, OutputFormat};
use photokit_core::config::Config;
use photokit_core::rename::{self, Case, RenameOptions, SortBy};
use photokit_core::{naming, preset, Fit, Flip, Op, ResizeSpec};

/// Exit codes, documented in docs/cli.md.
mod exit {
    pub const FAILED: u8 = 1;
    pub const PARTIAL: u8 = 3;
    pub const EXISTS: u8 = 4;
    pub const BACKEND: u8 = 5;
}

#[derive(Parser)]
#[command(name = "photokit", version, about = "PhotoKit — edit fast, create freely, run locally.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Resize images by width, height or percentage
    Resize {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[command(flatten)]
        size: SizeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Crop to a rectangle, or center-crop to an aspect ratio
    Crop {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Center crop to an aspect ratio such as 16:9 or 1:1
        #[arg(long, conflicts_with_all = ["x", "y"])]
        aspect: Option<String>,
        #[arg(long, default_value_t = 0)]
        x: u32,
        #[arg(long, default_value_t = 0)]
        y: u32,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Rotate clockwise (90/180/270 are lossless; other angles expand the canvas)
    Rotate {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(long, short, allow_negative_numbers = true)]
        degrees: f32,
        /// Crop the empty corners (for small straightening angles)
        #[arg(long)]
        straighten: bool,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Mirror images
    Flip {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(long)]
        vertical: bool,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Convert between formats (optionally resizing in the same pass)
    Convert {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[command(flatten)]
        size: SizeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Re-encode in the same format with optimal settings and no metadata
    Optimize {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Show dimensions, format and size
    Info {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Process a whole folder with a preset or explicit settings
    Batch {
        /// Folder (or files) to process
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(long, short)]
        preset: Option<String>,
        #[arg(long, short)]
        recursive: bool,
        /// Output file name template, e.g. "trip-{n:3}" (see `photokit rename --help`)
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        size: SizeArgs,
        #[command(flatten)]
        out: OutArgs,
    },
    /// Rename many files in place using a pattern
    ///
    /// Tokens: {name} original name, {ext}, {parent} folder name, {n} or {n:3}
    /// counter (zero padded), {w} {h} pixel size, {date} YYYY-MM-DD,
    /// {time} HHMMSS, {datetime} YYYYMMDD-HHMMSS (file modification time).
    Rename {
        /// Folder or files
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        #[arg(long, short)]
        pattern: String,
        #[arg(long, default_value_t = 1)]
        start: u64,
        #[arg(long, default_value_t = 1)]
        step: u64,
        /// name, date or size
        #[arg(long, default_value = "name")]
        sort: String,
        /// keep, lower or upper
        #[arg(long, default_value = "keep")]
        case: String,
        /// Lowercase extensions and use .jpg for JPEG
        #[arg(long)]
        normalize_ext: bool,
        #[arg(long)]
        find: Option<String>,
        #[arg(long, default_value = "")]
        replace: String,
        #[arg(long, short)]
        recursive: bool,
        /// Only print the plan
        #[arg(long, short = 'n')]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    /// List export presets
    Presets {
        #[arg(long)]
        json: bool,
    },
    /// Run the web app on localhost
    Serve {
        #[arg(long)]
        port: Option<u16>,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Open the browser
        #[arg(long)]
        open: bool,
    },
    /// AI generation and editing through ComfyUI or the local worker
    Ai {
        #[command(subcommand)]
        cmd: AiCmd,
    },
    /// Show or change settings
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
}

#[derive(Subcommand)]
enum AiCmd {
    /// Show backends, health and capabilities
    Backends {
        #[arg(long)]
        json: bool,
    },
    /// List models available to a backend
    Models {
        #[arg(long)]
        backend: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Text to image
    Generate {
        #[command(flatten)]
        gen: GenArgs,
    },
    /// Transform an existing image with a prompt
    Img2img {
        input: PathBuf,
        /// 0 keeps the source, 1 ignores it
        #[arg(long, default_value_t = 0.6)]
        strength: f32,
        #[command(flatten)]
        gen: GenArgs,
    },
    /// Replace the white area of a mask
    Inpaint {
        input: PathBuf,
        #[arg(long)]
        mask: PathBuf,
        #[command(flatten)]
        gen: GenArgs,
    },
    /// Extend the canvas (pixels per side)
    Outpaint {
        input: PathBuf,
        #[arg(long, default_value_t = 0)]
        left: u32,
        #[arg(long, default_value_t = 0)]
        top: u32,
        #[arg(long, default_value_t = 0)]
        right: u32,
        #[arg(long, default_value_t = 0)]
        bottom: u32,
        #[command(flatten)]
        gen: GenArgs,
    },
    /// Upscale with a super-resolution model
    Upscale {
        input: PathBuf,
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
        #[command(flatten)]
        gen: GenArgs,
    },
    /// Remove the background (needs a backend that supports it)
    RemoveBg {
        input: PathBuf,
        #[command(flatten)]
        gen: GenArgs,
    },
    /// Rewrite a short idea into a detailed prompt with Ollama
    Enhance { prompt: String },
    /// Turn a plain-language request into edits and apply them (Ollama)
    Assist {
        input: PathBuf,
        request: String,
        /// Apply the plan instead of only showing it
        #[arg(long)]
        apply: bool,
        #[command(flatten)]
        out: OutArgs,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the current configuration
    Show,
    /// Print the config file location
    Path,
    /// Set a value, e.g. `photokit config set comfyui.url http://127.0.0.1:8188`
    Set { key: String, value: String },
}

#[derive(Args, Clone, Default)]
struct SizeArgs {
    #[arg(long, short = 'W')]
    width: Option<u32>,
    #[arg(long, short = 'H')]
    height: Option<u32>,
    /// Scale in percent
    #[arg(long)]
    percent: Option<f32>,
    /// contain, cover or fill
    #[arg(long)]
    fit: Option<Fit>,
    /// Allow making images larger
    #[arg(long)]
    upscale: bool,
}

impl SizeArgs {
    fn op(&self) -> Option<Op> {
        let spec = ResizeSpec {
            width: self.width,
            height: self.height,
            percent: self.percent,
            fit: self.fit.unwrap_or_default(),
            upscale: self.upscale,
        };
        (!spec.is_noop()).then_some(Op::Resize(spec))
    }
}

#[derive(Args, Clone, Default)]
struct OutArgs {
    /// Output file (single input) or folder
    #[arg(long, short)]
    output: Option<PathBuf>,
    /// jpeg, png, webp, avif, gif, bmp, tiff
    #[arg(long, short)]
    format: Option<OutputFormat>,
    /// 1-100 for JPEG/WebP/AVIF
    #[arg(long, short)]
    quality: Option<u8>,
    /// Largest allowed file size, e.g. 200KB or 1.5MB (lossy formats)
    #[arg(long)]
    target_size: Option<String>,
    /// Lossless WebP
    #[arg(long)]
    lossless: bool,
    /// Keep the ICC color profile (all other metadata is always removed)
    #[arg(long)]
    keep_metadata: bool,
    /// Remove metadata (the default; accepted for clarity)
    #[arg(long, conflicts_with = "keep_metadata")]
    strip_metadata: bool,
    /// Replace existing files
    #[arg(long)]
    overwrite: bool,
    /// Machine-readable output
    #[arg(long)]
    json: bool,
    /// No progress output
    #[arg(long)]
    quiet: bool,
}

#[derive(Args, Clone)]
struct GenArgs {
    #[arg(long, short, default_value = "")]
    prompt: String,
    #[arg(long, default_value = "blurry, low quality, watermark, text")]
    negative: String,
    /// comfyui or worker (default from config)
    #[arg(long)]
    backend: Option<String>,
    #[arg(long, short)]
    model: Option<String>,
    #[arg(long, default_value_t = 1024)]
    width: u32,
    #[arg(long, default_value_t = 1024)]
    height: u32,
    #[arg(long, default_value_t = 24)]
    steps: u32,
    #[arg(long, default_value_t = 7.0)]
    cfg: f32,
    #[arg(long)]
    seed: Option<u64>,
    /// Number of images
    #[arg(long, default_value_t = 1)]
    count: u32,
    /// Output file or folder (default: current folder)
    #[arg(long, short)]
    output: Option<PathBuf>,
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            let code = if e.downcast_ref::<photokit_ai::AiError>().is_some() {
                exit::BACKEND
            } else if e.to_string().contains("already exists") {
                exit::EXISTS
            } else {
                exit::FAILED
            };
            ExitCode::from(code)
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    match cli.cmd {
        Cmd::Resize { inputs, size, out } => {
            let op = size.op().context("give --width, --height or --percent")?;
            transform(&inputs, vec![op], &out, "resized")
        }
        Cmd::Crop { inputs, aspect, x, y, width, height, out } => match aspect {
            Some(a) => crop_aspect(&inputs, parse_aspect(&a)?, &out),
            None => {
                let (width, height) = (width.context("--width is required")?, height.context("--height is required")?);
                transform(&inputs, vec![Op::Crop { x, y, width, height }], &out, "cropped")
            }
        },
        Cmd::Rotate { inputs, degrees, straighten, out } => {
            let op = if straighten { Op::Straighten { degrees } } else { Op::Rotate { degrees } };
            transform(&inputs, vec![op], &out, "rotated")
        }
        Cmd::Flip { inputs, vertical, out } => {
            let axis = if vertical { Flip::Vertical } else { Flip::Horizontal };
            transform(&inputs, vec![Op::Flip { axis }], &out, "flipped")
        }
        Cmd::Convert { inputs, size, out } => {
            if out.format.is_none() && out.output.as_deref().and_then(OutputFormat::from_path).is_none() {
                bail!("give --format (jpeg, png, webp, avif, …) or an output file with an extension");
            }
            transform(&inputs, size.op().into_iter().collect(), &out, "")
        }
        Cmd::Optimize { inputs, mut out } => {
            if out.quality.is_none() {
                out.quality = Some(82);
            }
            transform(&inputs, vec![], &out, "optimized")
        }
        Cmd::Info { inputs, json } => info(&inputs, json),
        Cmd::Batch { inputs, preset: preset_name, recursive, name, dry_run, size, out } => {
            let files = expand(&inputs, recursive)?;
            let mut spec = BatchSpec { dry_run, ..Default::default() };
            if let Some(n) = &preset_name {
                let p = preset::find(n).with_context(|| format!("unknown preset '{n}' (see `photokit presets`)"))?;
                spec.ops = p.ops();
                spec.format = p.output.format;
                spec.encode = p.encode_options(OutputFormat::Jpeg);
                spec.target_bytes = p.output.target_kb.map(|k| k * 1024);
                if let Some(pat) = p.name_pattern {
                    spec.name_pattern = pat;
                }
            }
            if let Some(op) = size.op() {
                spec.ops = vec![op];
            }
            apply_out(&mut spec, &out)?;
            if let Some(n) = name {
                spec.name_pattern = n;
            }
            spec.output_dir = Some(match &out.output {
                Some(o) => o.clone(),
                None => default_batch_dir(&inputs),
            });
            run_batch(&files, &spec, &out)
        }
        Cmd::Rename {
            inputs,
            pattern,
            start,
            step,
            sort,
            case,
            normalize_ext,
            find,
            replace,
            recursive,
            dry_run,
            json,
        } => {
            let files = expand(&inputs, recursive)?;
            let opts = RenameOptions {
                pattern,
                start,
                step,
                case: match case.as_str() {
                    "lower" => Case::Lower,
                    "upper" => Case::Upper,
                    _ => Case::Keep,
                },
                normalize_ext,
                sort: match sort.as_str() {
                    "date" => SortBy::Date,
                    "size" => SortBy::Size,
                    _ => SortBy::Name,
                },
                find,
                replace,
            };
            let plan = rename::plan(&files, &opts)?;
            let renamed = if dry_run { 0 } else { rename::apply(&plan)? };
            if json {
                println!("{}", serde_json::json!({ "items": plan, "renamed": renamed, "dry_run": dry_run }));
            } else {
                for item in &plan {
                    let from = item.from.file_name().unwrap_or_default().to_string_lossy();
                    let to = item.to.file_name().unwrap_or_default().to_string_lossy();
                    if item.changed {
                        println!("{from}  ->  {to}{}", if item.conflict { "  (numbered: name taken)" } else { "" });
                    }
                }
                let changed = plan.iter().filter(|i| i.changed).count();
                if dry_run {
                    eprintln!("{changed} of {} files would be renamed (dry run)", plan.len());
                } else {
                    eprintln!("renamed {renamed} of {} files", plan.len());
                }
            }
            Ok(0)
        }
        Cmd::Presets { json } => {
            let all = preset::all();
            if json {
                println!("{}", serde_json::to_string_pretty(&all)?);
            } else {
                for p in all {
                    println!("{:<10} {}", p.name, p.description);
                }
            }
            Ok(0)
        }
        Cmd::Serve { port, host, open } => {
            let port = port.unwrap_or_else(|| Config::load().map(|c| c.server.port).unwrap_or(7878));
            let server = photokit_server::start(photokit_server::ServerOptions {
                host: host.clone(),
                port,
                ..Default::default()
            })?;
            let url = server.url();
            if host != "127.0.0.1" && host != "localhost" {
                eprintln!("warning: listening on {host}; anyone who can reach this address and knows the page token can read your files");
            }
            println!("PhotoKit is running at {url}  (Ctrl+C to stop)");
            if open {
                open_browser(&url);
            }
            server.join();
            Ok(0)
        }
        Cmd::Ai { cmd } => ai(cmd),
        Cmd::Config { cmd } => {
            match cmd {
                ConfigCmd::Show => print!("{}", toml_string(&Config::load()?)?),
                ConfigCmd::Path => println!(
                    "{}",
                    photokit_core::config::config_path().map(|p| p.display().to_string()).unwrap_or_default()
                ),
                ConfigCmd::Set { key, value } => {
                    let mut cfg = Config::load()?;
                    cfg.set(&key, &value)?;
                    println!("saved {}", cfg.save()?.display());
                }
            }
            Ok(0)
        }
    }
}

fn toml_string(cfg: &Config) -> Result<String> {
    Ok(toml::to_string_pretty(cfg)?)
}

/// "16:9" → 1.777…
fn parse_aspect(a: &str) -> Result<f32> {
    let (w, h) = a.split_once(':').context("aspect must look like 16:9")?;
    let (w, h): (f32, f32) = (w.trim().parse()?, h.trim().parse()?);
    if w <= 0.0 || h <= 0.0 {
        bail!("aspect values must be positive");
    }
    Ok(w / h)
}

/// Center crop each image to `ratio` (width / height).
fn crop_aspect(inputs: &[PathBuf], ratio: f32, out: &OutArgs) -> Result<u8> {
    let files = expand(inputs, false)?;
    let mut worst = 0;
    for f in &files {
        let (w, h) = codecs::probe_dimensions(f).with_context(|| format!("cannot read {}", f.display()))?;
        let (cw, ch) = if w as f32 / h as f32 > ratio {
            (((h as f32) * ratio).round() as u32, h)
        } else {
            (w, ((w as f32) / ratio).round() as u32)
        };
        let op = Op::Crop { x: (w - cw) / 2, y: (h - ch) / 2, width: cw.max(1), height: ch.max(1) };
        worst = worst.max(transform(std::slice::from_ref(f), vec![op], out, "cropped")?);
    }
    Ok(worst)
}

fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim().to_ascii_lowercase();
    let (n, mul) = if let Some(n) = t.strip_suffix("mb").or_else(|| t.strip_suffix('m')) {
        (n, 1024.0 * 1024.0)
    } else if let Some(n) = t.strip_suffix("kb").or_else(|| t.strip_suffix('k')) {
        (n, 1024.0)
    } else if let Some(n) = t.strip_suffix('b') {
        (n, 1.0)
    } else {
        (t.as_str(), 1024.0)
    };
    let v: f64 = n.trim().parse().with_context(|| format!("invalid size '{s}'"))?;
    Ok((v * mul) as u64)
}

fn apply_out(spec: &mut BatchSpec, out: &OutArgs) -> Result<()> {
    if let Some(f) = out.format {
        spec.format = Some(f);
    }
    if let Some(q) = out.quality {
        spec.encode.quality = q.clamp(1, 100);
    }
    if out.lossless {
        spec.encode.lossless = true;
    }
    if out.keep_metadata {
        spec.encode.keep_icc = true;
    }
    if let Some(t) = &out.target_size {
        spec.target_bytes = Some(parse_size(t)?);
    }
    spec.overwrite = out.overwrite;
    Ok(())
}

/// Folders become their image files; files are kept as given.
fn expand(inputs: &[PathBuf], recursive: bool) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for i in inputs {
        if i.is_dir() {
            files.extend(rename::collect_images(i, recursive)?);
        } else if i.exists() {
            files.push(i.clone());
        } else {
            bail!("{} does not exist", i.display());
        }
    }
    if files.is_empty() {
        bail!("no supported images found");
    }
    Ok(files)
}

fn default_batch_dir(inputs: &[PathBuf]) -> PathBuf {
    let base = inputs
        .first()
        .map(|i| if i.is_dir() { i.clone() } else { i.parent().map(Path::to_path_buf).unwrap_or_default() })
        .unwrap_or_default();
    base.join("photokit-out")
}

/// Shared implementation of the single-purpose commands.
fn transform(inputs: &[PathBuf], ops: Vec<Op>, out: &OutArgs, suffix: &str) -> Result<u8> {
    let files = expand(inputs, false)?;
    let mut spec = BatchSpec { ops, ..Default::default() };
    apply_out(&mut spec, out)?;

    // Single explicit output file.
    if let (Some(o), [file]) = (&out.output, files.as_slice()) {
        if !o.is_dir() && o.extension().is_some() {
            if spec.format.is_none() {
                spec.format = OutputFormat::from_path(o);
            }
            if o.exists() && !out.overwrite {
                bail!("{} already exists (use --overwrite)", o.display());
            }
            let item = batch::process_file(file, o, &spec)?;
            report_single(&item, out);
            return Ok(0);
        }
    }

    spec.output_dir = out.output.clone();
    spec.name_pattern =
        if suffix.is_empty() || spec.output_dir.is_some() { "{name}".into() } else { format!("{{name}}-{suffix}") };
    run_batch(&files, &spec, out)
}

fn report_single(item: &batch::ItemResult, out: &OutArgs) {
    if out.json {
        println!("{}", serde_json::to_string(item).unwrap_or_default());
    } else if !out.quiet {
        println!(
            "{} -> {}  {}x{}  {} -> {}",
            item.input.display(),
            item.output.display(),
            item.width,
            item.height,
            human(item.input_bytes),
            human(item.output_bytes)
        );
    }
}

fn run_batch(files: &[PathBuf], spec: &BatchSpec, out: &OutArgs) -> Result<u8> {
    let cancel = Arc::new(AtomicBool::new(false));
    let done = AtomicUsize::new(0);
    let total = files.len();
    let tty = std::io::stderr().is_terminal() && !out.quiet && !out.json;
    let report: BatchReport = batch::run(files, spec, &cancel, |item| {
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if out.json || out.quiet {
            return;
        }
        if let Some(e) = &item.error {
            eprintln!("{}fail {}: {e}", if tty { "\r\x1b[K" } else { "" }, item.input.display());
        } else if spec.dry_run || total <= 20 || !tty {
            println!(
                "{}{} -> {}  {}x{}  {}",
                if tty { "\r\x1b[K" } else { "" },
                item.input.display(),
                item.output.display(),
                item.width,
                item.height,
                human(item.output_bytes)
            );
        }
        if tty {
            eprint!("\r\x1b[K[{n}/{total}]");
        }
    });
    if tty {
        eprint!("\r\x1b[K");
    }
    if out.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else if !out.quiet {
        let saved = if report.input_bytes > 0 {
            100.0 - report.output_bytes as f64 * 100.0 / report.input_bytes as f64
        } else {
            0.0
        };
        eprintln!(
            "{}{} ok, {} failed in {:.2}s  {} -> {} ({:+.0}%)",
            if spec.dry_run { "dry run: " } else { "" },
            report.succeeded,
            report.failed,
            report.elapsed_ms as f64 / 1000.0,
            human(report.input_bytes),
            human(report.output_bytes),
            -saved
        );
    }
    Ok(if report.failed == 0 {
        0
    } else if report.succeeded > 0 {
        exit::PARTIAL
    } else {
        exit::FAILED
    })
}

fn info(inputs: &[PathBuf], json: bool) -> Result<u8> {
    let files = expand(inputs, false)?;
    let mut failed = false;
    for f in files {
        match codecs::decode_file(&f) {
            Ok(d) => {
                let v = serde_json::json!({
                    "path": f,
                    "format": d.format.map(|x| x.to_string()),
                    "width": d.image.width(),
                    "height": d.image.height(),
                    "color": format!("{:?}", d.image.color()),
                    "bytes": d.source_bytes,
                    "icc_profile": d.icc_profile.is_some(),
                });
                if json {
                    println!("{v}");
                } else {
                    println!(
                        "{}  {}  {}x{}  {:?}  {}{}",
                        f.display(),
                        d.format.map(|x| x.to_string()).unwrap_or_else(|| "?".into()),
                        d.image.width(),
                        d.image.height(),
                        d.image.color(),
                        human(d.source_bytes),
                        if d.icc_profile.is_some() { "  ICC" } else { "" }
                    );
                }
            }
            Err(e) => {
                failed = true;
                eprintln!("{}: {e:#}", f.display());
            }
        }
    }
    Ok(if failed { exit::PARTIAL } else { 0 })
}

fn ai(cmd: AiCmd) -> Result<u8> {
    let cfg = Config::load()?;
    match cmd {
        AiCmd::Backends { json } => {
            let mut rows = Vec::new();
            for b in photokit_ai::backends(&cfg) {
                let h = b.health();
                let caps = if h.ok { b.discover_capabilities().ok() } else { None };
                rows.push(serde_json::json!({ "id": b.id(), "name": b.name(), "health": h, "capabilities": caps }));
            }
            let ollama = photokit_ai::ollama::Ollama::new(&cfg.ollama).health();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "backends": rows, "ollama": ollama }))?
                );
            } else {
                for r in &rows {
                    let ok = r["health"]["ok"].as_bool().unwrap_or(false);
                    println!(
                        "{} {:<12} {}",
                        if ok { "●" } else { "○" },
                        r["id"].as_str().unwrap_or(""),
                        r["health"]["message"].as_str().unwrap_or("")
                    );
                    if let Some(ops) = r["capabilities"]["operations"].as_array() {
                        let ops: Vec<&str> = ops.iter().filter_map(|o| o.as_str()).collect();
                        println!("               {}", ops.join(", "));
                    }
                    for n in r["capabilities"]["notes"].as_array().into_iter().flatten() {
                        println!("               note: {}", n.as_str().unwrap_or(""));
                    }
                }
                println!("{} {:<12} {}", if ollama.ok { "●" } else { "○" }, "ollama", ollama.message);
            }
            Ok(0)
        }
        AiCmd::Models { backend, json } => {
            let b = photokit_ai::backend(&cfg, backend.as_deref())?;
            let models = b.list_models()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&models)?);
            } else {
                for m in models {
                    println!("{:<11} {}", m.kind, m.name);
                }
            }
            Ok(0)
        }
        AiCmd::Generate { gen } => generate(&cfg, &gen, Operation::TextToImage, None, None, |_| {}),
        AiCmd::Img2img { input, strength, gen } => {
            generate(&cfg, &gen, Operation::ImageToImage, Some(&input), None, |r| r.denoise = strength)
        }
        AiCmd::Inpaint { input, mask, gen } => {
            generate(&cfg, &gen, Operation::Inpaint, Some(&input), Some(&mask), |_| {})
        }
        AiCmd::Outpaint { input, left, top, right, bottom, gen } => {
            generate(&cfg, &gen, Operation::Outpaint, Some(&input), None, |r| r.pad = [left, top, right, bottom])
        }
        AiCmd::Upscale { input, scale, gen } => {
            let dims = codecs::probe_dimensions(&input);
            generate(&cfg, &gen, Operation::Upscale, Some(&input), None, |r| {
                r.scale = scale;
                if let Some((w, h)) = dims {
                    r.width = w.clamp(64, 4096);
                    r.height = h.clamp(64, 4096);
                }
            })
        }
        AiCmd::RemoveBg { input, gen } => generate(&cfg, &gen, Operation::RemoveBackground, Some(&input), None, |r| {
            r.width = 1024;
            r.height = 1024;
        }),
        AiCmd::Enhance { prompt } => {
            println!("{}", photokit_ai::ollama::Ollama::new(&cfg.ollama).enhance_prompt(&prompt)?);
            Ok(0)
        }
        AiCmd::Assist { input, request, apply, out } => {
            let plan = photokit_ai::ollama::Ollama::new(&cfg.ollama).assist(&request)?;
            eprintln!("plan: {}", if plan.summary.is_empty() { "(no summary)" } else { &plan.summary });
            eprintln!("{}", serde_json::to_string_pretty(&plan)?);
            if !apply {
                eprintln!("re-run with --apply to write the result");
                return Ok(0);
            }
            let mut out = out;
            out.format = out.format.or(plan.format);
            out.quality = out.quality.or(plan.quality);
            transform(&[input], plan.ops, &out, "edited")
        }
    }
}

fn generate(
    cfg: &Config,
    g: &GenArgs,
    op: Operation,
    input: Option<&Path>,
    mask: Option<&Path>,
    tweak: impl FnOnce(&mut JobRequest),
) -> Result<u8> {
    let backend = photokit_ai::backend(cfg, g.backend.as_deref())?;
    let mut req = JobRequest {
        operation: op,
        prompt: g.prompt.clone(),
        negative_prompt: g.negative.clone(),
        model: g.model.clone(),
        width: g.width,
        height: g.height,
        steps: g.steps,
        cfg: g.cfg,
        seed: g.seed,
        batch: g.count,
        ..Default::default()
    };
    if let Some(i) = input {
        req.source = Some(std::fs::read(i).with_context(|| format!("cannot read {}", i.display()))?);
    }
    if let Some(m) = mask {
        req.mask = Some(std::fs::read(m).with_context(|| format!("cannot read {}", m.display()))?);
    }
    tweak(&mut req);
    let cancel = AtomicBool::new(false);
    let tty = std::io::stderr().is_terminal() && !g.json;
    let images = photokit_ai::run_job(backend.as_ref(), &req, &cancel, |p| {
        if tty {
            match p.fraction {
                Some(f) => eprint!("\r\x1b[K{:?} {:>3.0}%", p.state, f * 100.0),
                None => eprint!("\r\x1b[K{:?}…", p.state),
            }
        }
    })?;
    if tty {
        eprint!("\r\x1b[K");
    }

    let mut written = Vec::new();
    for (i, img) in images.iter().enumerate() {
        let mut data = img.data.clone();
        let default_name = format!("photokit-{}-{}.png", op.key().replace('_', "-"), img.seed.unwrap_or(i as u64));
        let path = match &g.output {
            Some(o) if o.is_dir() => naming::unique_path(&o.join(&default_name)),
            Some(o) if images.len() == 1 => o.clone(),
            Some(o) => {
                let stem = o.file_stem().unwrap_or_default().to_string_lossy();
                naming::unique_path(&o.with_file_name(format!("{stem}-{}.png", i + 1)))
            }
            None => naming::unique_path(&PathBuf::from(&default_name)),
        };
        // Re-encode when the requested extension is not PNG.
        if let Some(fmt) = OutputFormat::from_path(&path).filter(|f| *f != OutputFormat::Png) {
            let d = codecs::decode_bytes(&data)?;
            data = codecs::encode(&d.image, &EncodeOptions { format: fmt, quality: 92, ..Default::default() }, None)?;
        }
        naming::write_atomic(&path, &data)?;
        written.push(serde_json::json!({ "path": path, "seed": img.seed, "bytes": data.len() }));
        if !g.json {
            println!("{}  (seed {})", path.display(), img.seed.map(|s| s.to_string()).unwrap_or_else(|| "?".into()));
        }
    }
    if g.json {
        println!("{}", serde_json::to_string_pretty(&written)?);
    }
    Ok(0)
}

fn human(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{:.2} MB", n as f64 / 1024.0 / 1024.0),
    }
}

fn open_browser(url: &str) {
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if result.is_err() {
        eprintln!("open {url} in your browser");
    }
}
