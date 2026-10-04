//! Local stream relay. Radio stations and connected music servers are played through
//! http://127.0.0.1:<port>/r/<secret>/<address>, which lets the player route them through the visualizer
//! (browsers silence outside audio that lacks CORS headers), plays old http:// stations, reads the song
//! titles stations send (ICY metadata), and keeps server sign-in details out of the web page. Only this
//! computer can reach it, and only with the per-launch secret.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use std::io::Read;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use tiny_http::{Header, Request, Response, Server, StatusCode};

struct Relay {
    port: u16,
    secret: String,
}

static RELAY: OnceLock<Relay> = OnceLock::new();
/// Addresses that need sign-in headers (connected servers), by short key, with when each was last used; the
/// page only ever sees the key. Kept to MAX_TARGETS, least recently used dropped first.
static TARGETS: Mutex<Option<HashMap<String, (Instant, lt_core::remote::Target)>>> = Mutex::new(None);
const MAX_TARGETS: usize = 4000;
/// A stalled server (lost Wi-Fi, a station that died without closing) ends the stream after this long with
/// no data, so its thread doesn't wait forever.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

pub fn start(app: &AppHandle) -> Result<(), String> {
    let server = Server::http("127.0.0.1:0").map_err(|e| format!("stream relay: {e}"))?;
    let port = server.server_addr().to_ip().map(|a| a.port()).ok_or("stream relay: no port")?;
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    let secret: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let _ = RELAY.set(Relay { port, secret });
    let app = app.clone();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let app = app.clone();
            // Streams run for as long as someone listens, so each gets its own thread.
            std::thread::spawn(move || serve(&app, req));
        }
    });
    Ok(())
}

/// The relay address prefix; the interface appends the base64url (no padding) of the real address.
pub fn base() -> String {
    RELAY.get().map(|r| format!("http://127.0.0.1:{}/r/{}/", r.port, r.secret)).unwrap_or_default()
}

pub fn url_for(target: &str) -> String {
    format!("{}{}", base(), URL_SAFE_NO_PAD.encode(target))
}

/// The address without the parts that change on every call: a Subsonic address carries a fresh salt and
/// token each time ("t" and "s"), so they are left out of the key.
fn stable_address(url: &str) -> String {
    let Ok(mut u) = reqwest::Url::parse(url) else { return url.to_string() };
    let pairs: Vec<(String, String)> = u.query_pairs().into_owned().collect();
    if !(u.path().contains("/rest/") && pairs.iter().any(|(k, _)| k == "t") && pairs.iter().any(|(k, _)| k == "s")) {
        return url.to_string();
    }
    u.query_pairs_mut().clear().extend_pairs(pairs.iter().filter(|(k, _)| k != "t" && k != "s"));
    u.to_string()
}

fn key_of(t: &lt_core::remote::Target) -> String {
    use md5::{Digest, Md5};
    let mut h = Md5::new();
    h.update(stable_address(&t.url).as_bytes());
    for (k, v) in &t.headers {
        h.update(k.as_bytes());
        h.update(v.as_bytes());
    }
    format!("{:x}", h.finalize())
}

/// Relay address for a connected server's file or cover. Always a short key, so neither sign-in headers nor
/// sign-in details inside the address ever reach the page. The same song or cover always gets the same key
/// (the newest signed address replaces the older one).
pub fn register(t: lt_core::remote::Target) -> String {
    let key = key_of(&t);
    {
        let mut guard = TARGETS.lock().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(key.clone(), (Instant::now(), t));
        if map.len() > MAX_TARGETS {
            let mut by_age: Vec<(Instant, String)> = map.iter().map(|(k, (at, _))| (*at, k.clone())).collect();
            by_age.sort();
            for (_, k) in by_age.into_iter().take(map.len() - MAX_TARGETS * 3 / 4) {
                map.remove(&k);
            }
        }
    }
    format!("{}k/{key}", base())
}

#[tauri::command]
pub fn relay_base() -> String {
    base()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Title {
    /// The relay address the title belongs to, so the interface knows which stream it is
    stream: String,
    title: String,
}

fn header(name: &str, value: &str) -> Option<Header> {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).ok()
}

fn fail(req: Request, code: u16, msg: &str) {
    let _ = req.respond(Response::from_string(msg).with_status_code(code));
}

fn serve(app: &AppHandle, req: Request) {
    let Some(relay) = RELAY.get() else { return };
    let path = req.url().split('?').next().unwrap_or("").to_string();
    let want_icy = req.url().contains("icy=1");
    let mut parts = path.trim_start_matches('/').splitn(3, '/');
    let (Some("r"), Some(secret), Some(enc)) = (parts.next(), parts.next(), parts.next()) else { return fail(req, 404, "not found") };
    if secret.len() != relay.secret.len() || secret.bytes().zip(relay.secret.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) != 0 {
        return fail(req, 403, "forbidden");
    }
    let (target, extra_headers) = if let Some(key) = enc.strip_prefix("k/") {
        let found = TARGETS.lock().unwrap().as_mut().and_then(|m| m.get_mut(key)).map(|(at, t)| {
            *at = Instant::now();
            t.clone()
        });
        match found {
            Some(t) => (t.url, t.headers),
            None => return fail(req, 404, "unknown stream"),
        }
    } else {
        match URL_SAFE_NO_PAD.decode(enc).ok().and_then(|b| String::from_utf8(b).ok()) {
            Some(t) => (t, vec![]),
            None => return fail(req, 400, "bad address"),
        }
    };
    let Ok(url) = reqwest::Url::parse(&target) else { return fail(req, 400, "bad address") };
    if !matches!(url.scheme(), "http" | "https") {
        return fail(req, 400, "only http(s) streams");
    }
    // A blocking client's timeout applies to each wait (the answer, then every read of the audio), not to
    // the whole stream, so a live station plays on while a stalled one ends after READ_TIMEOUT.
    let client = |signed: bool| {
        let b = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(READ_TIMEOUT)
            .tcp_keepalive(Duration::from_secs(15))
            .user_agent(concat!("LuteAndTavern/", env!("CARGO_PKG_VERSION")));
        // Sign-in headers (X-Plex-Token, X-Emby-Token, ...) stay on their own server: redirects elsewhere stop
        // here and are followed below without them.
        if signed { b.redirect(lt_core::remote::same_server_redirects()) } else { b }.build()
    };
    let range = req.headers().iter().find(|h| h.field.equiv("Range")).map(|r| r.value.as_str().to_string());
    let send = |to: &reqwest::Url, with_sign_in: bool| -> Result<reqwest::blocking::Response, String> {
        let c = client(with_sign_in).map_err(|e| e.without_url().to_string())?;
        let mut up = c.get(to.clone());
        if with_sign_in {
            for (k, v) in &extra_headers {
                up = up.header(k.as_str(), v.as_str());
            }
        }
        if let Some(r) = &range {
            up = up.header("Range", r.as_str());
        }
        if want_icy {
            up = up.header("Icy-MetaData", "1");
        }
        up.send().map_err(|e| e.without_url().to_string())
    };
    let signed = !extra_headers.is_empty();
    let mut res = match send(&url, signed) {
        Ok(r) => r,
        Err(e) => return fail(req, 502, &e),
    };
    if signed {
        if let Some(next) = lt_core::remote::cross_origin_redirect(&res) {
            res = match send(&next, false) {
                Ok(r) => r,
                Err(e) => return fail(req, 502, &e),
            };
        }
    }
    // relative addresses in a playlist are relative to where it was finally served from
    let url = res.url().clone();
    let code = res.status().as_u16();
    let ctype = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let get = |k: &str| res.headers().get(k).and_then(|v| v.to_str().ok()).map(String::from);
    let (length, range, metaint) = (get("content-length"), get("content-range"), get("icy-metaint").and_then(|m| m.trim().parse::<usize>().ok()));

    // HLS playlists: point every segment and sub-playlist back through the relay.
    if ctype.contains("mpegurl") || url.path().ends_with(".m3u8") {
        let text = res.text().unwrap_or_default();
        let out: String = text
            .lines()
            .map(|l| {
                let t = l.trim();
                if t.is_empty() || t.starts_with('#') {
                    if let Some(i) = t.find("URI=\"") {
                        let rest = &t[i + 5..];
                        if let Some(j) = rest.find('"') {
                            if let Ok(abs) = url.join(&rest[..j]) {
                                return format!("{}{}{}", &t[..i + 5], url_for(abs.as_str()), &rest[j..]);
                            }
                        }
                    }
                    l.to_string()
                } else {
                    url.join(t).map(|abs| url_for(abs.as_str())).unwrap_or_else(|_| l.to_string())
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut r = Response::from_string(out).with_status_code(StatusCode(code));
        for h in [header("Content-Type", "application/vnd.apple.mpegurl"), header("Access-Control-Allow-Origin", "*"), header("Cache-Control", "no-store")].into_iter().flatten() {
            r.add_header(h);
        }
        let _ = req.respond(r);
        return;
    }

    let mut headers: Vec<Header> = [
        header("Content-Type", if ctype.is_empty() { "audio/mpeg" } else { &ctype }),
        header("Access-Control-Allow-Origin", "*"),
        header("Cache-Control", "no-store"),
        header("Accept-Ranges", if range.is_some() || length.is_some() && metaint.is_none() { "bytes" } else { "none" }),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let Some(r) = &range {
        headers.extend(header("Content-Range", r));
    }
    let stream_id = req.url().to_string();
    let body: Box<dyn Read + Send> = match metaint {
        Some(every) if every > 0 => {
            let app = app.clone();
            Box::new(IcyReader::new(res, every, move |title| {
                let _ = app.emit("radio-title", Title { stream: stream_id.clone(), title });
            }))
        }
        _ => Box::new(res),
    };
    // A known length is passed on only for plain files (seeking in songs from a server); live streams are chunked.
    let len = if metaint.is_none() { length.and_then(|l| l.parse::<usize>().ok()) } else { None };
    let _ = req.respond(Response::new(StatusCode(code), headers, body, len, None));
}

/// Strips the song-title blocks a radio station mixes into its audio (ICY metadata), reporting each new title.
struct IcyReader<R, F> {
    inner: R,
    every: usize,
    left: usize,
    last: String,
    on_title: F,
}

impl<R: Read, F: FnMut(String)> IcyReader<R, F> {
    fn new(inner: R, every: usize, on_title: F) -> Self {
        Self { inner, every, left: every, last: String::new(), on_title }
    }
}

impl<R: Read, F: FnMut(String)> Read for IcyReader<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.left == 0 {
            let mut len = [0u8; 1];
            self.inner.read_exact(&mut len)?;
            let mut meta = vec![0u8; len[0] as usize * 16];
            self.inner.read_exact(&mut meta)?;
            let text = String::from_utf8_lossy(&meta);
            if let Some(start) = text.find("StreamTitle='") {
                let rest = &text[start + 13..];
                let title = rest.split("';").next().unwrap_or("").trim().to_string();
                if !title.is_empty() && title != self.last {
                    self.last = title.clone();
                    (self.on_title)(title);
                }
            }
            self.left = self.every;
        }
        let n = buf.len().min(self.left);
        let got = self.inner.read(&mut buf[..n])?;
        self.left -= got;
        Ok(got)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsonic_addresses_keep_one_key_and_old_ones_are_dropped() {
        let t = |salt: &str| lt_core::remote::Target::plain(format!("http://nas:4533/rest/getCoverArt.view?u=listener&t=tok{salt}&s={salt}&v=1.16.1&c=LuteAndTavern&f=json&id=al-1&size=300"));
        assert_eq!(key_of(&t("a")), key_of(&t("b")), "a fresh salt is the same cover");
        let other = lt_core::remote::Target::plain("http://nas:4533/rest/getCoverArt.view?u=listener&t=x&s=y&id=al-2&size=300".into());
        assert_ne!(key_of(&t("a")), key_of(&other));
        let plex = |tok: &str| lt_core::remote::Target { url: "http://nas:32400/library/parts/1/file.mp3?t=1&s=2".into(), headers: vec![("X-Plex-Token".into(), tok.into())] };
        assert_ne!(key_of(&plex("a")), key_of(&plex("b")), "only Subsonic sign-in parts are left out");
        for i in 0..MAX_TARGETS + 10 {
            register(lt_core::remote::Target::plain(format!("http://nas/cover/{i}")));
        }
        let len = TARGETS.lock().unwrap().as_ref().map(|m| m.len()).unwrap_or(0);
        assert!(len <= MAX_TARGETS, "{len}");
    }

    #[test]
    fn icy_titles_are_read_and_removed_from_the_audio() {
        let mut data = b"AAAA".to_vec();
        let meta = b"StreamTitle='Artist - Song';";
        let blocks = meta.len().div_ceil(16);
        data.push(blocks as u8);
        data.extend_from_slice(meta);
        data.resize(data.len() + blocks * 16 - meta.len(), 0);
        data.extend_from_slice(b"BBBB");
        data.push(0);
        data.extend_from_slice(b"CC");
        let mut titles = Vec::new();
        let mut out = Vec::new();
        IcyReader::new(&data[..], 4, |t| titles.push(t)).read_to_end(&mut out).unwrap();
        assert_eq!(out, b"AAAABBBBCC");
        assert_eq!(titles, ["Artist - Song"]);
    }
}
