# Architecture

```text
                    ┌──────────────── photokit-core ────────────────┐
 photokit-cli ─────►│ ops (resize/crop/rotate/straighten/flip)      │
                    │ batch (parallel, rayon) · rename · naming      │
 photokit-server ──►│ presets · config                               │──► photokit-codecs
   ▲   (tiny_http)  └────────────────────────────────────────────────┘     decode / encode /
   │                                                                        target size / oxipng
   │ JSON API + embedded UI (apps/web/ui)
   │
   ├── browser   (photokit serve → http://127.0.0.1:7878)
   └── desktop   (Tauri window; the server runs in-process on a free port)

 photokit-ai: Backend trait ─┬─ ComfyUI adapter  (HTTP API + workflows/comfyui/*.json)
                             ├─ Local worker     (workers/local-inference, Python, diffusers)
                             └─ Ollama           (prompt rewriting, natural-language edit plans)
```

## Crates

| Crate | Role |
|---|---|
| `photokit-codecs` | Decoding (EXIF orientation applied, ICC captured), encoding for JPEG/PNG/WebP/AVIF/GIF/BMP/TIFF, quality search for a target size, PNG optimization with oxipng |
| `photokit-core` | Edit operations, SIMD resizing (`fast_image_resize`), batch runner, bulk rename, name templates, presets, configuration |
| `photokit-ai` | The backend contract (`discover_capabilities`, `list_models`, `validate_job`, `submit_job`, `get_progress`, `cancel_job`, `collect_results`), adapters, `run_job` driver and shared post-processing |
| `photokit-server` | Local HTTP server, background jobs, in-memory open images, thumbnail cache, AI gallery, embedded UI |
| `photokit-cli` | The `photokit` binary |
| `apps/desktop/src-tauri` | Tauri 2 shell that starts the server and opens a native window on it |

## One UI for web and desktop

The UI in `apps/web/ui` is plain HTML/CSS/JS embedded into the server binary at
compile time. The desktop app does not reimplement anything: it starts the same
server on a random loopback port and points a native webview at it (`?desktop=1`
hides browser-only actions such as downloads). Set `PHOTOKIT_UI_DIR=apps/web/ui`
to serve the files from disk while editing the UI.

## Security model

- The server binds to `127.0.0.1` by default.
- Each launch generates a 128-bit token, injected into the page; every `/api`
  call must present it, so other websites in the same browser cannot use the API.
- Files are only written where the user chose, or next to the source with a
  collision-safe name. Renames are two-phase and roll back on failure.
- AI backends are only contacted at the configured URLs; nothing is uploaded elsewhere.

## Performance notes

- Batch and rename never load AI code; AI adapters are plain HTTP clients.
- Batch jobs run on all cores with rayon; output names are planned up front so
  parallel workers never collide.
- Resizing uses SIMD convolution (Lanczos3); previews use bilinear.
- Release builds use thin LTO and one codegen unit; dev builds optimize
  dependencies so image code is usable while developing.
