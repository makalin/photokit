<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="photokit-logo.svg">
    <img src="assets/photokit-logo-dark-text.svg" alt="PhotoKit" width="420">
  </picture>
</p>

# PhotoKit

**Fast Image Tools & Local AI Studio**

PhotoKit is a lightweight, privacy-first toolkit for everyday image work and AI-powered image creation. Resize a JPEG in seconds, rename and convert a whole folder of photos, generate new images with local diffusion models, or edit an existing image with a prompt — without launching a heavyweight editor.

It ships as three front-ends on one Rust engine:

| | How to run | What it is |
|---|---|---|
| **CLI** | `photokit …` | Scriptable commands for single files and folders |
| **Web app** | `photokit serve --open` → http://127.0.0.1:7878 | The full UI in your browser, served from your own machine |
| **Desktop app** | PhotoKit.app / PhotoKit setup.exe | Native macOS and Windows window around the same UI (Tauri 2) |

## Quick start

Requirements: [Rust](https://rustup.rs) 1.88+. For the desktop app also the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (Xcode command line tools on macOS,
WebView2 + MSVC build tools on Windows).

| Task | macOS | Windows (PowerShell) |
|---|---|---|
| Web app on localhost | `scripts/web.sh` | `scripts\web.ps1` |
| Desktop app (dev) | `scripts/desktop.sh` | `scripts\desktop.ps1` |
| Install the `photokit` CLI | `scripts/install-cli.sh` | `scripts\install-cli.ps1` |
| Build installers | `scripts/package.sh` → `.app`, `.dmg` | `scripts\package.ps1` → `.msi`, setup `.exe` |

GitHub Actions (`.github/workflows/build.yml`) runs the tests and builds the CLI and the
installers on macOS and Windows.

```bash
# Rename a folder of photos, then make web-ready copies
photokit rename ~/Pictures/trip -p "trip-{n:3}" --normalize-ext
photokit batch ~/Pictures/trip --preset web          # -> ~/Pictures/trip/photokit-out

# AI (start ComfyUI, or the bundled worker: python workers/local-inference/worker.py --mock)
photokit ai backends
photokit ai generate -p "retro computer on a desk at sunset" -o desk.png
```

Full command reference: [docs/cli.md](docs/cli.md).

## Principles

- **Instant for everyday tasks:** basic image operations never load an AI model.
- **Local first:** processing and AI inference stay on-device by default.
- **One engine, two interfaces:** Rust core shared by CLI and desktop GUI.
- **Modular AI:** connect ComfyUI, use compatible image-generation backends, or run supported models through a managed local worker.
- **No unnecessary frontend stack:** Tauri 2 with vanilla HTML, CSS and JavaScript; no Electron, React or Vite requirement.
- **Non-destructive:** originals are preserved unless explicitly overwritten.

## Features

### 1. Fast image tools

- Resize by width, height, percentage or fit mode (contain, cover, fill)
- Crop, rotate, flip and straighten
- Read JPEG, PNG, WebP, GIF, BMP, TIFF; write those plus AVIF
- Compress and optimize with target quality or target file size
- **Bulk rename** with patterns (`{parent}-{n:3}`, `{date}`, `{w}x{h}`, find/replace, case), previewed before applying
- Batch process whole folders in parallel, with dry runs and collision-safe names
- Metadata stripped by default; ICC color profile optionally kept; EXIF orientation applied
- Export presets for web, social posts, stories, thumbnails, email and archive
- Drag-and-drop, clipboard paste, before/after preview, undo/redo
- Transparent background and alpha-aware conversion

### 2. AI image studio

- **Text-to-image:** create images from prompts
- **Image-to-image:** transform a supplied image while preserving chosen structure
- **Inpainting:** mask and replace or remove selected regions
- **Outpainting:** extend the canvas beyond the original frame
- **Background removal and replacement** using supported local models
- **Upscaling and restoration** using compatible super-resolution models
- **Smart crop and subject-aware framing**
- **Prompt presets, seeds, negative prompts, model selection and generation history**
- **Optional AI assistant:** interpret natural-language requests into safe, previewable actions

Capabilities depend on the selected model and backend. PhotoKit will expose unsupported operations as unavailable rather than claiming every model can perform every task.

### 3. Local AI backends

| Backend | Purpose | Strategy |
|---|---|---|
| **ComfyUI** | Primary image generation/editing backend | Connect to an existing local ComfyUI instance via its HTTP/WebSocket API; submit workflows, monitor progress, collect results |
| **Managed local worker** | Embedded-style local inference | Optional separate Python worker using supported diffusion libraries; downloaded and launched only when needed |
| **Ollama** | Prompt rewriting, tagging and assistant commands | Optional local LLM integration; image generation is **not** assumed to be an Ollama capability |
| **Remote provider adapters** | Optional cloud image generation | Disabled by default; explicit opt-in, clear upload notice and credentials stored securely |

**Important:** ComfyUI is the recommended MVP image-generation engine. A native Rust diffusion runtime is a potential future optimization, not an initial dependency or performance promise.

## Desktop experience

The web and desktop apps share one interface with four workspaces:

1. **Quick Tools** — drop an image, choose size/format/quality, export immediately.
2. **Batch & Rename** — pick a folder, preview and apply pattern renames, then resize/convert everything in parallel with a live log.
3. **AI Studio** — enter a prompt, select a model, generate/edit/upscale images, view results.
4. **Settings** — configure ComfyUI, the local worker, Ollama and export defaults.

### Quick Tools layout

```text
┌───────────────────────────────────────────────────────────────┐
│ PhotoKit       Quick Tools | Batch | AI Studio | Models       │
├───────────────────────────────────────────────────────────────┤
│                                                               │
│       Drop image here / Paste / Browse                        │
│                                                               │
│  Width [ 1200 ]  Height [ Auto ]  [✓] Keep aspect ratio       │
│  Format [ WebP ▼ ]  Quality [ 85 ─────●── ]                   │
│  Fit [ Contain ▼ ]   Output [ Original folder ▼ ]             │
│                                                               │
│  Original: 3.2 MB         Estimated: calculated on preview     │
│                                      [ Process Image ]        │
└───────────────────────────────────────────────────────────────┘
```

### AI Studio layout

```text
┌───────────────────────────────────────────────────────────────┐
│ PhotoKit       Quick Tools | Batch | AI Studio | Models       │
├───────────────────────────────────────────────────────────────┤
│  Prompt                                                      │
│  [ A retro-futurist city at sunset...                   ]     │
│  Mode [Text to Image ▼]   Backend [ComfyUI ▼]                  │
│  Model [Configured model ▼]  Size [1024 × 1024 ▼]             │
│  Steps [ 24 ]   Seed [ Random ]                               │
│                                                               │
│  [ Source / Mask ]         [ Generated Preview ]              │
│                                                               │
│  [ Generate ]  [ Save ]  [ Send to Quick Tools ]               │
└───────────────────────────────────────────────────────────────┘
```

The app should remain usable when no AI backend is installed.

## CLI design

The CLI binary is `photokit`.

```bash
# Resize to a maximum width, preserving aspect ratio
photokit resize photo.jpg --width 1200

# Crop to exact dimensions
photokit resize photo.jpg --width 1200 --height 800 --fit cover

# Compress and convert
photokit convert photo.jpg --format webp --quality 82

# Optimize a folder
photokit batch ./assets --preset web --output ./optimized

# Strip metadata
photokit optimize photo.jpg --strip-metadata

# Generate using an available ComfyUI workflow
photokit ai generate --prompt "retro computer on a desk" --model my-model --width 1024 --height 1024

# Inpaint using a mask and a configured workflow
photokit ai inpaint input.png --mask mask.png --prompt "remove the cable"

# Upscale with a compatible local backend
photokit ai upscale image.png --scale 2

# List backends and installed models
photokit ai backends
photokit ai models
```

Commands support `--output`, `--overwrite`, `--json`, `--quiet` and meaningful exit codes; batch and rename support dry runs and collision-safe file names. See [docs/cli.md](docs/cli.md).

## Technology stack

| Layer | Technology |
|---|---|
| Native core and CLI | Rust, `clap`, `rayon` |
| Image processing | `image`, `fast_image_resize` |
| Codec/optimization adapters | `image` codecs, `libwebp` for lossy WebP, `ravif` for AVIF, `oxipng` for PNG |
| Local web server | `tiny_http`, UI embedded in the binary |
| Desktop | Tauri 2 (wraps the same server and UI) |
| UI | Vanilla HTML, CSS, JavaScript |
| AI generation | ComfyUI integration first |
| Optional local inference | Managed Python worker, isolated environment |
| Local LLM | Ollama integration |
| Configuration | TOML |
| Generation history | PNG files with JSON sidecars in the app data folder |
| Distribution | Native Windows and macOS installers; portable CLI binaries |

Codec licenses, platform compatibility, binary sizes and GPU acceleration must be evaluated before distribution.

## Architecture

```text
photokit/
├── Cargo.toml                 # workspace
├── crates/
│   ├── photokit-codecs/       # decode/encode, target size, PNG optimization
│   ├── photokit-core/         # ops, batch, rename, naming, presets, config
│   ├── photokit-ai/           # backend contract, ComfyUI / worker / Ollama adapters
│   ├── photokit-server/       # localhost web server + JSON API
│   └── photokit-cli/          # `photokit` binary
├── apps/
│   ├── web/ui/                # HTML/CSS/JS shared by browser and desktop
│   └── desktop/src-tauri/     # Tauri 2 app (macOS + Windows)
├── workers/local-inference/   # optional Python diffusers worker (--mock for testing)
├── workflows/comfyui/         # API-format workflow templates
├── presets/                   # web, social, story, thumbnail, email, archive
├── scripts/                   # run / install / package for macOS (.sh) and Windows (.ps1)
├── assets/                    # logo variants and app icon
└── docs/                      # cli.md, architecture.md, ai-backends.md
```

See [docs/architecture.md](docs/architecture.md) and [docs/ai-backends.md](docs/ai-backends.md).

### Processing flow

```text
Image input ──> Validation ──> Rust processing pipeline ──> Export
                                      ↑
                                CLI / Desktop

Prompt / Source image ──> AI job queue ──> Backend adapter
                                              ├── ComfyUI
                                              ├── Managed local worker
                                              └── Optional remote provider
                                         └── Result ──> Preview / Export
```

### Backend contract

Every AI adapter will implement a common job interface:

- `discover_capabilities()` — report supported operations, model requirements and limits
- `list_models()` — enumerate compatible available models
- `validate_job()` — validate inputs and resource requirements
- `submit_job()` — start work and return a job ID
- `get_progress()` — expose progress, errors and previews when available
- `cancel_job()` — attempt cancellation where supported
- `collect_results()` — import output into PhotoKit's workspace

The UI will never assume a backend supports inpainting, upscaling or text-to-image without capability discovery.

## Model management and GPU support

- Scan configured local model directories without moving or duplicating models
- Register existing ComfyUI installations and workflows
- Show model format, file size and compatible backend
- Download models only with explicit consent, license/source information and checksums where available
- Allow CPU execution for supported operations and hardware acceleration when the backend supports it
- Detect compatible NVIDIA CUDA, AMD and Apple Silicon backends without promising universal support
- Keep model inference in a separate process so AI crashes do not bring down Quick Tools
- Allow cancellation and configurable concurrency to avoid exhausting VRAM

**Platform note:** Windows, macOS and Linux can use different GPU execution paths. Direct AMD acceleration is backend- and driver-dependent; ComfyUI compatibility must be validated per machine.

## Security and privacy

- Offline-first core features; no mandatory account or telemetry
- Local AI by default; external API calls require explicit configuration
- No silent image uploads
- Limit desktop filesystem permissions to user-selected paths
- Validate output destinations and sanitize filenames
- Do not execute arbitrary downloaded ComfyUI custom nodes automatically
- Display warnings for untrusted model files and workflows
- Store remote API keys using an appropriate OS credential store
- Preserve source files unless overwrite is explicitly requested

## Performance targets

These are engineering goals to benchmark, not measured results:

- CLI startup suitable for short, one-shot image tasks
- Quick Tools usable without initializing any AI services
- Responsive GUI during batch processing and model loading
- Parallel processing with configurable memory limits
- Lazy-load AI backend connections and model metadata
- Release builds with minimized binary size and startup overhead

## Development roadmap

### Milestone 1 — Useful on day one
- [x] Rust workspace and CLI
- [x] Resize, crop, rotate, straighten, convert and compress
- [x] Batch processing, bulk rename and presets
- [x] Collision-safe, non-destructive exports
- [x] Windows/macOS CI builds

### Milestone 2 — Lightweight desktop
- [x] Localhost web app and Tauri 2 shell sharing one vanilla UI
- [x] Drag-and-drop, clipboard and previews
- [x] Quick Tools and Batch & Rename workspaces
- [x] Progress and cancellation
- [ ] Windows Explorer and macOS Finder integration (optional)

### Milestone 3 — Local AI Studio
- [x] ComfyUI connection discovery and health checks
- [x] Text-to-image with workflow templates and capability checks
- [x] Model selector, generation parameters and progress
- [x] Image-to-image, inpainting, outpainting and upscaling
- [x] Generation gallery and export to Quick Tools
- [ ] Per-step ComfyUI progress over WebSocket

### Milestone 4 — Advanced AI
- [x] Optional managed local inference worker
- [x] Ollama-assisted prompts and natural-language actions
- [x] Background removal (worker with `rembg`, or a custom ComfyUI workflow)
- [ ] Smart crop
- [ ] Model manager, GPU diagnostics and resource limits
- [ ] Optional remote provider adapters

### Milestone 5 — Polish and distribution
- [x] Installer build scripts and CI artifacts
- [ ] Signed and notarized builds
- [ ] Benchmark suite
- [ ] Accessibility pass

## Testing

`cargo test` runs unit tests for every crate (resize planning, ops, codecs round
trips, target-size search, name templates, rename plans, batch runs, workflow
templates, the backend contract with a mock backend) plus CLI integration tests
for flags, output paths and exit codes. Planned additions:

- Golden-image tests for dimensions, color profiles and alpha behavior
- CLI integration tests for flags, output paths and exit codes
- Batch stress tests for large folders and corrupted images
- Backend contract tests with mocked AI jobs
- End-to-end ComfyUI tests on a supported local installation
- GPU-specific compatibility tests with documented results
- Privacy checks to ensure no unexpected network traffic in offline mode

## Non-goals

PhotoKit is not intended to replace Photoshop's full layer editor, professional retouching workflow or color-management suite. It focuses on quick image utilities and an approachable AI creation workspace.

## License

MIT — see [LICENSE](LICENSE). AI models you download are covered by their own licenses.

---

**PhotoKit — Edit fast. Create freely. Run locally.**
