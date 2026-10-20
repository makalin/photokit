//! The web UI, embedded at compile time. Set `PHOTOKIT_UI_DIR` to serve the
//! files from disk instead while developing the UI.

const FILES: &[(&str, &str, &[u8])] = &[
    ("index.html", "text/html; charset=utf-8", include_bytes!("../../../apps/web/ui/index.html")),
    ("app.css", "text/css; charset=utf-8", include_bytes!("../../../apps/web/ui/app.css")),
    ("app.js", "text/javascript; charset=utf-8", include_bytes!("../../../apps/web/ui/app.js")),
    ("logo.svg", "image/svg+xml", include_bytes!("../../../apps/web/ui/logo.svg")),
    ("mark.svg", "image/svg+xml", include_bytes!("../../../apps/web/ui/mark.svg")),
];

pub fn get(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let name = match path.trim_start_matches('/') {
        "" => "index.html",
        p => p,
    };
    let (_, mime, embedded) = FILES.iter().find(|(n, _, _)| *n == name)?;
    if let Some(dir) = std::env::var_os("PHOTOKIT_UI_DIR") {
        if let Ok(data) = std::fs::read(std::path::Path::new(&dir).join(name)) {
            return Some((data, mime));
        }
    }
    Some((embedded.to_vec(), mime))
}
