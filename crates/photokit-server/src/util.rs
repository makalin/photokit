use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tiny_http::{Header, Request, Response, StatusCode};

pub type Resp = Response<std::io::Cursor<Vec<u8>>>;

pub fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid header")
}

pub fn json<T: Serialize>(value: &T) -> Resp {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    Response::from_data(body).with_header(header("Content-Type", "application/json"))
}

pub fn error(code: u16, msg: impl std::fmt::Display) -> Resp {
    json(&serde_json::json!({ "error": msg.to_string() })).with_status_code(StatusCode(code))
}

pub fn bytes(data: Vec<u8>, mime: &str) -> Resp {
    Response::from_data(data).with_header(header("Content-Type", mime)).with_header(header("Cache-Control", "no-store"))
}

pub fn read_body(req: &mut Request, limit: u64) -> Result<Vec<u8>> {
    let reader = req.as_reader();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            return Ok(buf);
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() as u64 > limit {
            anyhow::bail!("request body is too large");
        }
    }
}

pub fn read_json<T: DeserializeOwned>(req: &mut Request) -> Result<T> {
    let body = read_body(req, 64 * 1024 * 1024)?;
    serde_json::from_slice(&body).context("invalid JSON body")
}

pub fn header_value(req: &Request, name: &str) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

/// Split `/a/b?x=1&y=2` into the path and decoded query pairs.
pub fn split_url(url: &str) -> (String, HashMap<String, String>) {
    let (path, q) = url.split_once('?').unwrap_or((url, ""));
    let query = q
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    (percent_decode(path), query)
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 2;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

type ThumbMap = HashMap<(PathBuf, u32), (SystemTime, Vec<u8>)>;

/// Small LRU-ish cache for folder thumbnails, keyed by path + mtime + size.
#[derive(Default)]
pub struct ThumbCache {
    map: Mutex<ThumbMap>,
}

impl ThumbCache {
    pub fn get_or_make(&self, path: &Path, size: u32) -> Result<Vec<u8>> {
        let mtime = std::fs::metadata(path)?.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let key = (path.to_path_buf(), size);
        if let Some((t, data)) = self.map.lock().unwrap().get(&key) {
            if *t == mtime {
                return Ok(data.clone());
            }
        }
        let decoded = photokit_core::codecs::decode_file(path)?;
        let thumb = photokit_core::ops::thumbnail(&decoded.image, size);
        let data = encode_preview(&thumb, 80)?.0;
        let mut map = self.map.lock().unwrap();
        if map.len() > 800 {
            map.clear();
        }
        map.insert(key, (mtime, data.clone()));
        Ok(data)
    }
}

/// Fast display encoding: JPEG for opaque images, PNG when alpha matters.
pub fn encode_preview(img: &image::DynamicImage, quality: u8) -> Result<(Vec<u8>, &'static str)> {
    use photokit_core::codecs::{encode, EncodeOptions, OutputFormat};
    let (format, mime) =
        if img.color().has_alpha() { (OutputFormat::Png, "image/png") } else { (OutputFormat::Jpeg, "image/jpeg") };
    let opts = EncodeOptions { format, quality, png_level: 0, ..Default::default() };
    Ok((encode(img, &opts, None)?, mime))
}

/// Top-level locations for the folder browser.
pub fn roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(h) = dirs::home_dir() {
        out.push(h);
    }
    for d in [dirs::picture_dir(), dirs::desktop_dir(), dirs::download_dir()].into_iter().flatten() {
        out.push(d);
    }
    if cfg!(windows) {
        for c in b'A'..=b'Z' {
            let p = PathBuf::from(format!("{}:\\", c as char));
            if p.exists() {
                out.push(p);
            }
        }
    } else {
        out.push(PathBuf::from("/"));
        if cfg!(target_os = "macos") {
            out.push(PathBuf::from("/Volumes"));
        }
    }
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_urls() {
        let (p, q) = super::split_url("/api/fs/list?path=%2FUsers%2Fme%20x&n=1");
        assert_eq!(p, "/api/fs/list");
        assert_eq!(q["path"], "/Users/me x");
        assert_eq!(super::percent_decode("100%"), "100%");
    }
}
