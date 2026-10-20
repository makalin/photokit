//! Background jobs (batch processing, AI generation) polled by the UI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Serialize)]
pub struct JobInfo {
    pub id: String,
    pub kind: String,
    /// running, done, failed, cancelled
    pub state: String,
    pub done: usize,
    pub total: usize,
    pub fraction: Option<f32>,
    pub message: Option<String>,
    /// Most recent per-item results (batch) for the live log.
    pub recent: Vec<Value>,
    pub result: Value,
}

pub struct Job {
    pub info: Mutex<JobInfo>,
    pub cancel: AtomicBool,
}

impl Job {
    pub fn update(&self, f: impl FnOnce(&mut JobInfo)) {
        f(&mut self.info.lock().unwrap());
    }

    pub fn finish(&self, result: anyhow::Result<Value>) {
        let cancelled = self.cancel.load(Ordering::Relaxed);
        self.update(|i| match result {
            Ok(v) => {
                i.state = if cancelled { "cancelled" } else { "done" }.into();
                i.fraction = Some(1.0);
                i.result = v;
            }
            Err(e) => {
                i.state = if cancelled { "cancelled" } else { "failed" }.into();
                i.message = Some(format!("{e:#}"));
            }
        });
    }
}

#[derive(Default)]
pub struct Jobs {
    map: Mutex<HashMap<String, Arc<Job>>>,
}

impl Jobs {
    pub fn create(&self, kind: &str, total: usize) -> Arc<Job> {
        let id = format!("j{:x}", photokit_ai::random_seed());
        let job = Arc::new(Job {
            info: Mutex::new(JobInfo {
                id: id.clone(),
                kind: kind.into(),
                state: "running".into(),
                done: 0,
                total,
                fraction: None,
                message: None,
                recent: Vec::new(),
                result: Value::Null,
            }),
            cancel: AtomicBool::new(false),
        });
        let mut map = self.map.lock().unwrap();
        if map.len() > 200 {
            map.retain(|_, j| j.info.lock().unwrap().state == "running");
        }
        map.insert(id, job.clone());
        job
    }

    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.map.lock().unwrap().get(id).cloned()
    }
}
