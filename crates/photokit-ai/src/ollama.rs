//! Optional Ollama integration: prompt rewriting and natural-language edit plans.
//! Ollama is never used to generate images.

use anyhow::{bail, Context, Result};
use photokit_core::config::OllamaConfig;
use photokit_core::{Fit, Flip, Op, OutputFormat, ResizeSpec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::{agent, unavailable};

pub struct Ollama {
    cfg: OllamaConfig,
}

/// A validated, previewable edit plan produced from a natural-language request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssistPlan {
    pub ops: Vec<Op>,
    pub format: Option<OutputFormat>,
    pub quality: Option<u8>,
    pub summary: String,
}

impl Ollama {
    pub fn new(cfg: &OllamaConfig) -> Self {
        Self { cfg: cfg.clone() }
    }

    fn generate(&self, prompt: &str, system: &str, json_mode: bool) -> Result<String> {
        let url = format!("{}/api/generate", self.cfg.url.trim_end_matches('/'));
        let mut body = json!({ "model": self.cfg.model, "prompt": prompt, "system": system, "stream": false });
        if json_mode {
            body["format"] = json!("json");
        }
        let resp = agent(120).post(&url).send_json(body).map_err(|e| unavailable("Ollama", &self.cfg.url, e))?;
        let v: Value = resp.into_json()?;
        v["response"].as_str().map(|s| s.trim().to_string()).context("Ollama returned no response")
    }

    pub fn health(&self) -> crate::Health {
        let url = format!("{}/api/version", self.cfg.url.trim_end_matches('/'));
        match agent(5).get(&url).call() {
            Ok(r) => {
                let v: Value = r.into_json().unwrap_or_default();
                crate::Health {
                    ok: true,
                    url: self.cfg.url.clone(),
                    message: format!("Ollama {} (model {})", v["version"].as_str().unwrap_or("?"), self.cfg.model),
                }
            }
            Err(e) => crate::Health {
                ok: false,
                url: self.cfg.url.clone(),
                message: unavailable("Ollama", &self.cfg.url, e).to_string(),
            },
        }
    }

    pub fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.cfg.url.trim_end_matches('/'));
        let v: Value = agent(10).get(&url).call().map_err(|e| unavailable("Ollama", &self.cfg.url, e))?.into_json()?;
        Ok(v["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str().map(String::from)).collect())
    }

    /// Rewrite a short idea into a detailed image-generation prompt.
    pub fn enhance_prompt(&self, prompt: &str) -> Result<String> {
        let system = "You rewrite short ideas into a single vivid prompt for an image diffusion model. \
            Describe subject, setting, lighting, style and camera in under 70 words. \
            Reply with the prompt only, no quotes or preamble.";
        let out = self.generate(prompt, system, false)?;
        Ok(out.trim_matches('"').to_string())
    }

    /// Turn a request like "make it 800px wide webp" into a validated plan.
    pub fn assist(&self, request: &str) -> Result<AssistPlan> {
        let system = r#"Convert the user's photo editing request into JSON with only these optional keys:
{"width":int,"height":int,"percent":number,"fit":"contain|cover|fill","rotate":90|180|270,
"flip":"horizontal|vertical","grayscale":bool,"format":"jpeg|png|webp|avif","quality":1-100,
"summary":"one short sentence"}. Omit keys that were not requested. Output JSON only."#;
        let text = self.generate(request, system, true)?;
        parse_plan(&text)
    }
}

pub fn parse_plan(text: &str) -> Result<AssistPlan> {
    let v: Value = serde_json::from_str(text).context("assistant returned invalid JSON")?;
    let num = |k: &str| v[k].as_f64();
    let mut plan = AssistPlan { summary: v["summary"].as_str().unwrap_or("").to_string(), ..Default::default() };
    if let Some(d) = num("rotate") {
        let d = d as i64;
        if ![90, 180, 270, -90].contains(&d) {
            bail!("unsupported rotation {d}");
        }
        plan.ops.push(Op::Rotate { degrees: d.rem_euclid(360) as f32 });
    }
    match v["flip"].as_str() {
        Some("horizontal") => plan.ops.push(Op::Flip { axis: Flip::Horizontal }),
        Some("vertical") => plan.ops.push(Op::Flip { axis: Flip::Vertical }),
        _ => {}
    }
    let dim = |k: &str| num(k).filter(|n| (1.0..=20000.0).contains(n)).map(|n| n as u32);
    let resize = ResizeSpec {
        width: dim("width"),
        height: dim("height"),
        percent: num("percent").filter(|p| (1.0..=1000.0).contains(p)).map(|p| p as f32),
        fit: v["fit"].as_str().and_then(|f| f.parse::<Fit>().ok()).unwrap_or_default(),
        upscale: false,
    };
    if !resize.is_noop() {
        plan.ops.push(Op::Resize(resize));
    }
    if v["grayscale"].as_bool() == Some(true) {
        plan.ops.push(Op::Grayscale);
    }
    plan.format = v["format"].as_str().and_then(|f| f.parse().ok());
    plan.quality = num("quality").map(|q| q.clamp(1.0, 100.0) as u8);
    if plan.ops.is_empty() && plan.format.is_none() && plan.quality.is_none() {
        bail!("the assistant could not map that request to an edit");
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    #[test]
    fn plan_parsing_validates() {
        let p = super::parse_plan(r#"{"width":800,"format":"webp","rotate":90,"summary":"x"}"#).unwrap();
        assert_eq!(p.ops.len(), 2);
        assert!(super::parse_plan(r#"{"rotate":45}"#).is_err());
        assert!(super::parse_plan(r#"{}"#).is_err());
    }
}
