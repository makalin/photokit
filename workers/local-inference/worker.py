#!/usr/bin/env python3
"""PhotoKit local inference worker.

A small HTTP/JSON server that runs diffusion models with Hugging Face
`diffusers`. PhotoKit talks to it through the same job contract it uses for
every backend:

    GET  /health                 engine and device
    GET  /capabilities           supported operations
    GET  /models                 configured models
    POST /jobs                   submit a job, returns {"id": ...}
    GET  /jobs/<id>              state, progress, message
    POST /jobs/<id>/cancel       request cancellation
    GET  /jobs/<id>/result       {"images": [{"data": <base64 png>, "seed": n}]}

Run `python worker.py --mock` to test the integration without a GPU or model
download: it produces procedural images with Pillow instead.
"""

from __future__ import annotations

import argparse
import base64
import io
import json
import os
import random
import threading
import traceback
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from PIL import Image, ImageDraw, ImageFilter, ImageOps

DEFAULT_MODEL = os.environ.get("PHOTOKIT_MODEL", "stabilityai/sdxl-turbo")
JOBS: dict[str, dict] = {}
LOCK = threading.Lock()


class Cancelled(Exception):
    pass


def png_b64(img: Image.Image) -> str:
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return base64.b64encode(buf.getvalue()).decode()


def load_b64(data: str | None, mode: str = "RGB") -> Image.Image | None:
    if not data:
        return None
    return Image.open(io.BytesIO(base64.b64decode(data))).convert(mode)


class MockEngine:
    """Procedural stand-in that exercises the full pipeline quickly."""

    name = "mock engine"
    device = "cpu"

    def capabilities(self) -> list[str]:
        return ["text_to_image", "image_to_image", "inpaint", "outpaint", "upscale", "remove_background"]

    def models(self) -> list[dict]:
        return [{"name": "mock-diffusion", "kind": "checkpoint"}, {"name": "mock-upscaler", "kind": "upscale"}]

    def run(self, job: dict, step) -> list[Image.Image]:
        req = job["request"]
        op = req["operation"]
        rng = random.Random(req.get("seed") or 0)
        steps = max(1, min(int(req.get("steps", 20)), 50))
        for i in range(steps):
            step(i + 1, steps)
        src = load_b64(req.get("source"))
        if op == "text_to_image":
            w, h = int(req["width"]), int(req["height"])
            a = tuple(rng.randrange(256) for _ in range(3))
            b = tuple(rng.randrange(256) for _ in range(3))
            grad = Image.linear_gradient("L").resize((w, h)).rotate(rng.randrange(360), expand=False)
            img = Image.composite(Image.new("RGB", (w, h), a), Image.new("RGB", (w, h), b), grad)
            draw = ImageDraw.Draw(img)
            for _ in range(12):
                x, y, r = rng.randrange(w), rng.randrange(h), rng.randrange(20, max(21, w // 5))
                draw.ellipse((x - r, y - r, x + r, y + r), outline=tuple(255 - c for c in a), width=4)
            draw.text((16, 16), req.get("prompt", "")[:80], fill=(255, 255, 255))
            return [img]
        if op == "image_to_image":
            tint = Image.new("RGB", src.size, tuple(rng.randrange(256) for _ in range(3)))
            return [Image.blend(src.filter(ImageFilter.GaussianBlur(2)), tint, float(req.get("denoise", 0.5)) * 0.6)]
        if op == "inpaint":
            mask = load_b64(req.get("mask"), "L").resize(src.size)
            fill = src.filter(ImageFilter.GaussianBlur(24))
            return [Image.composite(fill, src, mask)]
        if op == "outpaint":
            l, t, r, b = (int(v) for v in req.get("pad", [0, 0, 0, 0]))
            canvas = src.resize((src.width + l + r, src.height + t + b)).filter(ImageFilter.GaussianBlur(30))
            canvas.paste(src, (l, t))
            return [canvas]
        if op == "upscale":
            s = float(req.get("scale", 2))
            return [src.resize((int(src.width * s), int(src.height * s)), Image.LANCZOS)]
        if op == "remove_background":
            gray = ImageOps.grayscale(src)
            alpha = gray.point(lambda p: 0 if p > 235 else 255)
            out = src.convert("RGBA")
            out.putalpha(alpha)
            return [out]
        raise ValueError(f"unsupported operation {op}")


class DiffusersEngine:
    """Real inference via diffusers. Pipelines are loaded lazily per operation."""

    name = "diffusers"

    def __init__(self, model: str):
        import torch  # noqa: F401 - fail early with a clear message

        self.torch = torch
        self.model = model
        if torch.cuda.is_available():
            self.device, self.dtype = "cuda", torch.float16
        elif getattr(torch.backends, "mps", None) and torch.backends.mps.is_available():
            self.device, self.dtype = "mps", torch.float16
        else:
            self.device, self.dtype = "cpu", torch.float32
        self.pipes: dict[str, object] = {}
        try:
            import rembg  # noqa: F401

            self.has_rembg = True
        except ImportError:
            self.has_rembg = False

    def capabilities(self) -> list[str]:
        ops = ["text_to_image", "image_to_image", "inpaint"]
        if self.has_rembg:
            ops.append("remove_background")
        return ops

    def models(self) -> list[dict]:
        return [{"name": self.model, "kind": "checkpoint"}]

    def pipe(self, op: str):
        if op not in self.pipes:
            from diffusers import AutoPipelineForImage2Image, AutoPipelineForInpainting, AutoPipelineForText2Image

            cls = {
                "text_to_image": AutoPipelineForText2Image,
                "image_to_image": AutoPipelineForImage2Image,
                "inpaint": AutoPipelineForInpainting,
            }[op]
            # Reuse already-loaded weights where possible.
            base = next(iter(self.pipes.values()), None)
            p = cls.from_pipe(base) if base is not None else cls.from_pretrained(self.model, torch_dtype=self.dtype)
            self.pipes[op] = p.to(self.device)
        return self.pipes[op]

    def run(self, job: dict, step) -> list[Image.Image]:
        req = job["request"]
        op = req["operation"]
        if op == "remove_background":
            from rembg import remove

            return [remove(load_b64(req.get("source")))]
        steps = int(req.get("steps", 24))
        gen = self.torch.Generator(device="cpu").manual_seed(int(req.get("seed") or 0))

        def callback(pipe, i, t, kwargs):
            step(i + 1, steps)
            if job["cancel"]:
                raise Cancelled()
            return kwargs

        common = dict(
            prompt=req.get("prompt", ""),
            negative_prompt=req.get("negative_prompt") or None,
            num_inference_steps=steps,
            guidance_scale=float(req.get("cfg", 7.0)),
            generator=gen,
            num_images_per_prompt=int(req.get("batch", 1)),
            callback_on_step_end=callback,
        )
        if op == "text_to_image":
            out = self.pipe(op)(width=int(req["width"]), height=int(req["height"]), **common)
        elif op == "image_to_image":
            out = self.pipe(op)(image=load_b64(req["source"]), strength=float(req.get("denoise", 0.6)), **common)
        elif op == "inpaint":
            src = load_b64(req["source"])
            out = self.pipe(op)(
                image=src, mask_image=load_b64(req["mask"], "L"), width=src.width, height=src.height, **common
            )
        else:
            raise ValueError(f"unsupported operation {op}")
        return out.images


def run_job(engine, job_id: str) -> None:
    job = JOBS[job_id]

    def step(i: int, n: int) -> None:
        job["progress"] = i / n
        job["state"] = "running"
        if job["cancel"]:
            raise Cancelled()

    try:
        images = engine.run(job, step)
        seed = int(job["request"].get("seed") or 0)
        job["images"] = [{"data": png_b64(img), "seed": seed + i} for i, img in enumerate(images)]
        job["state"], job["progress"] = "completed", 1.0
    except Cancelled:
        job["state"] = "cancelled"
    except Exception as e:  # report every failure to the client
        traceback.print_exc()
        job["state"], job["message"] = "failed", str(e)


def make_handler(engine):
    run_lock = threading.Lock()  # one GPU job at a time

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, fmt, *args):
            pass

        def send(self, code: int, body: dict) -> None:
            data = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            parts = [p for p in self.path.split("?")[0].split("/") if p]
            if parts == ["health"]:
                return self.send(200, {"ok": True, "engine": engine.name, "device": engine.device})
            if parts == ["capabilities"]:
                return self.send(200, {"operations": engine.capabilities(), "notes": []})
            if parts == ["models"]:
                return self.send(200, {"models": engine.models()})
            if len(parts) >= 2 and parts[0] == "jobs" and parts[1] in JOBS:
                job = JOBS[parts[1]]
                if len(parts) == 3 and parts[2] == "result":
                    if job["state"] != "completed":
                        return self.send(409, {"error": "job not completed"})
                    return self.send(200, {"images": job["images"]})
                return self.send(200, {k: job[k] for k in ("state", "progress", "message")})
            self.send(404, {"error": "not found"})

        def do_POST(self):
            parts = [p for p in self.path.split("?")[0].split("/") if p]
            length = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(length) or b"{}")
            if parts == ["jobs"]:
                if body.get("operation") not in engine.capabilities():
                    return self.send(400, {"error": f"unsupported operation {body.get('operation')}"})
                job_id = uuid.uuid4().hex
                JOBS[job_id] = {"request": body, "state": "queued", "progress": 0.0, "message": None, "cancel": False}

                def work():
                    with run_lock:
                        if not JOBS[job_id]["cancel"]:
                            run_job(engine, job_id)
                        else:
                            JOBS[job_id]["state"] = "cancelled"

                threading.Thread(target=work, daemon=True).start()
                return self.send(200, {"id": job_id})
            if len(parts) == 3 and parts[0] == "jobs" and parts[2] == "cancel" and parts[1] in JOBS:
                JOBS[parts[1]]["cancel"] = True
                return self.send(200, {"ok": True})
            self.send(404, {"error": "not found"})

    return Handler


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=7879)
    ap.add_argument("--model", default=DEFAULT_MODEL, help="diffusers model id or local path")
    ap.add_argument("--mock", action="store_true", help="procedural images, no torch needed")
    args = ap.parse_args()

    engine = MockEngine() if args.mock else DiffusersEngine(args.model)
    server = ThreadingHTTPServer((args.host, args.port), make_handler(engine))
    print(f"PhotoKit worker ({engine.name} on {engine.device}) listening on http://{args.host}:{args.port}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
