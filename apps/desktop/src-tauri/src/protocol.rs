//! `lt://` serves library files to the interface: `/audio/<song>`, `/take/<take>`, `/art/<file>`, `/sheet/<sheet>`.
//! Audio supports byte ranges so seeking works, and every response allows cross-origin reads
//! so the visualizer's Web Audio analyser can hear the music.

use crate::AppState;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, UriSchemeContext, UriSchemeResponder};

const MAX_CHUNK: u64 = 4 * 1024 * 1024;

pub fn handle<R: tauri::Runtime>(ctx: UriSchemeContext<'_, R>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let res = match resolve(&app, request.uri().path()) {
            Some(path) => serve(&path, request.headers().get(header::RANGE).and_then(|v| v.to_str().ok())),
            None => plain(StatusCode::NOT_FOUND, "not found"),
        };
        responder.respond(res);
    });
}

fn resolve<R: tauri::Runtime>(app: &tauri::AppHandle<R>, raw_path: &str) -> Option<PathBuf> {
    let path = percent_encoding::percent_decode_str(raw_path).decode_utf8_lossy().to_string();
    let mut parts = path.trim_start_matches('/').splitn(2, '/');
    let (kind, arg) = (parts.next()?, parts.next()?);
    let lib = &app.state::<AppState>().lib;
    let id = || arg.parse::<i64>().ok();
    match kind {
        "audio" => lib.audio_path(id()?).ok()?,
        "take" => lib.take_path(id()?).ok()?,
        "sheet" => lib.sheet_path(id()?).ok()?,
        "art" => lib.art_path(arg),
        _ => None,
    }
}

fn serve(path: &PathBuf, range: Option<&str>) -> Response<Vec<u8>> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return plain(StatusCode::NOT_FOUND, "file is missing");
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
    let base = || {
        Response::builder()
            .header(header::CONTENT_TYPE, &mime)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::CACHE_CONTROL, "no-cache")
    };
    if let Some((start, end)) = range.and_then(|r| parse_range(r, len)) {
        let end = end.min(start + MAX_CHUNK - 1);
        let mut buf = vec![0; (end - start + 1) as usize];
        if file.seek(SeekFrom::Start(start)).and_then(|_| file.read_exact(&mut buf)).is_err() {
            return plain(StatusCode::RANGE_NOT_SATISFIABLE, "bad range");
        }
        return base()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
            .header(header::CONTENT_LENGTH, buf.len())
            .body(buf)
            .unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "response error"));
    }
    let mut buf = Vec::with_capacity(len as usize);
    if file.read_to_end(&mut buf).is_err() {
        return plain(StatusCode::INTERNAL_SERVER_ERROR, "could not read file");
    }
    base().status(StatusCode::OK).header(header::CONTENT_LENGTH, buf.len()).body(buf).unwrap_or_else(|_| plain(StatusCode::INTERNAL_SERVER_ERROR, "response error"))
}

/// Parses "bytes=a-b", "bytes=a-" and "bytes=-n" into an inclusive range inside the file.
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    let (start, end) = match (a.parse::<u64>().ok(), b.parse::<u64>().ok()) {
        (Some(s), Some(e)) => (s, e.min(len - 1)),
        (Some(s), None) => (s, len - 1),
        (None, Some(n)) => (len.saturating_sub(n), len - 1),
        (None, None) => return None,
    };
    (start <= end && start < len).then_some((start, end))
}

fn plain(status: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(msg.as_bytes().to_vec())
        .expect("static response")
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=2000-", 1000), None);
    }
}
