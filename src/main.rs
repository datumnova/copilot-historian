mod api;
mod index;
mod model;
mod scan;
mod util;

use api::AppState;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tiny_http::{Header, Response, Server};

const UI_HTML: &str = include_str!("../assets/index.html");

fn default_root() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".copilot").join("session-state")
}

fn build_state(root: &PathBuf) -> AppState {
    let t0 = std::time::Instant::now();
    let sessions = scan::scan_root(root);
    let index = index::SearchIndex::build(&sessions);
    AppState {
        sessions,
        index,
        root: root.display().to_string(),
        scanned_at: util::now_epoch_ms(),
        scan_ms: t0.elapsed().as_millis() as u64,
    }
}

fn main() {
    let mut root = default_root();
    let mut port: u16 = 4577;
    let mut open_browser = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" | "-d" => {
                if let Some(v) = args.next() {
                    root = PathBuf::from(v);
                }
            }
            "--port" | "-p" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(4577);
                }
            }
            "--no-open" => open_browser = false,
            "--help" | "-h" => {
                println!(
                    "copilot-historian — dashboard for GitHub Copilot CLI session history\n\n\
                     USAGE: copilot-historian [--dir <session-state dir>] [--port <port>] [--no-open]\n\n\
                     Default dir:  ~/.copilot/session-state\n\
                     Default port: 4577"
                );
                return;
            }
            "--version" | "-V" => {
                println!("copilot-historian {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("unknown argument: {other} (try --help)");
                return;
            }
        }
    }

    if !root.exists() {
        eprintln!("session directory not found: {}", root.display());
        eprintln!("point me at it with:  copilot-historian --dir <path>");
        std::process::exit(1);
    }

    println!("copilot-historian  scanning {} ...", root.display());
    let state = build_state(&root);
    let n_usage = state.sessions.iter().filter(|s| s.segments > 0).count();
    let aiu: f64 = state.sessions.iter().map(|s| s.aiu).sum();
    println!(
        "  {} sessions ({} with usage data) · {:.0} AIU total · indexed in {} ms",
        state.sessions.len(),
        n_usage,
        aiu,
        state.scan_ms
    );

    let shared: Arc<RwLock<Arc<AppState>>> = Arc::new(RwLock::new(Arc::new(state)));
    let addr = format!("127.0.0.1:{port}");
    let server = match Server::http(&addr) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("failed to bind {addr}: {e}");
            std::process::exit(1);
        }
    };
    let url = format!("http://{addr}");
    println!("  serving {url}  (Ctrl+C to stop)");

    if open_browser {
        let _ = open_url(&url);
    }

    let mut workers = Vec::new();
    for _ in 0..4 {
        let server = Arc::clone(&server);
        let shared = Arc::clone(&shared);
        workers.push(std::thread::spawn(move || loop {
            let rq = match server.recv() {
                Ok(rq) => rq,
                Err(_) => break,
            };
            let state = { shared.read().unwrap().clone() };
            handle(rq, state, &shared);
        }));
    }
    for w in workers {
        let _ = w.join();
    }
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", url])
            .spawn()
            .map(|_| ())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().map(|_| ())
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
    }
}

fn handle(rq: tiny_http::Request, st: Arc<AppState>, shared: &Arc<RwLock<Arc<AppState>>>) {
    let url = rq.url().to_string();
    let path = url.split('?').next().unwrap_or("/").to_string();
    let q = api::parse_query(&url);

    let json_resp = |rq: tiny_http::Request, body: String, status: u32| {
        let resp = Response::from_string(body)
            .with_status_code(status)
            .with_header(
                Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..])
                    .unwrap(),
            )
            .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap());
        let _ = rq.respond(resp);
    };

    match path.as_str() {
        "/" | "/index.html" => {
            let resp = Response::from_string(UI_HTML).with_header(
                Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap(),
            );
            let _ = rq.respond(resp);
        }
        "/api/overview" => json_resp(rq, api::overview(&st).to_string(), 200),
        "/api/sessions" => json_resp(rq, api::sessions(&st, &q).to_string(), 200),
        "/api/search" => json_resp(rq, api::search(&st, &q).to_string(), 200),
        "/api/repos" => json_resp(rq, api::repos(&st).to_string(), 200),
        "/api/rescan" => {
            let root = PathBuf::from(&st.root);
            let fresh = Arc::new(build_state(&root));
            let summary = serde_json::json!({
                "sessions": fresh.sessions.len(),
                "scan_ms": fresh.scan_ms,
                "scanned_at": fresh.scanned_at,
            });
            *shared.write().unwrap() = fresh;
            json_resp(rq, summary.to_string(), 200);
        }
        p if p.starts_with("/api/session/") => {
            let id = p.trim_start_matches("/api/session/");
            match api::session_detail(&st, id) {
                Some(v) => json_resp(rq, v.to_string(), 200),
                None => json_resp(rq, r#"{"error":"session not found"}"#.into(), 404),
            }
        }
        _ => json_resp(rq, r#"{"error":"not found"}"#.into(), 404),
    }
}
