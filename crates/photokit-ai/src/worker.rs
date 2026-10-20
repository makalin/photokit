//! Adapter for the optional Python worker in `workers/local-inference`.
//!
//! The worker speaks a small JSON API that mirrors the backend contract, so
//! this adapter is mostly a thin translation layer.

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

use crate::http::{self, agent, unavailable};
use crate::{
    prepare_image, prepare_mask, Backend, Capabilities, GeneratedImage, Health, JobRequest, JobState, ModelInfo,
    Operation, Progress,
};

pub struct LocalWorker {
    url: String,
}

impl LocalWorker {
    pub fn new(url: &str) -> Self {
        Self { url: url.trim_end_matches('/').to_string() }
    }

    fn get(&self, path: &str) -> Result<Value> {
        let resp = agent(30)
            .get(&format!("{}{path}", self.url))
            .call()
            .map_err(|e| unavailable("Local worker", &self.url, e))?;
        http::json(resp)
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        let resp = agent(120)
            .post(&format!("{}{path}", self.url))
            .send_json(body)
            .map_err(|e| unavailable("Local worker", &self.url, e))?;
        http::json(resp)
    }
}

impl Backend for LocalWorker {
    fn id(&self) -> &'static str {
        "worker"
    }

    fn name(&self) -> &'static str {
        "Local worker"
    }

    fn health(&self) -> Health {
        match self.get("/health") {
            Ok(v) => Health {
                ok: true,
                url: self.url.clone(),
                message: format!(
                    "{} on {}",
                    v["engine"].as_str().unwrap_or("worker"),
                    v["device"].as_str().unwrap_or("unknown device")
                ),
            },
            Err(e) => Health { ok: false, url: self.url.clone(), message: e.to_string() },
        }
    }

    fn discover_capabilities(&self) -> Result<Capabilities> {
        let v = self.get("/capabilities")?;
        let operations = v["operations"]
            .as_array()
            .map(|a| a.iter().filter_map(|o| serde_json::from_value::<Operation>(o.clone()).ok()).collect())
            .unwrap_or_default();
        let notes = v["notes"]
            .as_array()
            .map(|a| a.iter().filter_map(|n| n.as_str().map(String::from)).collect())
            .unwrap_or_default();
        Ok(Capabilities { operations, notes })
    }

    fn list_models(&self) -> Result<Vec<ModelInfo>> {
        let v = self.get("/models")?;
        Ok(serde_json::from_value(v["models"].clone()).unwrap_or_default())
    }

    fn submit_job(&self, req: &JobRequest) -> Result<String> {
        let mut body = serde_json::to_value(req)?;
        if let Some(src) = &req.source {
            let max = if req.operation == Operation::Upscale { 2048 } else { req.width.max(req.height) };
            let (png, w, h) = prepare_image(src, max)?;
            body["source"] = json!(B64.encode(png));
            if let Some(mask) = &req.mask {
                body["mask"] = json!(B64.encode(prepare_mask(mask, w, h)?));
            }
        }
        let v = self.post("/jobs", body)?;
        if let Some(e) = v["error"].as_str() {
            anyhow::bail!("worker rejected job: {e}");
        }
        v["id"].as_str().map(String::from).context("worker returned no job id")
    }

    fn get_progress(&self, job: &str) -> Result<Progress> {
        let v = self.get(&format!("/jobs/{job}"))?;
        let state: JobState = serde_json::from_value(v["state"].clone()).unwrap_or(JobState::Running);
        Ok(Progress {
            state,
            fraction: v["progress"].as_f64().map(|f| f as f32),
            message: v["message"].as_str().map(String::from),
        })
    }

    fn cancel_job(&self, job: &str) -> Result<()> {
        self.post(&format!("/jobs/{job}/cancel"), json!({}))?;
        Ok(())
    }

    fn collect_results(&self, job: &str) -> Result<Vec<GeneratedImage>> {
        let v = self.get(&format!("/jobs/{job}/result"))?;
        v["images"]
            .as_array()
            .context("worker returned no images")?
            .iter()
            .map(|img| {
                let data = B64.decode(img["data"].as_str().unwrap_or_default())?;
                Ok(GeneratedImage { data, seed: img["seed"].as_u64() })
            })
            .collect()
    }
}
