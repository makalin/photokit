use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

use crate::AiError;

pub(crate) fn agent(timeout_secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(2)).timeout(Duration::from_secs(timeout_secs)).build()
}

pub(crate) fn unavailable(backend: &str, url: &str, e: ureq::Error) -> anyhow::Error {
    let reason = match e {
        ureq::Error::Status(code, resp) => {
            format!("HTTP {code}: {}", resp.into_string().unwrap_or_default().chars().take(300).collect::<String>())
        }
        ureq::Error::Transport(t) => match t.kind() {
            ureq::ErrorKind::ConnectionFailed => "connection refused (is it running?)".into(),
            ureq::ErrorKind::Dns => "unknown host".into(),
            _ => t.message().map(String::from).unwrap_or_else(|| t.kind().to_string()),
        },
    };
    AiError::Unavailable { backend: backend.into(), url: url.into(), reason }.into()
}

pub(crate) fn read_bytes(resp: ureq::Response) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    resp.into_reader().take(512 * 1024 * 1024).read_to_end(&mut buf).context("reading response")?;
    Ok(buf)
}

pub(crate) fn json(resp: ureq::Response) -> Result<Value> {
    resp.into_json().context("invalid JSON from backend")
}

/// Build a multipart/form-data body with one file part and text fields.
pub(crate) fn multipart(file_field: &str, filename: &str, data: &[u8], fields: &[(&str, &str)]) -> (String, Vec<u8>) {
    let boundary = format!("----photokit{}", crate::random_seed());
    let mut body = Vec::with_capacity(data.len() + 512);
    for (k, v) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{file_field}\"; filename=\"{filename}\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}
