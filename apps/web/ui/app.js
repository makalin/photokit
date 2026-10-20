// PhotoKit web UI. Vanilla JS, talks to the local Rust server.
"use strict";

const TOKEN = document.querySelector('meta[name="pk-token"]').content;
const DESKTOP = new URLSearchParams(location.search).has("desktop");
const $ = (s, root = document) => root.querySelector(s);
const $$ = (s, root = document) => [...root.querySelectorAll(s)];

// ---------------------------------------------------------------- helpers

async function api(path, { method, body, raw, headers = {} } = {}) {
  const opts = { method: method || (body !== undefined ? "POST" : "GET"), headers: { "X-PhotoKit-Token": TOKEN, ...headers } };
  if (body instanceof Blob || body instanceof ArrayBuffer) opts.body = body;
  else if (body !== undefined) { opts.body = JSON.stringify(body); opts.headers["Content-Type"] = "application/json"; }
  const res = await fetch("/api/" + path, opts);
  if (!res.ok) {
    let msg = res.statusText;
    try { msg = (await res.json()).error || msg; } catch { /* not JSON */ }
    throw new Error(msg);
  }
  if (raw) return res;
  return res.json();
}

const authUrl = (path) => `/api/${path}${path.includes("?") ? "&" : "?"}t=${TOKEN}`;

function toast(msg, isErr = false) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = "show" + (isErr ? " err" : "");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.className = ""), isErr ? 6000 : 2800);
}

function fmtBytes(n) {
  if (n == null) return "—";
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  return (n / 1024 / 1024).toFixed(2) + " MB";
}

function debounce(fn, ms) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}

const num = (el) => (el.value === "" ? null : Number(el.value));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function downloadBlob(blob, name) {
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  document.body.append(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 1000);
}

function filenameFrom(res, fallback) {
  const m = /filename="([^"]+)"/.exec(res.headers.get("Content-Disposition") || "");
  return m ? m[1] : fallback;
}

/** Poll a background job until it finishes. */
async function pollJob(id, onUpdate) {
  for (;;) {
    const info = await api("jobs/" + id);
    onUpdate?.(info);
    if (info.state !== "running") return info;
    await sleep(info.kind === "batch" ? 250 : 600);
  }
}

// ---------------------------------------------------------------- tabs

function showTab(name) {
  $$(".tabs button").forEach((b) => b.classList.toggle("active", b.dataset.tab === name));
  $$(".tab").forEach((t) => t.classList.toggle("active", t.id === "tab-" + name));
  if (name === "ai") refreshAi();
}
$$(".tabs button").forEach((b) => b.addEventListener("click", () => showTab(b.dataset.tab)));
$("#ai-chip").addEventListener("click", () => showTab("ai"));

// ---------------------------------------------------------------- file browser

const fb = {
  mode: "file", // file | folder | save
  path: null,
  selected: null,
  resolve: null,
};

function openBrowser({ mode = "file", title, start, name } = {}) {
  fb.mode = mode;
  fb.selected = null;
  $("#fb-title").textContent = title || (mode === "folder" ? "Choose folder" : mode === "save" ? "Save to folder" : "Open image");
  $("#fb-ok").textContent = mode === "file" ? "Open" : mode === "save" ? "Save here" : "Select folder";
  $("#fb-name").hidden = mode !== "save";
  $("#fb-name").value = name || "";
  $("#fb").showModal();
  loadDir(start || fb.path || localStorage.getItem("pk-last-dir") || "");
  return new Promise((resolve) => (fb.resolve = resolve));
}

async function loadDir(path) {
  const list = $("#fb-list");
  list.innerHTML = '<p class="muted small">Loading…</p>';
  try {
    const d = await api("fs/list?path=" + encodeURIComponent(path));
    fb.path = d.path;
    try { localStorage.setItem("pk-last-dir", d.path); } catch { /* storage off */ }
    $("#fb-path").value = d.path;
    $("#fb-up").disabled = !d.parent;
    $("#fb-up").dataset.parent = d.parent || "";
    $("#fb-roots").innerHTML = '<option value="">Places…</option>' + d.roots.map((r) => `<option>${esc(r)}</option>`).join("");
    list.innerHTML = "";
    for (const name of d.dirs) {
      const el = document.createElement("div");
      el.className = "fb-item";
      el.innerHTML = `<span class="ico">📁</span><span>${esc(name)}</span>`;
      el.addEventListener("dblclick", () => loadDir(joinPath(d.path, name, d.sep)));
      el.addEventListener("click", () => loadDir(joinPath(d.path, name, d.sep)));
      list.append(el);
    }
    for (const f of d.files) {
      const el = document.createElement("div");
      el.className = "fb-item";
      el.innerHTML = `<img loading="lazy" src="${authUrl("fs/thumb?size=96&path=" + encodeURIComponent(f.path))}" alt=""><span>${esc(f.name)}</span><span class="meta">${fmtBytes(f.size)}</span>`;
      if (fb.mode === "file") {
        el.addEventListener("click", () => {
          $$(".fb-item.sel", list).forEach((x) => x.classList.remove("sel"));
          el.classList.add("sel");
          fb.selected = f.path;
        });
        el.addEventListener("dblclick", () => finishBrowser(f.path));
      } else if (fb.mode === "save") {
        el.addEventListener("click", () => ($("#fb-name").value = f.name.replace(/\.[^.]+$/, "")));
      }
      list.append(el);
    }
    $("#fb-info").textContent = `${d.dirs.length} folders · ${d.files.length} images`;
    if (!d.dirs.length && !d.files.length) list.innerHTML = '<p class="muted small">Empty folder</p>';
  } catch (e) {
    list.innerHTML = `<p class="small" style="color:var(--err)">${esc(e.message)}</p>`;
  }
}

function finishBrowser(value) {
  $("#fb").close();
  fb.resolve?.(value);
  fb.resolve = null;
}

const joinPath = (dir, name, sep) => (dir.endsWith(sep) ? dir + name : dir + sep + name);
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);

$("#fb-up").addEventListener("click", (e) => { e.preventDefault(); if (e.currentTarget.dataset.parent) loadDir(e.currentTarget.dataset.parent); });
$("#fb-path").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); loadDir(e.target.value); } });
$("#fb-roots").addEventListener("change", (e) => e.target.value && loadDir(e.target.value));
$("#fb-cancel").addEventListener("click", () => finishBrowser(null));
$("#fb").addEventListener("close", () => { if (fb.resolve) { fb.resolve(null); fb.resolve = null; } });
$("#fb-ok").addEventListener("click", () => {
  if (fb.mode === "file") return fb.selected ? finishBrowser(fb.selected) : toast("Select an image first");
  if (fb.mode === "save") return finishBrowser({ dir: fb.path, name: $("#fb-name").value.trim() || null });
  finishBrowser(fb.path);
});

// ---------------------------------------------------------------- Quick Tools

const Q = {
  src: null, // source info from the server
  ops: [], // edit ops (rotate/flip/crop/straighten/grayscale), resize is separate
  redo: [],
  view: "edit",
  cropping: false,
  crop: null, // {x,y,w,h} in displayed pixels
  lastOutput: null,
};

function editDims() {
  if (!Q.src) return [0, 0];
  let [w, h] = [Q.src.width, Q.src.height];
  for (const op of Q.ops) {
    const near = (a, b) => Math.abs(a - b) < 0.01;
    if (op.op === "rotate") {
      const d = ((op.degrees % 360) + 360) % 360;
      if (near(d, 90) || near(d, 270)) [w, h] = [h, w];
      else if (!(near(d, 0) || near(d, 180) || near(d, 360))) {
        const t = (op.degrees * Math.PI) / 180, s = Math.abs(Math.sin(t)), c = Math.abs(Math.cos(t));
        [w, h] = [Math.floor(w * c + h * s), Math.floor(w * s + h * c)];
      }
    } else if (op.op === "straighten") {
      const t = (op.degrees * Math.PI) / 180, s = Math.abs(Math.sin(t)), c = Math.abs(Math.cos(t));
      const k = Math.min(w / (w * c + h * s), h / (w * s + h * c));
      [w, h] = [Math.max(1, Math.floor(w * k)), Math.max(1, Math.floor(h * k))];
    } else if (op.op === "crop") {
      [w, h] = [Math.min(op.width, w - op.x), Math.min(op.height, h - op.y)];
    }
  }
  return [w, h];
}

function resizeOp() {
  const width = num($("#q-w")), height = num($("#q-h")), percent = num($("#q-pct"));
  if (width == null && height == null && percent == null) return null;
  return { op: "resize", width, height, percent, fit: $("#q-fit").value, upscale: $("#q-upscale").checked };
}

function encodeOpts() {
  return {
    format: $("#q-format").value,
    quality: Number($("#q-q").value),
    lossless: $("#q-lossless").checked,
    keep_icc: $("#q-icc").checked,
    png_level: 2,
  };
}

function allOps() {
  const r = resizeOp();
  return r ? [...Q.ops, r] : [...Q.ops];
}

function processBody(action) {
  return { source: Q.src.id, ops: allOps(), encode: encodeOpts(), target_kb: num($("#q-target")), action };
}

async function loadSource(info) {
  Q.src = info;
  Q.ops = [];
  Q.redo = [];
  $("#q-straighten").value = 0;
  $("#q-straighten-v").textContent = "0°";
  $("#q-drop").hidden = true;
  $("#q-canvas").hidden = false;
  $("#q-s-orig").textContent = `${info.width}×${info.height} · ${fmtBytes(info.bytes)}`;
  if (info.format && !$("#q-preset").value && ["jpeg", "png", "webp"].includes(info.format) && !Q.formatTouched) {
    $("#q-format").value = info.format;
  }
  $("#q-save-msg").textContent = info.path ? "Saves next to the original" : "";
  setView("edit");
  await renderEdit();
  estimate();
}

async function openPath(path) {
  try {
    await loadSource(await api("source/open", { body: { path } }));
  } catch (e) { toast(e.message, true); }
}

async function uploadFile(file) {
  try {
    const info = await api("source/upload", { body: file, headers: { "X-Filename": encodeURIComponent(file.name || "pasted.png") } });
    await loadSource(info);
  } catch (e) { toast(e.message, true); }
}

let renderSeq = 0;
async function renderEdit() {
  if (!Q.src) return;
  const seq = ++renderSeq;
  $("#q-stage").classList.add("busy");
  try {
    let res;
    if (Q.view === "original") res = await api(`source/${Q.src.id}?max=2400`, { raw: true });
    else if (Q.view === "output") res = await api("process", { body: { ...processBody("preview"), max: 1 }, raw: true });
    else res = await api("render", { body: { source: Q.src.id, ops: Q.ops, max: 1600 }, raw: true });
    const blob = await res.blob();
    if (seq !== renderSeq) return;
    const img = $("#q-img");
    if (img.src.startsWith("blob:")) URL.revokeObjectURL(img.src);
    img.src = URL.createObjectURL(blob);
    if (Q.view === "output") showOutputStats(res);
  } catch (e) {
    if (seq === renderSeq) toast(e.message, true);
  } finally {
    if (seq === renderSeq) $("#q-stage").classList.remove("busy");
  }
}

function showOutputStats(res) {
  const bytes = Number(res.headers.get("X-Bytes"));
  const w = res.headers.get("X-Width"), h = res.headers.get("X-Height"), q = res.headers.get("X-Quality");
  const fmt = $("#q-format").value;
  const target = num($("#q-target"));
  $("#q-s-out").textContent = `${w}×${h} · ${fmtBytes(bytes)}` + (target && q ? ` · q${q}` : "");
  const saved = Q.src.bytes ? 1 - bytes / Q.src.bytes : 0;
  const el = $("#q-s-save");
  el.textContent = (saved >= 0 ? "−" : "+") + Math.abs(saved * 100).toFixed(0) + "%";
  el.className = saved >= 0 ? "good" : "bad";
  Q.lastOutput = { bytes, w, h, fmt };
}

let estSeq = 0;
const estimate = debounce(async () => {
  if (!Q.src) return;
  if (Q.view === "output") return renderEdit();
  const seq = ++estSeq;
  $("#q-s-out").textContent = "estimating…";
  try {
    const res = await api("process", { body: processBody("preview"), raw: true });
    if (seq === estSeq) showOutputStats(res);
  } catch (e) {
    if (seq === estSeq) { $("#q-s-out").textContent = "—"; toast(e.message, true); }
  }
}, 350);

function pushOp(op) {
  if (!Q.src) return toast("Open an image first");
  Q.ops.push(op);
  Q.redo = [];
  afterEdit();
}

function afterEdit() {
  const st = Q.ops.findLast?.((o) => o.op === "straighten");
  $("#q-straighten").value = st && Q.ops[Q.ops.length - 1] === st ? st.degrees : 0;
  $("#q-straighten-v").textContent = $("#q-straighten").value + "°";
  if (Q.view !== "edit") setView("edit");
  renderEdit();
  estimate();
}

$$("[data-op]").forEach((b) =>
  b.addEventListener("click", () => {
    const map = {
      rotl: { op: "rotate", degrees: 270 },
      rotr: { op: "rotate", degrees: 90 },
      fliph: { op: "flip", axis: "horizontal" },
      flipv: { op: "flip", axis: "vertical" },
      gray: { op: "grayscale" },
    };
    pushOp(map[b.dataset.op]);
  })
);

$("#q-straighten").addEventListener("input", (e) => {
  $("#q-straighten-v").textContent = e.target.value + "°";
});
$("#q-straighten").addEventListener("change", (e) => {
  if (!Q.src) return;
  const deg = Number(e.target.value);
  const last = Q.ops[Q.ops.length - 1];
  if (last?.op === "straighten") {
    if (deg === 0) Q.ops.pop();
    else last.degrees = deg;
  } else if (deg !== 0) Q.ops.push({ op: "straighten", degrees: deg });
  Q.redo = [];
  renderEdit();
  estimate();
});

$("#q-undo").addEventListener("click", undo);
$("#q-redo").addEventListener("click", redo);
$("#q-reset").addEventListener("click", () => { if (Q.ops.length) { Q.redo = []; Q.ops = []; afterEdit(); } });
function undo() { if (Q.ops.length) { Q.redo.push(Q.ops.pop()); afterEdit(); } }
function redo() { if (Q.redo.length) { Q.ops.push(Q.redo.pop()); afterEdit(); } }

function setView(v) {
  Q.view = v;
  $$("#q-view button").forEach((b) => b.classList.toggle("active", b.dataset.view === v));
  if (v !== "edit" && Q.cropping) endCrop();
}
$$("#q-view button").forEach((b) => b.addEventListener("click", () => { setView(b.dataset.view); renderEdit(); }));

// crop
$("#q-crop").addEventListener("click", () => (Q.cropping ? endCrop() : startCrop()));
function startCrop() {
  if (!Q.src) return toast("Open an image first");
  if (Q.view !== "edit") { setView("edit"); renderEdit(); }
  Q.cropping = true;
  Q.crop = null;
  $("#q-crop").classList.add("active");
  $("#q-canvas").classList.add("cropping");
  $("#q-crop-actions").hidden = false;
  toast("Drag on the image to select the crop area");
}
function endCrop() {
  Q.cropping = false;
  $("#q-crop").classList.remove("active");
  $("#q-canvas").classList.remove("cropping");
  $("#q-crop-actions").hidden = true;
  $("#q-cropbox").hidden = true;
}
{
  const wrap = $("#q-canvas");
  let start = null;
  wrap.addEventListener("pointerdown", (e) => {
    if (!Q.cropping) return;
    const r = $("#q-img").getBoundingClientRect();
    start = { x: e.clientX - r.left, y: e.clientY - r.top, r };
    wrap.setPointerCapture(e.pointerId);
  });
  wrap.addEventListener("pointermove", (e) => {
    if (!start) return;
    const { r } = start;
    let x2 = Math.max(0, Math.min(r.width, e.clientX - r.left));
    let y2 = Math.max(0, Math.min(r.height, e.clientY - r.top));
    const ratio = Number($("#q-crop-ratio").value);
    if (ratio) {
      const dw = x2 - start.x, dh = y2 - start.y;
      const h = Math.abs(dw) / ratio;
      y2 = start.y + Math.sign(dh || 1) * h;
      if (y2 < 0 || y2 > r.height) {
        y2 = Math.max(0, Math.min(r.height, y2));
        x2 = start.x + Math.sign(dw || 1) * Math.abs(y2 - start.y) * ratio;
      }
    }
    Q.crop = { x: Math.min(start.x, x2), y: Math.min(start.y, y2), w: Math.abs(x2 - start.x), h: Math.abs(y2 - start.y), dw: r.width, dh: r.height };
    const box = $("#q-cropbox");
    const off = $("#q-img").offsetLeft, offT = $("#q-img").offsetTop;
    Object.assign(box.style, { left: off + Q.crop.x + "px", top: offT + Q.crop.y + "px", width: Q.crop.w + "px", height: Q.crop.h + "px" });
    box.hidden = false;
  });
  wrap.addEventListener("pointerup", () => (start = null));
}
$("#q-crop-cancel").addEventListener("click", endCrop);
$("#q-crop-apply").addEventListener("click", () => {
  const c = Q.crop;
  if (!c || c.w < 4 || c.h < 4) return toast("Drag a larger area");
  const [W, H] = editDims();
  const sx = W / c.dw, sy = H / c.dh;
  const op = { op: "crop", x: Math.round(c.x * sx), y: Math.round(c.y * sy), width: Math.max(1, Math.round(c.w * sx)), height: Math.max(1, Math.round(c.h * sy)) };
  endCrop();
  pushOp(op);
});

// resize inputs keep aspect ratio
$("#q-w").addEventListener("input", () => linkDims("w"));
$("#q-h").addEventListener("input", () => linkDims("h"));
function linkDims(changed) {
  if (!$("#q-lock").classList.contains("active") || !Q.src || $("#q-fit").value !== "contain") return estimate();
  const [W, H] = editDims();
  if (changed === "w" && num($("#q-w"))) $("#q-h").value = "";
  if (changed === "h" && num($("#q-h"))) $("#q-w").value = "";
  $("#q-w").placeholder = changed === "h" && num($("#q-h")) ? Math.round((W * num($("#q-h"))) / H) : "Auto";
  $("#q-h").placeholder = changed === "w" && num($("#q-w")) ? Math.round((H * num($("#q-w"))) / W) : "Auto";
  $("#q-pct").value = "";
  estimate();
}
$("#q-lock").addEventListener("click", (e) => e.currentTarget.classList.toggle("active"));
$("#q-q").addEventListener("input", (e) => ($("#q-q-v").textContent = e.target.value));
$("#q-format").addEventListener("change", () => (Q.formatTouched = true));
["#q-pct", "#q-fit", "#q-upscale", "#q-format", "#q-q", "#q-target", "#q-lossless", "#q-icc"].forEach((s) =>
  $(s).addEventListener(s === "#q-q" || s === "#q-pct" || s === "#q-target" ? "input" : "change", () => { $("#q-preset").value = ""; estimate(); })
);

$("#q-preset").addEventListener("change", (e) => {
  const p = PRESETS.find((x) => x.name === e.target.value);
  if (!p) return;
  applyPresetTo("q", p);
  estimate();
});

function applyPresetTo(prefix, p) {
  const r = p.resize || {};
  $(`#${prefix}-w`).value = r.width ?? "";
  $(`#${prefix}-h`).value = r.height ?? "";
  $(`#${prefix}-fit`).value = r.fit || "contain";
  if (prefix === "q") {
    $("#q-pct").value = r.percent ?? "";
    $("#q-upscale").checked = !!r.upscale;
    if (p.output.format) $("#q-format").value = p.output.format;
    $("#q-q").value = p.output.quality ?? 85;
    $("#q-q-v").textContent = $("#q-q").value;
    $("#q-target").value = p.output.target_kb ?? "";
    $("#q-lossless").checked = !!p.output.lossless;
    $("#q-icc").checked = !!p.output.keep_metadata;
  } else {
    $("#b-format").value = p.output.format || "";
    $("#b-q").value = p.output.quality ?? 85;
    $("#b-target").value = p.output.target_kb ?? "";
    $("#b-icc").checked = !!p.output.keep_metadata;
    if (p.name_pattern) $("#b-name").value = p.name_pattern;
  }
}

// open / drop / paste
$("#q-open").addEventListener("click", async () => { const p = await openBrowser({ mode: "file" }); if (p) openPath(p); });
$("#q-drop-open").addEventListener("click", async (e) => { e.preventDefault(); const p = await openBrowser({ mode: "file" }); if (p) openPath(p); });
$("#q-upload").addEventListener("click", () => $("#q-file").click());
$("#q-file").addEventListener("change", (e) => { if (e.target.files[0]) uploadFile(e.target.files[0]); e.target.value = ""; });
{
  const stage = $("#q-stage");
  stage.addEventListener("dragover", (e) => { e.preventDefault(); stage.classList.add("dragover"); });
  stage.addEventListener("dragleave", () => stage.classList.remove("dragover"));
  stage.addEventListener("drop", (e) => {
    e.preventDefault();
    stage.classList.remove("dragover");
    const f = [...e.dataTransfer.files].find((x) => x.type.startsWith("image/") || /\.(jpe?g|png|webp|gif|bmp|tiff?)$/i.test(x.name));
    if (f) uploadFile(f);
  });
}
document.addEventListener("paste", (e) => {
  if (e.target.matches("input, textarea")) return;
  const item = [...(e.clipboardData?.items || [])].find((i) => i.type.startsWith("image/"));
  if (item) { showTab("quick"); uploadFile(item.getAsFile()); }
});
document.addEventListener("keydown", (e) => {
  if (e.target.matches("input, textarea, select") || !$("#tab-quick").classList.contains("active")) return;
  const mod = e.metaKey || e.ctrlKey;
  if (mod && e.key.toLowerCase() === "z") { e.preventDefault(); e.shiftKey ? redo() : undo(); }
  if (mod && e.key.toLowerCase() === "s") { e.preventDefault(); $("#q-save").click(); }
  if (e.key === "Escape" && Q.cropping) endCrop();
});

// export
$("#q-download").addEventListener("click", async () => {
  if (!Q.src) return toast("Open an image first");
  try {
    const res = await api("process", { body: processBody("download"), raw: true });
    downloadBlob(await res.blob(), filenameFrom(res, "photokit-image"));
  } catch (e) { toast(e.message, true); }
});
$("#q-save").addEventListener("click", async () => {
  if (!Q.src) return toast("Open an image first");
  let target = {};
  if (!Q.src.path || DESKTOP) {
    const dir = Q.src.path ? Q.src.path.replace(/[\\/][^\\/]*$/, "") : undefined;
    const stem = Q.src.name.replace(/\.[^.]+$/, "") + "-edited";
    const pick = await openBrowser({ mode: "save", start: dir, name: stem });
    if (!pick) return;
    target = { output_dir: pick.dir, name: pick.name };
  }
  try {
    const r = await api("process", { body: { ...processBody("save"), ...target } });
    $("#q-save-msg").textContent = "Saved " + r.path;
    toast(`Saved ${fmtBytes(r.bytes)} → ${r.path}`);
  } catch (e) { toast(e.message, true); }
});
$("#q-to-ai").addEventListener("click", () => {
  if (!Q.src) return toast("Open an image first");
  setAiSource(Q.src);
  showTab("ai");
});

// assistant
$("#q-assist-go").addEventListener("click", async () => {
  const text = $("#q-assist").value.trim();
  if (!text) return;
  $("#q-assist-msg").textContent = "Thinking…";
  try {
    const plan = await api("ai/assist", { body: { text } });
    for (const op of plan.ops) {
      if (op.op === "resize") {
        $("#q-w").value = op.width ?? "";
        $("#q-h").value = op.height ?? "";
        $("#q-pct").value = op.percent ?? "";
        $("#q-fit").value = op.fit || "contain";
      } else if (Q.src) Q.ops.push(op);
    }
    if (plan.format) $("#q-format").value = plan.format;
    if (plan.quality) { $("#q-q").value = plan.quality; $("#q-q-v").textContent = plan.quality; }
    $("#q-assist-msg").textContent = (plan.summary || "Applied.") + " Review the preview, then save.";
    if (Q.src) afterEdit();
  } catch (e) {
    $("#q-assist-msg").textContent = e.message;
  }
});

// ---------------------------------------------------------------- Batch & Rename

const B = { dir: null, files: [], off: new Set(), job: null };

async function loadBatchDir(dir) {
  if (!dir) return;
  try {
    const recursive = $("#b-rec").checked;
    let files;
    if (recursive) {
      // the plan endpoint walks subfolders for us
      const r = await api("rename/preview", { body: { dir, recursive: true, options: { pattern: "{name}" } } });
      files = r.items.map((i) => ({ path: i.from, name: i.from.split(/[\\/]/).pop() }));
    } else {
      files = (await api("fs/list?path=" + encodeURIComponent(dir))).files;
    }
    B.dir = dir;
    B.files = files;
    B.off.clear();
    $("#b-dir").value = dir;
    try { localStorage.setItem("pk-batch-dir", dir); } catch { /* storage off */ }
    renderGrid();
    setBatchView("grid");
  } catch (e) { toast(e.message, true); }
}

function selectedFiles() {
  return B.files.filter((f) => !B.off.has(f.path)).map((f) => f.path);
}

function renderGrid() {
  const g = $("#b-grid");
  g.innerHTML = "";
  if (!B.files.length) { g.innerHTML = '<p class="empty">No supported images in this folder.</p>'; }
  for (const f of B.files) {
    const t = document.createElement("div");
    t.className = "tile" + (B.off.has(f.path) ? " off" : "");
    t.innerHTML = `<input type="checkbox" ${B.off.has(f.path) ? "" : "checked"}><img loading="lazy" alt="" src="${authUrl("fs/thumb?size=256&path=" + encodeURIComponent(f.path))}"><div class="name" title="${esc(f.name)}">${esc(f.name)}</div>${f.size ? `<div class="size">${fmtBytes(f.size)}</div>` : ""}`;
    const box = $("input", t);
    const toggle = (on) => { on ? B.off.delete(f.path) : B.off.add(f.path); t.classList.toggle("off", !on); box.checked = on; updateCount(); };
    box.addEventListener("click", (e) => { e.stopPropagation(); toggle(box.checked); });
    t.addEventListener("click", () => toggle(B.off.has(f.path)));
    t.addEventListener("dblclick", () => { showTab("quick"); openPath(f.path); });
    g.append(t);
  }
  updateCount();
}

function updateCount() {
  const n = selectedFiles().length;
  $("#b-count").textContent = B.files.length ? `${n} of ${B.files.length} selected · double-click to edit` : "No folder loaded";
  $("#b-all").checked = n === B.files.length;
}

$("#b-all").addEventListener("change", (e) => {
  B.off = e.target.checked ? new Set() : new Set(B.files.map((f) => f.path));
  renderGrid();
});
$("#b-pick").addEventListener("click", async () => { const d = await openBrowser({ mode: "folder", start: B.dir }); if (d) loadBatchDir(d); });
$("#b-load").addEventListener("click", () => loadBatchDir($("#b-dir").value.trim()));
$("#b-dir").addEventListener("keydown", (e) => e.key === "Enter" && loadBatchDir(e.target.value.trim()));
$("#b-rec").addEventListener("change", () => B.dir && loadBatchDir(B.dir));

function setBatchView(v) {
  $$("#b-view button").forEach((b) => b.classList.toggle("active", b.dataset.view === v));
  $("#b-grid").hidden = v !== "grid";
  $("#b-plan").hidden = v !== "plan";
}
$$("#b-view button").forEach((b) => b.addEventListener("click", () => setBatchView(b.dataset.view)));

function showPlan(items, sameNote = "unchanged") {
  const tb = $("#b-plan tbody");
  tb.innerHTML = "";
  const base = (p) => String(p).split(/[\\/]/).pop();
  for (const it of items) {
    const tr = document.createElement("tr");
    const same = it.changed === false;
    tr.className = (same ? "same" : "") + (it.conflict ? " conflict" : "");
    tr.innerHTML = `<td>${esc(base(it.from))}</td><td class="arrow">→</td><td title="${esc(it.to)}">${esc(same ? sameNote : it.to.startsWith?.(B.dir) && !$("#b-out").value ? base(it.to) : it.to)}${it.conflict ? " (name taken, numbered)" : ""}</td>`;
    tb.append(tr);
  }
  setBatchView("plan");
}

function renameBody() {
  return {
    files: selectedFiles(),
    options: {
      pattern: $("#r-pattern").value,
      start: Number($("#r-start").value || 1),
      step: Number($("#r-step").value || 1),
      sort: $("#r-sort").value,
      case: $("#r-case").value,
      normalize_ext: $("#r-ext").checked,
      find: $("#r-find").value || null,
      replace: $("#r-replace").value,
    },
  };
}

$$(".tokens code").forEach((c) => c.addEventListener("click", () => {
  const i = $("#r-pattern");
  i.setRangeText(c.textContent, i.selectionStart, i.selectionEnd, "end");
  i.focus();
}));

$("#r-preview").addEventListener("click", async () => {
  if (!selectedFiles().length) return toast("Choose a folder with images first");
  try { showPlan((await api("rename/preview", { body: renameBody() })).items); } catch (e) { toast(e.message, true); }
});
$("#r-apply").addEventListener("click", async () => {
  const files = selectedFiles();
  if (!files.length) return toast("Choose a folder with images first");
  if (!confirm(`Rename ${files.length} files in place?`)) return;
  try {
    const r = await api("rename/apply", { body: renameBody() });
    toast(`Renamed ${r.renamed} files`);
    showPlan(r.items);
    const keep = B.dir;
    await loadBatchDir(keep);
    setBatchView("plan");
  } catch (e) { toast(e.message, true); }
});

function batchSpec(dryRun = false) {
  const width = num($("#b-w")), height = num($("#b-h"));
  const ops = width || height ? [{ op: "resize", width, height, fit: $("#b-fit").value, upscale: false }] : [];
  const sep = B.dir && B.dir.includes("\\") ? "\\" : "/";
  const out = $("#b-out").value.trim() || (B.dir ? B.dir.replace(/[\\/]$/, "") + sep + "photokit-out" : null);
  const target = num($("#b-target"));
  return {
    ops,
    format: $("#b-format").value || null,
    encode: { quality: Number($("#b-q").value || 85), keep_icc: $("#b-icc").checked, png_level: 2 },
    target_bytes: target ? target * 1024 : null,
    output_dir: out,
    name_pattern: $("#b-name").value || "{name}",
    overwrite: $("#b-overwrite").checked,
    dry_run: dryRun,
  };
}

$("#b-preset").addEventListener("change", (e) => {
  const p = PRESETS.find((x) => x.name === e.target.value);
  if (p) applyPresetTo("b", p);
});
$("#b-plan-btn").addEventListener("click", async () => {
  if (!selectedFiles().length) return toast("Choose a folder with images first");
  try { showPlan((await api("batch/plan", { body: { files: selectedFiles(), spec: batchSpec(true) } })).items); } catch (e) { toast(e.message, true); }
});
$("#b-run").addEventListener("click", async () => {
  const files = selectedFiles();
  if (!files.length) return toast("Choose a folder with images first");
  const spec = batchSpec();
  if (spec.overwrite && !confirm("Overwrite existing files with the same names?")) return;
  try {
    const { job } = await api("batch/run", { body: { files, spec } });
    B.job = job;
    $("#b-progress").hidden = false;
    $("#b-cancel").hidden = false;
    $("#b-log").innerHTML = "";
    let shown = 0;
    const info = await pollJob(job, (i) => {
      $("#b-bar").style.width = ((i.done / Math.max(1, i.total)) * 100).toFixed(1) + "%";
      $("#b-msg").textContent = `${i.done} / ${i.total}`;
      for (const it of i.recent.slice(shown)) logItem(it);
      shown = i.recent.length;
    });
    $("#b-cancel").hidden = true;
    const r = info.result || {};
    if (info.state === "failed") throw new Error(info.message);
    const saved = r.input_bytes ? (1 - r.output_bytes / r.input_bytes) * 100 : 0;
    $("#b-msg").textContent = `${info.state === "cancelled" ? "Cancelled. " : ""}${r.succeeded ?? 0} done, ${r.failed ?? 0} failed in ${((r.elapsed_ms ?? 0) / 1000).toFixed(1)} s · ${fmtBytes(r.input_bytes)} → ${fmtBytes(r.output_bytes)} (${saved >= 0 ? "−" : "+"}${Math.abs(saved).toFixed(0)}%) · ${spec.output_dir}`;
    toast(`Processed ${r.succeeded ?? 0} images`);
  } catch (e) { toast(e.message, true); }
});
$("#b-cancel").addEventListener("click", () => B.job && api(`jobs/${B.job}/cancel`, { body: {} }));

function logItem(it) {
  const li = document.createElement("li");
  const name = String(it.input).split(/[\\/]/).pop();
  if (it.error) { li.className = "err"; li.textContent = `✕ ${name}: ${it.error}`; }
  else if (it.skipped) li.textContent = `– ${name} skipped`;
  else li.textContent = `✓ ${name} → ${String(it.output).split(/[\\/]/).pop()} ${it.width}×${it.height} ${fmtBytes(it.output_bytes)}`;
  $("#b-log").prepend(li);
}

// ---------------------------------------------------------------- AI Studio

const A = { status: null, source: null, job: null, results: [], current: null, maskDirty: false };

async function refreshAi() {
  try {
    const st = await api("ai/status");
    A.status = st;
    const box = $("#a-backends");
    box.innerHTML = "";
    const sel = $("#a-backend");
    const prev = sel.value;
    sel.innerHTML = "";
    for (const b of st.backends) {
      const d = document.createElement("div");
      d.className = "backend" + (b.health.ok ? " ok" : "");
      const ops = b.capabilities?.operations || [];
      d.innerHTML = `<b>${esc(b.name)}</b><p>${esc(b.health.message)}</p>` +
        (ops.length ? `<div class="ops">${ops.map((o) => `<span>${o.replace(/_/g, " ")}</span>`).join("")}</div>` : "") +
        (b.capabilities?.notes?.length ? `<p>${b.capabilities.notes.map(esc).join("<br>")}</p>` : "");
      box.append(d);
      sel.insertAdjacentHTML("beforeend", `<option value="${b.id}">${esc(b.name)}${b.health.ok ? "" : " (offline)"}</option>`);
    }
    if (st.ollama) {
      const d = document.createElement("div");
      d.className = "backend" + (st.ollama.ok ? " ok" : "");
      d.innerHTML = `<b>Ollama <span class="muted small">(prompts &amp; assistant)</span></b><p>${esc(st.ollama.message)}</p>`;
      box.append(d);
    }
    const firstOk = st.backends.find((b) => b.health.ok);
    sel.value = prev && st.backends.some((b) => b.id === prev) ? prev : (firstOk || st.backends[0])?.id;
    const chip = $("#ai-chip");
    chip.textContent = firstOk ? `AI: ${firstOk.name}` : "AI: offline";
    chip.className = "status " + (firstOk ? "ok" : "off");
    updateAiForm();
  } catch (e) { toast(e.message, true); }
  loadGallery();
}

function currentBackend() {
  return A.status?.backends.find((b) => b.id === $("#a-backend").value);
}

function updateAiForm() {
  const b = currentBackend();
  const ops = b?.capabilities?.operations || [];
  for (const o of $("#a-mode").options) {
    o.disabled = b?.health.ok ? !ops.includes(o.value) : false;
    o.textContent = o.textContent.replace(/ — unavailable$/, "") + (o.disabled ? " — unavailable" : "");
  }
  const models = $("#a-model");
  const prev = models.value;
  const mode = $("#a-mode").value;
  const kind = mode === "upscale" ? "upscale" : "checkpoint";
  models.innerHTML = '<option value="">Default</option>' + (b?.models || []).filter((m) => m.kind === kind).map((m) => `<option>${esc(m.name)}</option>`).join("");
  if ([...models.options].some((o) => o.value === prev)) models.value = prev;
  $$("[data-for]").forEach((el) => (el.hidden = el.dataset.for !== mode));
  const needsSrc = mode !== "text_to_image";
  $("#a-source-card").style.opacity = needsSrc ? 1 : 0.5;
  $("#a-mask-tools").hidden = mode !== "inpaint" || !A.source;
  $("#a-mask").hidden = mode !== "inpaint" || !A.source;
  $("#a-prompt").disabled = mode === "upscale" || mode === "remove_background";
  $("#a-go").textContent = { upscale: "Upscale", remove_background: "Remove background" }[mode] || "Generate";
}
$("#a-backend").addEventListener("change", updateAiForm);
$("#a-mode").addEventListener("change", updateAiForm);
$("#a-refresh").addEventListener("click", refreshAi);

function setAiSource(info) {
  A.source = info;
  const img = $("#a-src-img");
  img.onload = () => { sizeMask(); updateAiForm(); };
  img.src = authUrl(`source/${info.id}?max=1600`);
  img.hidden = false;
  $("#a-src-empty").hidden = true;
  if ($("#a-mode").value === "text_to_image") $("#a-mode").value = "image_to_image";
  updateAiForm();
}

$("#a-src-open").addEventListener("click", async () => {
  const p = await openBrowser({ mode: "file" });
  if (!p) return;
  try { setAiSource(await api("source/open", { body: { path: p } })); } catch (e) { toast(e.message, true); }
});
$("#a-src-quick").addEventListener("click", () => (Q.src ? setAiSource(Q.src) : toast("Open an image in Quick Tools first")));

// mask painting
function sizeMask() {
  const img = $("#a-src-img"), c = $("#a-mask");
  c.width = A.source?.width || img.naturalWidth;
  c.height = A.source?.height || img.naturalHeight;
  c.style.width = img.clientWidth + "px";
  c.style.height = img.clientHeight + "px";
  c.style.left = img.offsetLeft + "px";
  c.style.top = img.offsetTop + "px";
  const ctx = c.getContext("2d");
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, c.width, c.height);
  A.maskDirty = false;
}
window.addEventListener("resize", () => {
  const img = $("#a-src-img"), c = $("#a-mask");
  if (img.hidden) return;
  Object.assign(c.style, { width: img.clientWidth + "px", height: img.clientHeight + "px", left: img.offsetLeft + "px", top: img.offsetTop + "px" });
});
{
  const c = $("#a-mask");
  let drawing = false, last = null;
  const pos = (e) => {
    const r = c.getBoundingClientRect();
    return { x: ((e.clientX - r.left) / r.width) * c.width, y: ((e.clientY - r.top) / r.height) * c.height, k: c.width / r.width };
  };
  const paint = (e) => {
    const p = pos(e), ctx = c.getContext("2d");
    ctx.strokeStyle = ctx.fillStyle = "#fff";
    ctx.lineCap = "round";
    ctx.lineWidth = Number($("#a-brush").value) * p.k;
    ctx.beginPath();
    ctx.moveTo(last ? last.x : p.x, last ? last.y : p.y);
    ctx.lineTo(p.x, p.y);
    ctx.stroke();
    last = p;
    A.maskDirty = true;
  };
  c.addEventListener("pointerdown", (e) => { drawing = true; last = null; c.setPointerCapture(e.pointerId); paint(e); });
  c.addEventListener("pointermove", (e) => drawing && paint(e));
  c.addEventListener("pointerup", () => { drawing = false; last = null; });
}
$("#a-mask-clear").addEventListener("click", sizeMask);
$("#a-mask-invert").addEventListener("click", () => {
  const c = $("#a-mask"), ctx = c.getContext("2d");
  const d = ctx.getImageData(0, 0, c.width, c.height);
  for (let i = 0; i < d.data.length; i += 4) { d.data[i] = d.data[i + 1] = d.data[i + 2] = 255 - d.data[i]; }
  ctx.putImageData(d, 0, 0);
  A.maskDirty = true;
});

$("#a-enhance").addEventListener("click", async () => {
  const p = $("#a-prompt").value.trim();
  if (!p) return toast("Write a short idea first");
  const btn = $("#a-enhance");
  btn.disabled = true;
  btn.textContent = "✨ Enhancing…";
  try { $("#a-prompt").value = (await api("ai/enhance", { body: { prompt: p } })).prompt; } catch (e) { toast(e.message, true); }
  btn.disabled = false;
  btn.textContent = "✨ Enhance prompt";
});

function aiRequest() {
  const [width, height] = $("#a-size").value.split("x").map(Number);
  const seed = $("#a-seed").value.trim();
  return {
    operation: $("#a-mode").value,
    prompt: $("#a-prompt").value.trim(),
    negative_prompt: $("#a-neg").value,
    model: $("#a-model").value || null,
    width, height,
    steps: Number($("#a-steps").value || 24),
    cfg: Number($("#a-cfg").value || 7),
    seed: seed ? Number(seed) : null,
    denoise: Number($("#a-denoise").value || 0.6),
    batch: Number($("#a-batch").value || 1),
    pad: [num($("#a-pl")) || 0, num($("#a-pt")) || 0, num($("#a-pr")) || 0, num($("#a-pb")) || 0],
    scale: Number($("#a-scale").value || 2),
  };
}

$("#a-go").addEventListener("click", async () => {
  const request = aiRequest();
  const body = { backend: $("#a-backend").value, request };
  if (request.operation !== "text_to_image") {
    if (!A.source) return toast("Choose a source image first", true);
    body.source = A.source.id;
  }
  if (request.operation === "inpaint") {
    if (!A.maskDirty) return toast("Paint over the area to change first", true);
    body.mask = $("#a-mask").toDataURL("image/png");
  }
  try {
    const { job } = await api("ai/generate", { body });
    A.job = job;
    $("#a-go").disabled = true;
    $("#a-cancel").hidden = false;
    $("#a-progress").hidden = false;
    const started = Date.now();
    const info = await pollJob(job, (i) => {
      const bar = $("#a-progress");
      bar.classList.toggle("indeterminate", i.fraction == null);
      $("#a-bar").style.width = i.fraction != null ? (i.fraction * 100).toFixed(0) + "%" : "";
      $("#a-msg").textContent = `${i.message || "starting"} · ${((Date.now() - started) / 1000).toFixed(0)} s`;
    });
    if (info.state === "done") {
      showResults(info.result.images);
      $("#a-msg").textContent = `Done in ${((Date.now() - started) / 1000).toFixed(1)} s`;
      loadGallery();
    } else {
      $("#a-msg").textContent = info.state === "cancelled" ? "Cancelled" : "";
      if (info.state === "failed") toast(info.message, true);
    }
  } catch (e) {
    toast(e.message, true);
  } finally {
    $("#a-go").disabled = false;
    $("#a-cancel").hidden = true;
    $("#a-progress").hidden = true;
  }
});
$("#a-cancel").addEventListener("click", () => A.job && api(`jobs/${A.job}/cancel`, { body: {} }));

function showResults(items) {
  A.results = items;
  const view = $("#a-result");
  view.innerHTML = "";
  items.forEach((it) => {
    const img = document.createElement("img");
    img.src = authUrl(`gallery/${it.id}/image`);
    if (items.length > 1) { img.className = "thumb"; img.addEventListener("click", () => showResults([it])); }
    view.append(img);
  });
  A.current = items[0];
  $("#a-result-actions").hidden = !items.length;
  const it = items[0];
  $("#a-result-meta").textContent = it ? `${it.width}×${it.height}${it.seed != null ? " · seed " + it.seed : ""}` : "";
}

async function loadGallery() {
  try {
    const items = await api("gallery");
    const g = $("#a-gallery");
    g.innerHTML = items.length ? "" : '<p class="muted small">No generations yet.</p>';
    for (const it of items.slice(0, 60)) {
      const img = document.createElement("img");
      img.loading = "lazy";
      img.src = authUrl(`gallery/${it.id}/image`);
      img.title = it.prompt || it.operation;
      img.addEventListener("click", () => showResults([it]));
      g.append(img);
    }
  } catch { /* gallery is optional */ }
}

$("#a-r-download").addEventListener("click", async () => {
  if (!A.current) return;
  const res = await api(`gallery/${A.current.id}/image`, { raw: true });
  downloadBlob(await res.blob(), `photokit-${A.current.id}.png`);
});
$("#a-r-save").addEventListener("click", async () => {
  if (!A.current) return;
  const pick = await openBrowser({ mode: "save", name: `photokit-${A.current.seed ?? A.current.id}` });
  if (!pick) return;
  try {
    const src = await api(`gallery/${A.current.id}/open`, { body: {} });
    const r = await api("process", { body: { source: src.id, ops: [], encode: { format: "png", png_level: 2 }, action: "save", output_dir: pick.dir, name: pick.name } });
    toast("Saved " + r.path);
  } catch (e) { toast(e.message, true); }
});
$("#a-r-quick").addEventListener("click", async () => {
  if (!A.current) return;
  try { await loadSource(await api(`gallery/${A.current.id}/open`, { body: {} })); showTab("quick"); } catch (e) { toast(e.message, true); }
});
$("#a-r-source").addEventListener("click", async () => {
  if (!A.current) return;
  try { setAiSource(await api(`gallery/${A.current.id}/open`, { body: {} })); } catch (e) { toast(e.message, true); }
});
$("#a-r-reuse").addEventListener("click", () => {
  const it = A.current;
  if (!it) return;
  $("#a-prompt").value = it.prompt || "";
  $("#a-neg").value = it.negative_prompt || "";
  if (it.seed != null) $("#a-seed").value = it.seed;
  if ([...$("#a-mode").options].some((o) => o.value === it.operation)) $("#a-mode").value = it.operation;
  updateAiForm();
  toast("Settings restored");
});

// ---------------------------------------------------------------- Settings

let CONFIG = null;
async function loadSettings() {
  try {
    CONFIG = await api("config");
    $("#c-comfy-url").value = CONFIG.comfyui.url;
    $("#c-comfy-ckpt").value = CONFIG.comfyui.checkpoint || "";
    $("#c-comfy-up").value = CONFIG.comfyui.upscale_model || "";
    $("#c-comfy-wf").value = CONFIG.comfyui.workflow_dir || "";
    $("#c-worker-url").value = CONFIG.worker.url;
    $("#c-ollama-url").value = CONFIG.ollama.url;
    $("#c-ollama-model").value = CONFIG.ollama.model;
    $("#c-backend").value = CONFIG.ai.default_backend;
    $("#c-format").value = CONFIG.defaults.format;
    $("#c-quality").value = CONFIG.defaults.quality;
    if (!Q.formatTouched) {
      $("#q-format").value = CONFIG.defaults.format;
      $("#q-q").value = CONFIG.defaults.quality;
      $("#q-q-v").textContent = CONFIG.defaults.quality;
    }
  } catch (e) { toast(e.message, true); }
}

$("#c-save").addEventListener("click", async () => {
  const opt = (v) => (v.trim() ? v.trim() : null);
  const cfg = structuredClone(CONFIG);
  cfg.comfyui.url = $("#c-comfy-url").value.trim();
  cfg.comfyui.checkpoint = opt($("#c-comfy-ckpt").value);
  cfg.comfyui.upscale_model = opt($("#c-comfy-up").value);
  cfg.comfyui.workflow_dir = opt($("#c-comfy-wf").value);
  cfg.worker.url = $("#c-worker-url").value.trim();
  cfg.ollama.url = $("#c-ollama-url").value.trim();
  cfg.ollama.model = $("#c-ollama-model").value.trim();
  cfg.ai.default_backend = $("#c-backend").value;
  cfg.defaults.format = $("#c-format").value;
  cfg.defaults.quality = Number($("#c-quality").value || 85);
  try {
    const r = await api("config", { body: cfg });
    CONFIG = cfg;
    $("#c-msg").textContent = "Saved to " + r.path;
    refreshAi();
  } catch (e) { $("#c-msg").textContent = e.message; }
});

// ---------------------------------------------------------------- boot

let PRESETS = [];
(async function boot() {
  try {
    const info = await api("info");
    const fmts = info.formats.map((f) => `<option value="${f}">${f.toUpperCase()}</option>`).join("");
    $("#q-format").innerHTML = fmts;
    $("#b-format").insertAdjacentHTML("beforeend", fmts);
    $("#c-format").innerHTML = fmts;
    $("#c-about").innerHTML = `PhotoKit ${esc(info.version)} · ${esc(info.os)}<br>Config: <code>${esc(info.config_path || "—")}</code><br>Data: <code>${esc(info.data_dir)}</code>`;
    PRESETS = await api("presets");
    const opts = PRESETS.map((p) => `<option value="${esc(p.name)}" title="${esc(p.description)}">${esc(p.name)} — ${esc(p.description)}</option>`).join("");
    $("#q-preset").insertAdjacentHTML("beforeend", opts);
    $("#b-preset").insertAdjacentHTML("beforeend", opts);
    await loadSettings();
    let lastBatch = null;
    try { lastBatch = localStorage.getItem("pk-batch-dir"); } catch { /* storage off */ }
    if (lastBatch) $("#b-dir").value = lastBatch;
    if (DESKTOP) $("#q-download").hidden = true;
    refreshAi();
  } catch (e) {
    toast("Cannot reach the PhotoKit server: " + e.message, true);
  }
})();
