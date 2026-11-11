//! rsPostman desktop application.
//!
//! The window shows exactly the same UI as `rspostman gui`: the backend is started in-process
//! on a free loopback port and the webview is pointed at it. The UI never talks to Tauri's
//! IPC, so no capabilities are granted to the page.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rspostman_core::Workspace;
use tauri::webview::DownloadEvent;
use tauri::{WebviewUrl, WebviewWindowBuilder};

fn downloads_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(std::path::PathBuf::from)?;
    let dir = home.join("Downloads");
    dir.is_dir().then_some(dir)
}

fn workspace_from_args() -> Option<std::path::PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2).find(|w| w[0] == "--workspace" || w[0] == "-w").map(|w| std::path::PathBuf::from(&w[1]))
}

fn main() {
    let ws = match Workspace::open(workspace_from_args()) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("rsPostman: cannot open the workspace: {e:#}");
            std::process::exit(1);
        }
    };
    let server = match rspostman_server::start(ws, 0) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("rsPostman: cannot start the local backend: {e:#}");
            std::process::exit(1);
        }
    };
    let url = server.url();
    eprintln!("rsPostman backend listening on {url}");

    let result = tauri::Builder::default()
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url.parse()?))
                .title("rsPostman")
                .inner_size(1360.0, 860.0)
                .min_inner_size(820.0, 520.0)
                // exports, reports and response downloads go to the user's Downloads folder
                .on_download(|_webview, event| {
                    if let DownloadEvent::Requested { destination, .. } = event {
                        if let (Some(dir), Some(name)) = (downloads_dir(), destination.file_name().map(|n| n.to_owned())) {
                            *destination = dir.join(name);
                        }
                    }
                    true
                })
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!());

    server.stop();
    if let Err(e) = result {
        eprintln!("rsPostman: {e}");
        std::process::exit(1);
    }
}
