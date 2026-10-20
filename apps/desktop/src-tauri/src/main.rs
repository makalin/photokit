//! PhotoKit desktop: a native window around the same local server the web
//! version uses, started on a free loopback port inside this process.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{WebviewUrl, WebviewWindowBuilder};

fn main() {
    let server = photokit_server::start(photokit_server::ServerOptions {
        host: "127.0.0.1".into(),
        port: 0,
        ..Default::default()
    })
    .expect("failed to start the PhotoKit engine");
    let url = format!("{}?desktop=1", server.url());

    tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("PhotoKit")
                .inner_size(1320.0, 860.0)
                .min_inner_size(900.0, 600.0)
                // Let the page receive HTML5 file drops instead of Tauri.
                .disable_drag_drop_handler()
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running PhotoKit");
}
