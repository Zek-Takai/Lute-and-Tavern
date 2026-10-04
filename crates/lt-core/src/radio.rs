//! Internet radio through Radio Browser (radio-browser.info): a free, open, community directory of about
//! 50,000 stations, most of them the online streams of real FM/AM stations. No account or key. Stations the
//! person saves or plays are kept in the library database.

use crate::library::Library;
use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const AGENT: &str = concat!("LuteAndTavern/", env!("CARGO_PKG_VERSION"));
/// Mirrors tried in order when the server list can't be fetched.
const FALLBACK: &[&str] = &["de1.api.radio-browser.info", "fi1.api.radio-browser.info", "nl1.api.radio-browser.info"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    pub uuid: String,
    pub name: String,
    /// The stream address (already resolved from any .pls/.m3u wrapper by Radio Browser)
    pub url: String,
    pub homepage: String,
    pub favicon: String,
    pub country: String,
    /// Comma-separated genres
    pub tags: String,
    pub codec: String,
    pub bitrate: i64,
    #[serde(default)]
    pub favorite: bool,
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder().user_agent(AGENT).timeout(Duration::from_secs(15)).build()?)
}

fn servers() -> Vec<String> {
    let mut list: Vec<String> = client()
        .ok()
        .and_then(|c| c.get("https://all.api.radio-browser.info/json/servers").send().ok())
        .and_then(|r| r.json::<Vec<Value>>().ok())
        .map(|v| v.iter().filter_map(|s| s["name"].as_str().map(String::from)).collect())
        .unwrap_or_default();
    list.sort();
    list.dedup();
    if list.is_empty() {
        list = FALLBACK.iter().map(|s| s.to_string()).collect();
    }
    list
}

fn get(path: &str, query: &[(&str, String)]) -> Result<Vec<Value>> {
    let c = client()?;
    let mut last = None;
    for host in servers() {
        let mut url = reqwest::Url::parse(&format!("https://{host}{path}"))?;
        url.query_pairs_mut().extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        match c.get(url).send().and_then(|r| r.error_for_status()) {
            Ok(r) => return r.json::<Vec<Value>>().map_err(|e| e.without_url()).context("Radio Browser sent something unexpected"),
            Err(e) => last = Some(e),
        }
    }
    bail!("Could not reach the radio directory: {}", last.map(|e| e.without_url().to_string()).unwrap_or_default())
}

fn station(v: &Value) -> Option<Station> {
    let s = |k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let url = Some(s("url_resolved")).filter(|u| !u.is_empty()).unwrap_or_else(|| s("url"));
    if url.is_empty() || v["lastcheckok"].as_i64() == Some(0) {
        return None;
    }
    Some(Station {
        uuid: s("stationuuid"),
        name: s("name"),
        url,
        homepage: s("homepage"),
        favicon: s("favicon"),
        country: s("country"),
        tags: s("tags"),
        codec: s("codec"),
        bitrate: v["bitrate"].as_i64().unwrap_or(0),
        favorite: false,
    })
}

/// Searches the directory. Any of name, genre tag and two-letter country code may be empty.
pub fn search(name: &str, tag: &str, country: &str, limit: usize) -> Result<Vec<Station>> {
    let mut q = vec![
        ("limit", limit.clamp(1, 200).to_string()),
        ("hidebroken", "true".into()),
        ("order", "votes".into()),
        ("reverse", "true".into()),
    ];
    for (k, v) in [("name", name), ("tag", tag), ("countrycode", country)] {
        if !v.trim().is_empty() {
            q.push((k, v.trim().to_string()));
        }
    }
    Ok(get("/json/stations/search", &q)?.iter().filter_map(station).collect())
}

/// The most-liked stations, optionally in one country.
pub fn top(country: &str, limit: usize) -> Result<Vec<Station>> {
    search("", "", country, limit)
}

/// The most used genre tags, for browsing.
pub fn top_tags(limit: usize) -> Result<Vec<(String, i64)>> {
    let q = [("order", "stationcount".to_string()), ("reverse", "true".into()), ("limit", limit.to_string()), ("hidebroken", "true".into())];
    Ok(get("/json/tags", &q)?
        .iter()
        .filter_map(|t| Some((t["name"].as_str()?.to_string(), t["stationcount"].as_i64().unwrap_or(0))))
        .collect())
}

/// Tells the directory a station was played (its etiquette asks for this; it keeps popularity honest).
pub fn count_click(uuid: &str) {
    let uuid = uuid.to_string();
    std::thread::spawn(move || {
        if let Some(host) = servers().into_iter().next() {
            let _ = client().and_then(|c| Ok(c.get(format!("https://{host}/json/url/{uuid}")).send()?));
        }
    });
}

const COLS: &str = "uuid, name, url, homepage, favicon, country, tags, codec, bitrate, favorite";

fn from_row(r: &rusqlite::Row) -> rusqlite::Result<Station> {
    Ok(Station {
        uuid: r.get(0)?,
        name: r.get(1)?,
        url: r.get(2)?,
        homepage: r.get(3)?,
        favicon: r.get(4)?,
        country: r.get(5)?,
        tags: r.get(6)?,
        codec: r.get(7)?,
        bitrate: r.get(8)?,
        favorite: r.get::<_, i64>(9)? != 0,
    })
}

impl Library {
    fn upsert_station(&self, s: &Station) -> Result<()> {
        self.db().execute(
            "INSERT INTO stations(uuid, name, url, homepage, favicon, country, tags, codec, bitrate) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT(uuid) DO UPDATE SET name = excluded.name, url = excluded.url, homepage = excluded.homepage, favicon = excluded.favicon, \
             country = excluded.country, tags = excluded.tags, codec = excluded.codec, bitrate = excluded.bitrate",
            params![s.uuid, s.name, s.url, s.homepage, s.favicon, s.country, s.tags, s.codec, s.bitrate],
        )?;
        Ok(())
    }

    pub fn set_station_favorite(&self, s: &Station, on: bool) -> Result<()> {
        self.upsert_station(s)?;
        self.db().execute("UPDATE stations SET favorite = ?2 WHERE uuid = ?1", params![s.uuid, on as i64])?;
        Ok(())
    }

    pub fn station_played(&self, s: &Station) -> Result<()> {
        self.upsert_station(s)?;
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.db().execute("UPDATE stations SET last_played = ?2 WHERE uuid = ?1", params![s.uuid, t])?;
        Ok(())
    }

    pub fn favorite_stations(&self) -> Result<Vec<Station>> {
        let db = self.db();
        let mut st = db.prepare(&format!("SELECT {COLS} FROM stations WHERE favorite = 1 ORDER BY name COLLATE NOCASE"))?;
        Ok(st.query_map([], from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn recent_stations(&self, limit: usize) -> Result<Vec<Station>> {
        let db = self.db();
        let mut st = db.prepare(&format!("SELECT {COLS} FROM stations WHERE last_played IS NOT NULL ORDER BY last_played DESC LIMIT ?1"))?;
        Ok(st.query_map([limit as i64], from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn station(&self, uuid: &str) -> Result<Option<Station>> {
        Ok(self.db().query_row(&format!("SELECT {COLS} FROM stations WHERE uuid = ?1"), [uuid], from_row).optional()?)
    }

    /// Marks which of `list` are favorites.
    pub fn mark_favorites(&self, mut list: Vec<Station>) -> Vec<Station> {
        let favs: std::collections::HashSet<String> = self.favorite_stations().unwrap_or_default().into_iter().map(|s| s.uuid).collect();
        for s in &mut list {
            s.favorite = favs.contains(&s.uuid);
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stations_are_saved_as_favorites_and_recents() {
        let d = tempfile::tempdir().unwrap();
        let l = Library::open(d.path()).unwrap();
        let s = Station { uuid: "u1".into(), name: "Jazz FM".into(), url: "http://x/stream".into(), ..Default::default() };
        l.station_played(&s).unwrap();
        assert_eq!(l.recent_stations(10).unwrap().len(), 1);
        assert!(l.favorite_stations().unwrap().is_empty());
        l.set_station_favorite(&s, true).unwrap();
        assert_eq!(l.favorite_stations().unwrap()[0].name, "Jazz FM");
        assert!(l.mark_favorites(vec![s.clone()])[0].favorite);
        l.set_station_favorite(&s, false).unwrap();
        assert!(l.favorite_stations().unwrap().is_empty());
    }

    #[test]
    fn directory_rows_become_stations() {
        let v: Value = serde_json::json!({"stationuuid": "a", "name": " KEXP ", "url": "http://x.pls", "url_resolved": "https://kexp/stream", "tags": "indie", "bitrate": 128, "lastcheckok": 1});
        let s = station(&v).unwrap();
        assert_eq!((s.name.as_str(), s.url.as_str(), s.bitrate), ("KEXP", "https://kexp/stream", 128));
        assert!(station(&serde_json::json!({"stationuuid": "b", "url": "http://x", "lastcheckok": 0})).is_none(), "broken stations are skipped");
    }
}
