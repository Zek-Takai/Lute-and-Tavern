//! Installed builds serve the interface over http://localhost (Apple's MusicKit sign-in only answers pages
//! served over http(s)), and that origin may call the app's commands. So the app must be the only thing
//! answering there: it binds both 127.0.0.1 and ::1 on its port before the window opens, and if either is
//! already taken by something else, the window uses the app's built-in tauri:// address instead (everything
//! works except Apple Music sign-in) rather than loading whatever answers on that port.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use tauri::{AppHandle, Runtime};
use tiny_http::{Header, Response, Server};

/// Starts serving; false when the port isn't ours (then the caller must not load http://localhost).
pub fn start<R: Runtime>(app: &AppHandle<R>, port: u16) -> bool {
    let v4 = match Server::http(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
        Ok(s) => s,
        Err(e) => {
            crate::log::write("error", &format!("Port {port} is in use by another program ({e}); Apple Music sign-in is unavailable this time."));
            return false;
        }
    };
    let v6 = match Server::http(SocketAddr::from((Ipv6Addr::LOCALHOST, port))) {
        Ok(s) => Some(s),
        // no IPv6 on this computer: "localhost" can only mean 127.0.0.1
        Err(e) if e.to_string().contains("not available") || e.to_string().contains("Address family") => None,
        Err(e) => {
            crate::log::write("error", &format!("Port {port} (IPv6) is in use by another program ({e}); Apple Music sign-in is unavailable this time."));
            return false;
        }
    };
    for server in std::iter::once(v4).chain(v6) {
        let server = Arc::new(server);
        let assets = app.asset_resolver();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let path = req.url().split(['?', '#']).next().unwrap_or("/").to_string();
                let res = match assets.get(path) {
                    Some(asset) => {
                        let mut r = Response::from_data(asset.bytes);
                        let mut add = |k: &str, v: &str| {
                            if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
                                r.add_header(h);
                            }
                        };
                        add("Content-Type", &asset.mime_type);
                        if let Some(csp) = &asset.csp_header {
                            add("Content-Security-Policy", csp);
                        }
                        add("Cache-Control", "no-cache");
                        r
                    }
                    None => Response::from_data(Vec::new()).with_status_code(404),
                };
                // a browser that went away mid-answer is not an error worth stopping for
                let _ = req.respond(res);
            }
        });
    }
    true
}
