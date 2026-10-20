# PhotoKit CLI

Install with `scripts/install-cli.sh` (macOS) or `scripts/install-cli.ps1` (Windows),
or run from the repo with `cargo run --release -p photokit-cli -- <command>`.

## Image tools

```bash
photokit resize photo.jpg --width 1200                 # -> photo-resized.jpg
photokit resize photo.jpg -W 1200 -H 800 --fit cover   # exact size, center crop
photokit resize *.jpg --percent 50 -o small/           # many files into a folder
photokit crop photo.jpg --aspect 16:9                  # center crop to a ratio
photokit crop photo.jpg --x 10 --y 20 --width 800 --height 600
photokit rotate photo.jpg -d 90                        # lossless quarter turns
photokit rotate photo.jpg -d 2.5 --straighten          # level a horizon
photokit flip photo.jpg [--vertical]
photokit convert photo.jpg -f webp -q 82               # -> photo.webp
photokit convert photo.jpg -o photo.avif               # format from extension
photokit convert big.jpg -f jpeg --target-size 300KB -W 2000
photokit optimize photo.jpg                            # re-encode, strip metadata
photokit info *.jpg [--json]
```

Output options shared by these commands:

| Option | Meaning |
|---|---|
| `-o, --output` | output file (single input) or folder |
| `-f, --format` | `jpeg`, `png`, `webp`, `avif`, `gif`, `bmp`, `tiff` |
| `-q, --quality` | 1–100 for JPEG/WebP/AVIF |
| `--target-size` | largest file size (`200KB`, `1.5MB`); quality is searched automatically |
| `--lossless` | lossless WebP |
| `--keep-metadata` | keep the ICC color profile (other metadata is always removed; EXIF orientation is applied to the pixels) |
| `--overwrite` | replace existing files; without it outputs get `-1`, `-2`… suffixes and an explicit existing `-o` file is an error |
| `--json`, `--quiet` | machine-readable output / no progress |

Originals are never modified unless you pass `--overwrite` with an output path
that points at them.

## Folders

```bash
photokit batch ./photos --preset web                         # -> ./photos/photokit-out
photokit batch ./photos -W 1600 -f webp -q 80 -o ./web
photokit batch ./photos --preset social --name "post-{n:2}" -r --dry-run
photokit presets
```

Files are processed in parallel on all CPU cores.

## Renaming

```bash
photokit rename ./photos -p "{parent}-{n:3}" --dry-run   # preview
photokit rename ./photos -p "{date}-{n:3}" --sort date --normalize-ext
photokit rename ./photos -p "{name}" --find "IMG_" --replace "trip_" --case lower
```

| Token | Value |
|---|---|
| `{name}` | original file name without extension |
| `{ext}` | original extension |
| `{parent}` | name of the containing folder |
| `{n}`, `{n:3}` | counter, optionally zero-padded (`--start`, `--step`) |
| `{w}`, `{h}` | pixel width / height |
| `{date}`, `{time}`, `{datetime}` | file modification time (UTC) |

Renames are planned first and applied in two phases, so swaps and collisions
inside the set are handled; names already used by other files get a numeric suffix.
The same tokens work for `batch --name`.

## Web app

```bash
photokit serve [--port 7878] [--open]
```

## AI

```bash
photokit ai backends                     # health + supported operations
photokit ai models [--backend comfyui]
photokit ai generate -p "retro computer on a desk" --width 1024 --height 1024 --seed 42
photokit ai img2img photo.jpg -p "watercolor painting" --strength 0.5
photokit ai inpaint photo.jpg --mask mask.png -p "empty wooden table"
photokit ai outpaint photo.jpg --right 256 --left 256 -p "beach at sunset"
photokit ai upscale image.png --scale 2
photokit ai remove-bg portrait.jpg -o cutout.png
photokit ai enhance "cat astronaut"      # rewrite a prompt with Ollama
photokit ai assist photo.jpg "make it 800px wide webp" [--apply]
```

Masks are white where the image should change. Inpainting results are blended
back into the full-resolution original, so unmasked areas keep their pixels.

## Configuration

```bash
photokit config show
photokit config path
photokit config set comfyui.url http://127.0.0.1:8188
photokit config set ai.default_backend worker
```

`PHOTOKIT_CONFIG_DIR` and `PHOTOKIT_DATA_DIR` override the config and data locations.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | failure |
| 2 | invalid arguments |
| 3 | some files failed (others succeeded) |
| 4 | output already exists (use `--overwrite`) |
| 5 | AI backend unreachable |
