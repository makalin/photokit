//! ComfyUI adapter: fills API-format workflow templates and drives the HTTP API.

use std::collections::HashSet;

use anyhow::{bail, Context, Result};
use photokit_core::config::ComfyConfig;
use serde_json::{json, Value};

use crate::http::{self, agent, unavailable};
use crate::{
    prepare_image, prepare_mask, random_seed, Backend, Capabilities, GeneratedImage, Health, JobRequest, JobState,
    ModelInfo, Operation, Progress,
};

const BUILTIN: &[(Operation, &str)] = &[
    (Operation::TextToImage, include_str!("../../../workflows/comfyui/text_to_image.json")),
    (Operation::ImageToImage, include_str!("../../../workflows/comfyui/image_to_image.json")),
    (Operation::Inpaint, include_str!("../../../workflows/comfyui/inpaint.json")),
    (Operation::Outpaint, include_str!("../../../workflows/comfyui/outpaint.json")),
    (Operation::Upscale, include_str!("../../../workflows/comfyui/upscale.json")),
];

pub struct ComfyUi {
    cfg: ComfyConfig,
    client_id: String,
}

impl ComfyUi {
    pub fn new(cfg: &ComfyConfig) -> Self {
        Self { cfg: cfg.clone(), client_id: format!("photokit-{}", random_seed()) }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.cfg.url.trim_end_matches('/'), path)
    }

    fn get(&self, path: &str) -> Result<Value> {
        let url = self.url(path);
        let resp = agent(15).get(&url).call().map_err(|e| unavailable("ComfyUI", &self.cfg.url, e))?;
        http::json(resp)
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        let url = self.url(path);
        let resp = agent(30).post(&url).send_json(body).map_err(|e| unavailable("ComfyUI", &self.cfg.url, e))?;
        let text = resp.into_string().unwrap_or_default();
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// The template for an operation: user workflow dir first, then built-in.
    fn template(&self, op: Operation) -> Option<Value> {
        if let Some(dir) = &self.cfg.workflow_dir {
            let path = dir.join(format!("{}.json", op.key()));
            if let Ok(text) = std::fs::read_to_string(path) {
                return serde_json::from_str(&text).ok();
            }
        }
        BUILTIN.iter().find(|(o, _)| *o == op).and_then(|(_, t)| serde_json::from_str(t).ok())
    }

    fn upload(&self, data: &[u8], name: &str) -> Result<String> {
        let (ctype, body) = http::multipart("image", name, data, &[("overwrite", "true"), ("type", "input")]);
        let url = self.url("/upload/image");
        let resp = agent(60)
            .post(&url)
            .set("Content-Type", &ctype)
            .send_bytes(&body)
            .map_err(|e| unavailable("ComfyUI", &self.cfg.url, e))?;
        let v = http::json(resp)?;
        let name = v["name"].as_str().context("ComfyUI upload returned no name")?;
        Ok(match v["subfolder"].as_str().filter(|s| !s.is_empty()) {
            Some(sub) => format!("{sub}/{name}"),
            None => name.to_string(),
        })
    }

    fn model_list(&self, node: &str, input: &str) -> Vec<String> {
        self.get(&format!("/object_info/{node}"))
            .ok()
            .and_then(|v| {
                v[node]["input"]["required"][input][0]
                    .as_array()
                    .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
            })
            .unwrap_or_default()
    }
}

/// Replace placeholder strings in a workflow with typed values.
fn fill(value: &mut Value, vars: &serde_json::Map<String, Value>) {
    match value {
        Value::String(s) => {
            if let Some(key) = s.strip_prefix("{{").and_then(|r| r.strip_suffix("}}")) {
                if let Some(v) = vars.get(key) {
                    *value = v.clone();
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|v| fill(v, vars)),
        Value::Object(o) => o.values_mut().for_each(|v| fill(v, vars)),
        _ => {}
    }
}

fn class_types(workflow: &Value) -> HashSet<String> {
    workflow
        .as_object()
        .map(|o| o.values().filter_map(|n| n["class_type"].as_str().map(String::from)).collect())
        .unwrap_or_default()
}

impl Backend for ComfyUi {
    fn id(&self) -> &'static str {
        "comfyui"
    }

    fn name(&self) -> &'static str {
        "ComfyUI"
    }

    fn health(&self) -> Health {
        match self.get("/system_stats") {
            Ok(v) => {
                let version = v["system"]["comfyui_version"].as_str().unwrap_or("unknown version");
                let device = v["devices"][0]["name"].as_str().unwrap_or("unknown device");
                Health { ok: true, url: self.cfg.url.clone(), message: format!("ComfyUI {version} on {device}") }
            }
            Err(e) => Health { ok: false, url: self.cfg.url.clone(), message: e.to_string() },
        }
    }

    fn discover_capabilities(&self) -> Result<Capabilities> {
        let info = self.get("/object_info")?;
        let nodes: HashSet<String> = info.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        let mut caps = Capabilities::default();
        for op in Operation::ALL {
            match self.template(op) {
                Some(wf) => {
                    let missing: Vec<String> = class_types(&wf).into_iter().filter(|c| !nodes.contains(c)).collect();
                    if missing.is_empty() {
                        caps.operations.push(op);
                    } else {
                        caps.notes.push(format!("{op}: missing nodes {}", missing.join(", ")));
                    }
                }
                None => caps.notes.push(format!("{op}: no workflow template (add {}.json)", op.key())),
            }
        }
        if self.model_list("CheckpointLoaderSimple", "ckpt_name").is_empty() {
            caps.operations.retain(|o| *o == Operation::Upscale);
            caps.notes.push("no checkpoints installed in ComfyUI/models/checkpoints".into());
        }
        if self.model_list("UpscaleModelLoader", "model_name").is_empty() {
            caps.operations.retain(|o| *o != Operation::Upscale);
            caps.notes.push("upscale: no models in ComfyUI/models/upscale_models".into());
        }
        Ok(caps)
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        self.get("/system_stats")?; // fail fast with a clear error when offline
        let mut out: Vec<ModelInfo> = self
            .model_list("CheckpointLoaderSimple", "ckpt_name")
            .into_iter()
            .map(|name| ModelInfo { name, kind: "checkpoint".into() })
            .collect();
        out.extend(
            self.model_list("UpscaleModelLoader", "model_name")
                .into_iter()
                .map(|name| ModelInfo { name, kind: "upscale".into() }),
        );
        Ok(out)
    }

    fn submit_job(&self, req: &JobRequest) -> Result<String> {
        let mut wf = self.template(req.operation).with_context(|| format!("no workflow for {}", req.operation))?;
        let seed = req.seed.unwrap_or_else(random_seed);
        let mut vars = serde_json::Map::new();
        let mut set = |k: &str, v: Value| {
            vars.insert(k.to_string(), v);
        };
        set("prompt", json!(req.prompt));
        set("negative", json!(req.negative_prompt));
        set("seed", json!(seed));
        set("steps", json!(req.steps));
        set("cfg", json!(req.cfg));
        set("width", json!(req.width / 8 * 8));
        set("height", json!(req.height / 8 * 8));
        set("batch", json!(req.batch.clamp(1, 8)));
        set("denoise", json!(req.denoise.clamp(0.0, 1.0)));
        set("pad_left", json!(req.pad[0]));
        set("pad_top", json!(req.pad[1]));
        set("pad_right", json!(req.pad[2]));
        set("pad_bottom", json!(req.pad[3]));

        let models = self.list_models()?;
        let pick = |kind: &str, preferred: Option<&String>| -> Option<String> {
            preferred
                .filter(|p| models.iter().any(|m| m.kind == kind && &m.name == *p))
                .cloned()
                .or_else(|| models.iter().find(|m| m.kind == kind).map(|m| m.name.clone()))
        };
        if req.operation == Operation::Upscale {
            let m = pick("upscale", req.model.as_ref().or(self.cfg.upscale_model.as_ref()))
                .context("no upscale model installed in ComfyUI")?;
            set("upscale_model", json!(m));
        } else {
            let m = pick("checkpoint", req.model.as_ref().or(self.cfg.checkpoint.as_ref()))
                .context("no checkpoint installed in ComfyUI")?;
            set("checkpoint", json!(m));
        }

        if let Some(src) = &req.source {
            let max = if req.operation == Operation::Upscale { 2048 } else { req.width.max(req.height) };
            let (png, w, h) = prepare_image(src, max)?;
            set("image", json!(self.upload(&png, "photokit-source.png")?));
            if let Some(mask) = &req.mask {
                let mask = prepare_mask(mask, w, h)?;
                set("mask", json!(self.upload(&mask, "photokit-mask.png")?));
            }
        }

        fill(&mut wf, &vars);
        let resp = self.post("/prompt", json!({ "prompt": wf, "client_id": self.client_id }))?;
        if let Some(err) = resp.get("error") {
            bail!("ComfyUI rejected the workflow: {} {}", err, resp.get("node_errors").cloned().unwrap_or_default());
        }
        resp["prompt_id"].as_str().map(String::from).context("ComfyUI returned no prompt_id")
    }

    fn get_progress(&self, job: &str) -> Result<Progress> {
        let hist = self.get(&format!("/history/{job}"))?;
        if let Some(entry) = hist.get(job) {
            let status = &entry["status"];
            if status["status_str"].as_str() == Some("error") {
                let msg = status["messages"]
                    .as_array()
                    .and_then(|m| m.iter().find(|x| x[0] == "execution_error"))
                    .map(|x| x[1]["exception_message"].as_str().unwrap_or("execution error").to_string());
                return Ok(Progress { state: JobState::Failed, fraction: None, message: msg });
            }
            if status["completed"].as_bool().unwrap_or(true) {
                return Ok(Progress { state: JobState::Completed, fraction: Some(1.0), message: None });
            }
        }
        let queue = self.get("/queue")?;
        let contains =
            |key: &str| queue[key].as_array().is_some_and(|a| a.iter().any(|item| item[1].as_str() == Some(job)));
        let state = if contains("queue_running") {
            JobState::Running
        } else if contains("queue_pending") {
            JobState::Queued
        } else if hist.get(job).is_none() {
            JobState::Cancelled
        } else {
            JobState::Running
        };
        Ok(Progress { state, fraction: None, message: None })
    }

    fn cancel_job(&self, job: &str) -> Result<()> {
        self.post("/queue", json!({ "delete": [job] }))?;
        let queue = self.get("/queue")?;
        let running = queue["queue_running"].as_array().is_some_and(|a| a.iter().any(|i| i[1].as_str() == Some(job)));
        if running {
            self.post("/interrupt", json!({}))?;
        }
        Ok(())
    }

    fn collect_results(&self, job: &str) -> Result<Vec<GeneratedImage>> {
        let hist = self.get(&format!("/history/{job}"))?;
        let outputs = hist[job]["outputs"].as_object().context("job has no outputs")?;
        let mut images = Vec::new();
        for node in outputs.values() {
            for img in node["images"].as_array().into_iter().flatten() {
                if img["type"].as_str() == Some("temp") {
                    continue;
                }
                let url = self.url("/view");
                let resp = agent(120)
                    .get(&url)
                    .query("filename", img["filename"].as_str().unwrap_or_default())
                    .query("subfolder", img["subfolder"].as_str().unwrap_or_default())
                    .query("type", img["type"].as_str().unwrap_or("output"))
                    .call()
                    .map_err(|e| unavailable("ComfyUI", &self.cfg.url, e))?;
                images.push(GeneratedImage { data: http::read_bytes(resp)?, seed: None });
            }
        }
        if images.is_empty() {
            bail!("ComfyUI finished but produced no images");
        }
        Ok(images)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_parse_and_fill_typed_values() {
        for (op, text) in BUILTIN {
            let wf: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{op}: {e}"));
            assert!(!class_types(&wf).is_empty());
        }
        let mut wf: Value = serde_json::from_str(BUILTIN[0].1).unwrap();
        let mut vars = serde_json::Map::new();
        vars.insert("seed".into(), json!(42));
        vars.insert("prompt".into(), json!("a \"quoted\" cat"));
        fill(&mut wf, &vars);
        assert_eq!(wf["3"]["inputs"]["seed"], json!(42));
        assert_eq!(wf["6"]["inputs"]["text"], json!("a \"quoted\" cat"));
    }
}
