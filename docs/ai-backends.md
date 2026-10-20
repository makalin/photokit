# AI backends

PhotoKit never assumes a backend can do something: the UI and CLI ask each
backend for its capabilities and only offer those operations.

| Operation | ComfyUI (bundled workflows) | Local worker (diffusers) | Local worker `--mock` |
|---|---|---|---|
| Text to image | ✓ checkpoint needed | ✓ | ✓ |
| Image to image | ✓ | ✓ | ✓ |
| Inpaint | ✓ | ✓ | ✓ |
| Outpaint | ✓ | – | ✓ |
| Upscale | ✓ upscale model needed | – | ✓ |
| Remove background | add `remove_background.json` | ✓ with `rembg` | ✓ |

Ollama is used only for text: prompt rewriting (`ai enhance`, ✨ in AI Studio)
and turning plain-language requests into edit plans (`ai assist`, Assistant in
Quick Tools). Plans are validated and shown before anything is written.

## ComfyUI (recommended)

1. Install [ComfyUI](https://github.com/comfyanonymous/ComfyUI) and put a checkpoint
   in `models/checkpoints` (and optionally an ESRGAN-style model in `models/upscale_models`).
2. Start it: `python main.py --listen 127.0.0.1 --port 8188`.
3. PhotoKit connects to `comfyui.url` (default `http://127.0.0.1:8188`).

Workflows live in `workflows/comfyui` (API format with `{{placeholders}}`).
Set `comfyui.workflow_dir` to a folder of your own to override them by file name.
An operation is available only if every node type its workflow uses exists on
the server, and PhotoKit never installs custom nodes.

Progress is reported as queued/running; ComfyUI exposes per-step progress only
over WebSocket, which is not wired up yet.

## Local worker

See [workers/local-inference/README.md](../workers/local-inference/README.md).
It runs in its own Python process (default `http://127.0.0.1:7879`), reports
per-step progress and supports cancellation. `--mock` produces procedural images
without torch, useful for trying the app and for tests.

## Post-processing

`run_job` applies the same finishing steps for every backend:

- **Upscale**: model upscalers have a fixed factor (usually 4×); the result is
  resized to exactly the requested scale.
- **Inpaint**: the generated image is blended back into the full-resolution
  source with a feathered mask, so unmasked pixels are untouched.

## Adding a backend

Implement `photokit_ai::Backend` and register it in `photokit_ai::backends()`.
The `Mock` backend in `crates/photokit-ai/src/lib.rs` tests shows the minimal contract.
