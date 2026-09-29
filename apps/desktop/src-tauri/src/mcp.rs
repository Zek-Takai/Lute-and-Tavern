//! Local MCP server so other AI apps (Claude, Hermes, anything that speaks MCP) can write Suno songs into the
//! library. Off until the person turns it on. Listens on 127.0.0.1 only and needs a bearer token that lives in
//! the system keychain. The tools themselves are in lt_core::mcp.

use crate::{writer, AppState};
use serde::Serialize;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tiny_http::{Header, Method, Request, Response, Server};

pub const PORT: u16 = 17655;
const KEYCHAIN_SERVICE: &str = "Lute & Tavern";
const KEYCHAIN_USER: &str = "mcp-token";
const MAX_BODY: u64 = 2 * 1024 * 1024;

#[derive(Default)]
pub struct Running(Mutex<Option<Arc<Server>>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInfo {
    enabled: bool,
    running: bool,
    url: String,
    error: Option<String>,
}

fn url() -> String {
    format!("http://127.0.0.1:{PORT}/mcp")
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_USER).map_err(|e| format!("The system keychain is not available: {e}"))
}

fn new_token() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    let t: String = b.iter().map(|x| format!("{x:02x}")).collect();
    entry()?.set_password(&t).map_err(|e| format!("Could not save the token in the keychain: {e}"))?;
    Ok(t)
}

fn token() -> Result<String, String> {
    match entry()?.get_password() {
        Ok(t) if !t.is_empty() => Ok(t),
        _ => new_token(),
    }
}

pub fn start(app: &AppHandle) -> Result<(), String> {
    stop(app);
    let token = token()?;
    let server = Arc::new(Server::http(("127.0.0.1", PORT)).map_err(|e| format!("Could not open port {PORT}: {e}"))?);
    *app.state::<Running>().0.lock().unwrap() = Some(server.clone());
    let app = app.clone();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            serve(&app, &token, req);
        }
    });
    crate::log::write("info", &format!("MCP server listening on {}", url()));
    Ok(())
}

pub fn stop(app: &AppHandle) {
    if let Some(s) = app.state::<Running>().0.lock().unwrap().take() {
        s.unblock();
    }
}

fn header<'a>(req: &'a Request, name: &'static str) -> Option<&'a str> {
    req.headers().iter().find(|h| h.field.equiv(name)).map(|h| h.value.as_str())
}

fn json_header() -> Header {
    Header::from_bytes("Content-Type", "application/json").unwrap()
}

fn same_bytes(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn serve(app: &AppHandle, token: &str, mut req: Request) {
    let respond = |req: Request, code: u16, body: String| {
        let _ = req.respond(Response::from_string(body).with_status_code(code).with_header(json_header()));
    };
    if req.url().split('?').next() != Some("/mcp") {
        return respond(req, 404, r#"{"error":"Not found. The MCP address ends in /mcp."}"#.into());
    }
    // Web pages must not reach this server (DNS rebinding); desktop MCP clients send no Origin.
    if let Some(o) = header(&req, "Origin") {
        if !(o.starts_with("http://127.0.0.1") || o.starts_with("http://localhost")) {
            return respond(req, 403, r#"{"error":"Origin not allowed"}"#.into());
        }
    }
    let authed = header(&req, "Authorization").and_then(|h| h.strip_prefix("Bearer ")).is_some_and(|t| same_bytes(t.trim(), token));
    if !authed {
        let _ = req.respond(
            Response::from_string(r#"{"error":"Missing or wrong token. Copy it from Lute & Tavern Settings."}"#)
                .with_status_code(401)
                .with_header(json_header())
                .with_header(Header::from_bytes("WWW-Authenticate", "Bearer").unwrap()),
        );
        return;
    }
    if *req.method() != Method::Post {
        let _ = req.respond(Response::empty(405).with_header(Header::from_bytes("Allow", "POST").unwrap()));
        return;
    }
    let mut body = String::new();
    if req.as_reader().take(MAX_BODY).read_to_string(&mut body).is_err() {
        return respond(req, 400, r#"{"error":"Unreadable body"}"#.into());
    }
    let msg: serde_json::Value = match serde_json::from_str::<serde_json::Value>(&body) {
        Ok(v) if v.is_object() => v,
        _ => return respond(req, 400, r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Send one JSON-RPC message"}}"#.into()),
    };
    let state = app.state::<AppState>();
    let cfg = writer::config(&state);
    let cx = lt_core::mcp::Context { lib: &state.lib, writer_name: cfg.name, exclude: cfg.exclude, version: env!("CARGO_PKG_VERSION") };
    let out = lt_core::mcp::handle(&cx, &msg);
    if let Some(id) = out.changed {
        let _ = app.emit("song-updated", id);
    }
    match out.reply {
        Some(r) => respond(req, 200, r.to_string()),
        None => {
            let _ = req.respond(Response::empty(202));
        }
    }
}

fn info(app: &AppHandle, error: Option<String>) -> McpInfo {
    let enabled = app.state::<AppState>().lib.setting_bool("mcpEnabled");
    let running = app.state::<Running>().0.lock().unwrap().is_some();
    McpInfo { enabled, running, url: url(), error }
}

#[tauri::command]
pub fn mcp_info(app: AppHandle) -> McpInfo {
    info(&app, None)
}

#[tauri::command]
pub fn mcp_set_enabled(app: AppHandle, state: State<'_, AppState>, enabled: bool) -> Result<McpInfo, String> {
    state.lib.set_setting("mcpEnabled", &serde_json::Value::Bool(enabled)).map_err(|e| e.to_string())?;
    if enabled {
        let err = start(&app).err();
        return Ok(info(&app, err));
    }
    stop(&app);
    Ok(info(&app, None))
}

/// Hands the token to the Settings page so the person can paste it into their AI app.
#[tauri::command]
pub fn mcp_token() -> Result<String, String> {
    token()
}

/// Replaces the token; apps using the old one lose access.
#[tauri::command]
pub fn mcp_new_token(app: AppHandle) -> Result<McpInfo, String> {
    new_token()?;
    let err = if app.state::<Running>().0.lock().unwrap().is_some() { start(&app).err() } else { None };
    Ok(info(&app, err))
}
