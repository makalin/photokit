//! PhotoKit's local web server.
//!
//! Serves the embedded web UI and a JSON API on the loopback interface. The
//! desktop app embeds this same server, so browser and GUI behave identically.
//!
//! Every `/api` request must carry the per-launch token that is injected into
//! the served page, so other websites open in the same browser cannot drive
//! the API.

mod api;
mod assets;
mod gallery;
mod jobs;
mod sources;
mod util;

use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;

use anyhow::{anyhow, Result};
use photokit_core::config::Config;

pub struct ServerOptions {
    pub host: String,
    /// 0 picks a free port.
    pub port: u16,
    pub threads: usize,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self { host: "127.0.0.1".into(), port: 7878, threads: 16 }
    }
}

pub struct State {
    pub config: RwLock<Config>,
    pub token: String,
    pub sources: sources::Sources,
    pub jobs: jobs::Jobs,
    pub thumbs: util::ThumbCache,
}

pub struct RunningServer {
    pub addr: SocketAddr,
    pub token: String,
    server: Arc<tiny_http::Server>,
    workers: Vec<JoinHandle<()>>,
}

impl RunningServer {
    pub fn url(&self) -> String {
        let host = if self.addr.ip().is_unspecified() { "127.0.0.1".to_string() } else { self.addr.ip().to_string() };
        format!("http://{host}:{}/", self.addr.port())
    }

    /// Block until the server stops.
    pub fn join(self) {
        for w in self.workers {
            let _ = w.join();
        }
    }

    pub fn stop(&self) {
        self.server.unblock();
    }
}

pub fn start(opts: ServerOptions) -> Result<RunningServer> {
    let config = Config::load().unwrap_or_default();
    let server = tiny_http::Server::http((opts.host.as_str(), opts.port))
        .map_err(|e| anyhow!("cannot listen on {}:{}: {e}", opts.host, opts.port))?;
    let addr = server.server_addr().to_ip().ok_or_else(|| anyhow!("server is not on an IP socket"))?;
    let server = Arc::new(server);
    let token = photokit_ai::random_token();
    let state = Arc::new(State {
        config: RwLock::new(config),
        token: token.clone(),
        sources: Default::default(),
        jobs: Default::default(),
        thumbs: Default::default(),
    });
    let workers = (0..opts.threads.max(2))
        .map(|_| {
            let server = server.clone();
            let state = state.clone();
            std::thread::spawn(move || {
                while let Ok(req) = server.recv() {
                    api::handle(&state, req);
                }
            })
        })
        .collect();
    Ok(RunningServer { addr, token, server, workers })
}
