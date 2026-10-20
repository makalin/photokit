//! Filename templates, sanitizing and collision-safe output paths.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Values available to a name template.
#[derive(Debug, Clone, Default)]
pub struct NameContext {
    pub stem: String,
    pub ext: String,
    pub parent: String,
    pub index: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub modified: Option<SystemTime>,
}

impl NameContext {
    pub fn for_path(path: &Path, index: u64) -> Self {
        Self {
            stem: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            ext: path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            parent: path
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            index,
            width: None,
            height: None,
            modified: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
        }
    }
}

/// Tokens a template needs that are expensive to compute.
pub fn needs_dimensions(template: &str) -> bool {
    template.contains("{w}") || template.contains("{h}")
}

/// Render a template such as `trip-{n:3}-{w}x{h}`.
///
/// Tokens: `{name}`, `{ext}`, `{parent}`, `{n}` / `{n:WIDTH}`, `{w}`, `{h}`,
/// `{date}` (YYYY-MM-DD), `{time}` (HHMMSS), `{datetime}` (YYYYMMDD-HHMMSS).
/// Unknown tokens are kept literally so typos are visible in previews.
pub fn render(template: &str, ctx: &NameContext) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let Some(close) = rest[open..].find('}') else {
            out.push_str(&rest[open..]);
            return sanitize(&out);
        };
        let token = &rest[open + 1..open + close];
        let (name, arg) = token.split_once(':').unwrap_or((token, ""));
        let (date, time) = ctx.modified.map(civil).unwrap_or_default();
        match name {
            "name" => out.push_str(&ctx.stem),
            "ext" => out.push_str(&ctx.ext),
            "parent" => out.push_str(&ctx.parent),
            "n" | "i" => {
                let width: usize = arg.parse().unwrap_or(0);
                out.push_str(&format!("{:0width$}", ctx.index, width = width));
            }
            "w" => out.push_str(&ctx.width.map(|v| v.to_string()).unwrap_or_default()),
            "h" => out.push_str(&ctx.height.map(|v| v.to_string()).unwrap_or_default()),
            "date" => out.push_str(&date),
            "time" => out.push_str(&time),
            "datetime" => out.push_str(&format!("{}-{}", date.replace('-', ""), time)),
            _ => out.push_str(&rest[open..=open + close]),
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    sanitize(&out)
}

/// Make a string safe as a file name on Windows, macOS and Linux.
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s.trim().trim_end_matches('.').to_string();
    if s.is_empty() {
        s = "image".into();
    }
    let upper = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&upper.as_str()) {
        s.insert(0, '_');
    }
    s
}

/// Return `path`, or `name-1.ext`, `name-2.ext`, … if it already exists.
pub fn unique_path(path: &Path) -> PathBuf {
    unique_path_with(path, |p| p.exists())
}

pub fn unique_path_with(path: &Path, taken: impl Fn(&Path) -> bool) -> PathBuf {
    if !taken(path) {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    let dir = path.parent().unwrap_or(Path::new(""));
    (1..).map(|i| dir.join(format!("{stem}-{i}{ext}"))).find(|p| !taken(p)).expect("infinite iterator")
}

/// Write via a temporary sibling then rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!(
        "{}.pktmp",
        path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    std::fs::write(&tmp, data)?;
    if path.exists() {
        // Windows rename does not replace existing files.
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)
}

/// (YYYY-MM-DD, HHMMSS) in UTC, without pulling in a date crate.
fn civil(t: SystemTime) -> (String, String) {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's days-from-civil inverse.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (format!("{y:04}-{m:02}-{d:02}"), format!("{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn renders_tokens() {
        let ctx = NameContext {
            stem: "IMG_0001".into(),
            ext: "JPG".into(),
            parent: "trip".into(),
            index: 7,
            width: Some(1200),
            height: Some(800),
            modified: Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        };
        assert_eq!(render("{parent}-{n:3}", &ctx), "trip-007");
        assert_eq!(render("{name}_{w}x{h}", &ctx), "IMG_0001_1200x800");
        assert_eq!(render("{date}", &ctx), "2023-11-14");
        assert_eq!(render("{datetime}", &ctx), "20231114-221320");
        assert_eq!(render("a/b:{bogus}", &ctx), "a_b_{bogus}");
    }

    #[test]
    fn sanitizes_reserved() {
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("  .. "), "image");
    }

    #[test]
    fn unique_appends_counter() {
        let taken = ["a.png", "a-1.png"];
        let p = unique_path_with(Path::new("a.png"), |p| taken.iter().any(|t| Path::new(t) == p));
        assert_eq!(p, PathBuf::from("a-2.png"));
    }
}
