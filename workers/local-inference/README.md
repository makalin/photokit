# PhotoKit local inference worker

An optional, isolated Python process that runs diffusion models with
[diffusers](https://github.com/huggingface/diffusers). PhotoKit talks to it
over `http://127.0.0.1:7879` (config key `worker.url`). Crashes or out-of-memory
errors here never take down PhotoKit's image tools.

```bash
cd workers/local-inference
python3 -m venv .venv
source .venv/bin/activate          # Windows: .venv\Scripts\activate
pip install -r requirements.txt

# Real inference (downloads the model on first use):
python worker.py --model stabilityai/sdxl-turbo

# No GPU / quick test: procedural images, only Pillow required
python worker.py --mock
```

Then pick **Local worker** as the backend in AI Studio, or run
`photokit ai generate --backend worker --prompt "..."`.

| Mode | Operations |
|---|---|
| `--mock` | all six (procedural placeholders) |
| diffusers | text to image, image to image, inpaint; background removal if `rembg` is installed |

GPU selection is automatic: CUDA, then Apple Metal (MPS), then CPU. Jobs run
one at a time to avoid exhausting VRAM.
