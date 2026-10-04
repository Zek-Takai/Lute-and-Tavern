//! Jellyfin and Emby (they share an API). Signs in once with the username and password to get an access
//! token; only the token is kept (in the keychain). Audio is the original file.

use crate::remote::{Connection, MusicServer, RemoteItem, RemoteSearch, Target, get_json, http, net, text};
use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

fn auth_header(device: &str, token: Option<&str>) -> String {
    let mut h = format!(
        "MediaBrowser Client=\"Lute & Tavern\", Device=\"Lute & Tavern\", DeviceId=\"{device}\", Version=\"{}\"",
        env!("CARGO_PKG_VERSION")
    );
    if let Some(t) = token {
        h.push_str(&format!(", Token=\"{t}\""));
    }
    h
}

fn headers(device: &str, token: Option<&str>) -> Vec<(String, String)> {
    let h = auth_header(device, token);
    let mut v = vec![("Authorization".to_string(), h.clone()), ("X-Emby-Authorization".to_string(), h)];
    if let Some(t) = token {
        v.push(("X-Emby-Token".to_string(), t.to_string()));
    }
    v
}

/// Signs in; returns (access token, details to keep with the connection, server description).
pub fn sign_in(base_url: &str, username: &str, password: &str, device: &str) -> Result<(String, Value, String)> {
    let mut req = http()?.post(format!("{base_url}/Users/AuthenticateByName")).header("Content-Type", "application/json").header("Accept", "application/json");
    for (k, v) in headers(device, None) {
        req = req.header(k, v);
    }
    let res = req.json(&json!({"Username": username, "Pw": password})).send().map_err(|e| anyhow!("Could not reach {base_url}: {}", e.without_url()))?;
    if res.status().as_u16() == 401 {
        bail!("The username or password was not accepted");
    }
    if !res.status().is_success() {
        bail!("{base_url} answered {} (is this a Jellyfin or Emby server?)", res.status());
    }
    let v: Value = res.json().map_err(net)?;
    let token = text(&v, "AccessToken");
    let user = text(&v["User"], "Id");
    if token.is_empty() || user.is_empty() {
        bail!("{base_url} didn't send back a sign-in");
    }
    let info = get_json(&format!("{base_url}/System/Info"), &headers(device, Some(&token)), base_url).unwrap_or(Value::Null);
    let product = Some(text(&info, "ProductName")).filter(|p| !p.is_empty()).unwrap_or_else(|| "Jellyfin/Emby".into());
    Ok((token, json!({"user": user, "device": device}), format!("{product} {}", text(&info, "Version")).trim().to_string()))
}

pub struct Jellyfin<'a> {
    pub conn: &'a Connection,
    pub token: &'a str,
}

const FIELDS: &str = "Fields=Genres,ChildCount,ProductionYear,AlbumArtist,DateCreated&EnableImageTypes=Primary";

impl Jellyfin<'_> {
    fn user(&self) -> &str {
        self.conn.extra["user"].as_str().unwrap_or("")
    }

    fn headers(&self) -> Vec<(String, String)> {
        headers(self.conn.extra["device"].as_str().unwrap_or("lute-and-tavern"), Some(self.token))
    }

    fn get(&self, path: &str) -> Result<Value> {
        get_json(&format!("{}{path}", self.conn.base_url), &self.headers(), &self.conn.name)
    }

    fn items(&self, query: &str) -> Result<Vec<RemoteItem>> {
        Ok(items(&self.get(&format!("/Users/{}/Items?{query}&{FIELDS}", self.user()))?["Items"]))
    }

    fn playlists(&self) -> Result<Vec<RemoteItem>> {
        let v = self.get(&format!("/Users/{}/Items?IncludeItemTypes=Playlist&Recursive=true&{FIELDS}&Fields=MediaType", self.user()))?;
        Ok(v["Items"].as_array().into_iter().flatten().filter(|p| p["MediaType"] == "Audio" || p["MediaType"].is_null()).map(item).collect())
    }
}

fn item(v: &Value) -> RemoteItem {
    let kind = match v["Type"].as_str().unwrap_or("") {
        "Audio" => "song",
        "MusicAlbum" => "album",
        "MusicArtist" => "artist",
        "Playlist" => "playlist",
        _ => "",
    };
    // the item's own picture, else its album's
    let cover = if v["ImageTags"]["Primary"].is_string() { Some(text(v, "Id")) } else if v["AlbumPrimaryImageTag"].is_string() { Some(text(v, "AlbumId")) } else { None };
    let artist = Some(text(v, "AlbumArtist")).filter(|a| !a.is_empty() && kind == "album").or_else(|| v["Artists"][0].as_str().map(String::from)).unwrap_or_default();
    RemoteItem {
        id: text(v, "Id"),
        kind: kind.into(),
        name: text(v, "Name"),
        artist,
        album: text(v, "Album"),
        year: text(v, "ProductionYear"),
        genre: v["Genres"][0].as_str().unwrap_or("").to_string(),
        cover,
        duration: v["RunTimeTicks"].as_f64().map(|t| t / 10_000_000.0),
        track: v["IndexNumber"].as_i64().filter(|_| kind == "song"),
        count: v["ChildCount"].as_i64().or(v["SongCount"].as_i64()),
        added: v["DateCreated"].as_str().and_then(crate::remote::iso_time),
    }
}

fn items(v: &Value) -> Vec<RemoteItem> {
    v.as_array().into_iter().flatten().map(item).filter(|i| !i.kind.is_empty()).collect()
}

fn enc(s: &str) -> String {
    reqwest::Url::parse_with_params("https://x/?", &[("v", s)]).ok().and_then(|u| u.query().map(|q| q[2..].to_string())).unwrap_or_default()
}

impl MusicServer for Jellyfin<'_> {
    fn ping(&self) -> Result<String> {
        let v = self.get("/System/Info")?;
        Ok(format!("{} {}", text(&v, "ProductName"), text(&v, "Version")).trim().to_string())
    }

    fn search(&self, raw: &str) -> Result<RemoteSearch> {
        let q = enc(raw);
        let found = self.items(&format!("searchTerm={q}&IncludeItemTypes=Audio,MusicAlbum&Recursive=true&Limit=40"))?;
        let artists = items(&self.get(&format!("/Artists?searchTerm={q}&userId={}&Limit=8&{FIELDS}", self.user()))?["Items"]);
        let ql = raw.trim().to_lowercase();
        Ok(RemoteSearch {
            songs: found.iter().filter(|i| i.kind == "song").cloned().collect(),
            albums: found.into_iter().filter(|i| i.kind == "album").collect(),
            artists: artists.into_iter().map(|a| RemoteItem { kind: "artist".into(), ..a }).collect(),
            playlists: self.playlists()?.into_iter().filter(|p| p.name.to_lowercase().contains(&ql)).collect(),
            folders: vec![],
        })
    }

    fn home(&self) -> Result<RemoteSearch> {
        let albums = self.items("IncludeItemTypes=MusicAlbum&SortBy=DateCreated&SortOrder=Descending&Recursive=true&Limit=30")?;
        Ok(RemoteSearch { playlists: self.playlists()?, albums, ..Default::default() })
    }

    fn open(&self, kind: &str, id: &str) -> Result<Vec<RemoteItem>> {
        match kind {
            "album" => self.items(&format!("ParentId={id}&IncludeItemTypes=Audio&SortBy=ParentIndexNumber,IndexNumber,SortName")),
            "artist" => self.items(&format!("ArtistIds={id}&IncludeItemTypes=MusicAlbum&Recursive=true&SortBy=ProductionYear,SortName")),
            "playlist" => Ok(items(&self.get(&format!("/Playlists/{id}/Items?UserId={}&{FIELDS}", self.user()))?["Items"])),
            _ => bail!("Can't open a {kind}"),
        }
    }

    fn library_stamp(&self) -> Result<Option<String>> {
        // How many songs, and when the newest change was saved.
        let v = self.get(&format!("/Users/{}/Items?IncludeItemTypes=Audio&Recursive=true&Limit=1&SortBy=DateLastSaved&SortOrder=Descending&Fields=DateLastSaved", self.user()))?;
        Ok(Some(format!("{}:{}", v["TotalRecordCount"], text(&v["Items"][0], "DateLastSaved"))))
    }

    fn all_songs(&self) -> Result<Vec<RemoteItem>> {
        let mut out = Vec::new();
        let mut start = 0usize;
        loop {
            let page = self.items(&format!("IncludeItemTypes=Audio&Recursive=true&SortBy=SortName&StartIndex={start}&Limit=1000"))?;
            start += page.len();
            let done = page.len() < 1000;
            out.extend(page);
            if done {
                break;
            }
        }
        Ok(out)
    }

    fn stream(&self, song_id: &str) -> Result<Target> {
        Ok(Target { url: format!("{}/Audio/{song_id}/stream?static=true", self.conn.base_url), headers: self.headers() })
    }

    fn cover(&self, cover: &str, size: u32) -> Option<Target> {
        Some(Target { url: format!("{}/Items/{cover}/Images/Primary?maxHeight={size}&maxWidth={size}&quality=90", self.conn.base_url), headers: self.headers() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_map_songs_albums_and_their_covers() {
        let song = item(&json!({"Id": "s1", "Type": "Audio", "Name": "Numb", "Artists": ["Linkin Park"], "Album": "Meteora", "AlbumId": "a1",
                                "AlbumPrimaryImageTag": "x", "RunTimeTicks": 1_870_000_000i64, "IndexNumber": 13}));
        assert_eq!((song.kind.as_str(), song.artist.as_str(), song.cover.as_deref(), song.duration, song.track), ("song", "Linkin Park", Some("a1"), Some(187.0), Some(13)));
        let album = item(&json!({"Id": "a1", "Type": "MusicAlbum", "Name": "Meteora", "AlbumArtist": "Linkin Park", "ImageTags": {"Primary": "t"}, "ChildCount": 13}));
        assert_eq!((album.kind.as_str(), album.artist.as_str(), album.cover.as_deref(), album.count), ("album", "Linkin Park", Some("a1"), Some(13)));
        assert!(item(&json!({"Type": "Movie"})).kind.is_empty(), "video is ignored");
        assert!(auth_header("d", Some("t")).ends_with("Token=\"t\""));
    }

    #[test]
    fn playlist_search_matches_accented_names() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let body = if req.url().contains("IncludeItemTypes=Playlist") {
                    json!({"Items": [{"Id": "p1", "Type": "Playlist", "Name": "Café Mix", "MediaType": "Audio"}, {"Id": "p2", "Type": "Playlist", "Name": "Road", "MediaType": "Audio"}]})
                } else {
                    json!({"Items": []})
                };
                let _ = req.respond(tiny_http::Response::from_string(body.to_string()));
            }
        });
        let c = Connection { id: 1, kind: "jellyfin".into(), name: "J".into(), base_url: format!("http://127.0.0.1:{port}"), username: String::new(), extra: json!({"user": "u"}) };
        let j = Jellyfin { conn: &c, token: "t" };
        let found = j.search("CAFÉ").unwrap().playlists;
        assert_eq!(found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Café Mix"]);
    }
}
