//! SQLite schema and migrations. The database lives in the app's data folder as `library.db`.

use anyhow::Result;
use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    // v1: library, songwriter songs, playlists, sheets, settings
    r#"
    CREATE TABLE songs (
        id          INTEGER PRIMARY KEY,
        source      TEXT NOT NULL CHECK (source IN ('library','ai')),
        title       TEXT NOT NULL,
        artist      TEXT NOT NULL DEFAULT '',
        album       TEXT NOT NULL DEFAULT '',
        year        TEXT NOT NULL DEFAULT '',
        genre       TEXT NOT NULL DEFAULT 'Unsorted',
        track       INTEGER,
        duration    REAL,
        path        TEXT UNIQUE,
        file_mtime  INTEGER,
        folder_id   INTEGER REFERENCES folders(id) ON DELETE CASCADE,
        art         TEXT,
        added_at    INTEGER NOT NULL,
        updated_at  INTEGER NOT NULL,
        -- songwriter fields (source = 'ai')
        request     TEXT,
        status      TEXT,
        sub_genre   TEXT NOT NULL DEFAULT '',
        song_key    TEXT NOT NULL DEFAULT '',
        bpm         INTEGER NOT NULL DEFAULT 0,
        style       TEXT NOT NULL DEFAULT '',
        exclude     TEXT NOT NULL DEFAULT '',
        lyrics      TEXT NOT NULL DEFAULT '',
        master_take INTEGER,
        -- sheet music written or imported in the app
        notes_json  TEXT NOT NULL DEFAULT '[]',
        chords_json TEXT NOT NULL DEFAULT '[]'
    );
    CREATE INDEX songs_genre ON songs(genre COLLATE NOCASE);
    CREATE TABLE folders (
        id   INTEGER PRIMARY KEY,
        path TEXT NOT NULL UNIQUE
    );
    CREATE TABLE takes (
        id       INTEGER PRIMARY KEY,
        song_id  INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        name     TEXT NOT NULL,
        path     TEXT NOT NULL,
        origin   TEXT NOT NULL,
        added_at INTEGER NOT NULL
    );
    CREATE TABLE versions (
        id         INTEGER PRIMARY KEY,
        song_id    INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        label      TEXT NOT NULL,
        note       TEXT NOT NULL,
        title      TEXT NOT NULL,
        lyrics     TEXT NOT NULL,
        style      TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE messages (
        id         INTEGER PRIMARY KEY,
        song_id    INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        role       TEXT NOT NULL CHECK (role IN ('user','writer')),
        text       TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE sheets (
        id       INTEGER PRIMARY KEY,
        song_id  INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        name     TEXT NOT NULL,
        kind     TEXT NOT NULL CHECK (kind IN ('pdf','image','musicxml')),
        path     TEXT NOT NULL,
        added_at INTEGER NOT NULL
    );
    CREATE TABLE playlists (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE playlist_songs (
        playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        song_id     INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        pos         INTEGER NOT NULL,
        PRIMARY KEY (playlist_id, song_id)
    );
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    // v2: genre tags people and the songwriter can create; defaults are seeded for quick picking
    r#"
    CREATE TABLE genres (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
        origin     TEXT NOT NULL CHECK (origin IN ('default','user','writer','file')),
        created_at INTEGER NOT NULL
    );
    INSERT INTO genres(name, origin, created_at)
    SELECT value, 'default', 0 FROM json_each('["Pop","Rock","Hip-Hop","R&B","Soul","Country","Folk","Indie","Alternative","Electronic","Dance","Jazz","Blues","Classical","Metal","Punk","Reggae","Latin","World","Ambient","Lo-Fi","Soundtrack","Acoustic","Singer-Songwriter","Kids"]');
    INSERT OR IGNORE INTO genres(name, origin, created_at)
    SELECT DISTINCT genre, 'file', 0 FROM songs WHERE genre <> 'Unsorted';
    "#,
    // v3: stems. Parts of one song (vocals, drums...) attach to that song instead of becoming songs.
    // A song made only of stems has no path and is identified by its stem_group (folder + title).
    r#"
    ALTER TABLE songs ADD COLUMN stem_group TEXT;
    CREATE UNIQUE INDEX songs_stem_group ON songs(stem_group) WHERE stem_group IS NOT NULL;
    CREATE TABLE stems (
        id         INTEGER PRIMARY KEY,
        song_id    INTEGER NOT NULL REFERENCES songs(id) ON DELETE CASCADE,
        name       TEXT NOT NULL,
        path       TEXT NOT NULL UNIQUE,
        file_mtime INTEGER,
        folder_id  INTEGER REFERENCES folders(id) ON DELETE CASCADE
    );
    CREATE INDEX stems_song ON stems(song_id);
    "#,
    // v4: the managed library folder, and each song's own folder inside it.
    r#"
    ALTER TABLE folders ADD COLUMN managed INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE songs ADD COLUMN song_dir TEXT;
    CREATE INDEX songs_song_dir ON songs(song_dir);
    "#,
    // v5: songs that stream from a service (Apple Music) instead of a file.
    r#"
    ALTER TABLE songs ADD COLUMN provider TEXT NOT NULL DEFAULT 'local';
    ALTER TABLE songs ADD COLUMN provider_id TEXT;
    ALTER TABLE songs ADD COLUMN art_url TEXT;
    CREATE UNIQUE INDEX songs_provider_id ON songs(provider, provider_id) WHERE provider_id IS NOT NULL;
    "#,
    // v6: takes belong to the lyrics version they were made from; songs no longer get "finished".
    r#"
    ALTER TABLE takes ADD COLUMN version_id INTEGER REFERENCES versions(id) ON DELETE SET NULL;
    UPDATE takes SET version_id = (
        SELECT v.id FROM versions v WHERE v.song_id = takes.song_id AND v.created_at <= takes.added_at ORDER BY v.id DESC LIMIT 1);
    UPDATE songs SET status = 'ready' WHERE status IN ('takes', 'done');
    "#,
    // v7: the in-app assistant's conversation (each row is one chat message as JSON).
    r#"
    CREATE TABLE assistant_messages (
        id         INTEGER PRIMARY KEY,
        body       TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    "#,
    // v8: internet radio stations the person saved or played, and connections to their own music servers.
    r#"
    CREATE TABLE stations (
        uuid        TEXT PRIMARY KEY,
        name        TEXT NOT NULL,
        url         TEXT NOT NULL,
        homepage    TEXT NOT NULL DEFAULT '',
        favicon     TEXT NOT NULL DEFAULT '',
        country     TEXT NOT NULL DEFAULT '',
        tags        TEXT NOT NULL DEFAULT '',
        codec       TEXT NOT NULL DEFAULT '',
        bitrate     INTEGER NOT NULL DEFAULT 0,
        favorite    INTEGER NOT NULL DEFAULT 0,
        last_played INTEGER
    );
    CREATE TABLE connections (
        id         INTEGER PRIMARY KEY,
        kind       TEXT NOT NULL,
        name       TEXT NOT NULL,
        base_url   TEXT NOT NULL,
        username   TEXT NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL
    );
    "#,
    // v9: per-kind connection details that aren't secret (Plex server id and addresses, Jellyfin user id...).
    r#"
    ALTER TABLE connections ADD COLUMN extra TEXT NOT NULL DEFAULT '{}';
    "#,
    // v10: what the backup has already put on each server (path inside the library folder, size, mtime).
    r#"
    CREATE TABLE backup_files (
        conn_id     INTEGER NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
        rel         TEXT NOT NULL,
        size        INTEGER NOT NULL,
        mtime       INTEGER NOT NULL,
        uploaded_at INTEGER NOT NULL,
        PRIMARY KEY (conn_id, rel)
    );
    "#,
    // v11: playlists that follow the person across devices: a lasting id, a change time, and deletions.
    r#"
    ALTER TABLE playlists ADD COLUMN uuid TEXT;
    ALTER TABLE playlists ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
    CREATE TABLE playlist_tombstones (
        uuid       TEXT PRIMARY KEY,
        name       TEXT NOT NULL,
        deleted_at INTEGER NOT NULL
    );
    "#,
    // v12: songs brought in from a streaming service's library (e.g. "apple-library"), so they can leave
    // again when the person removes them there; songs they saved by hand stay.
    r#"
    ALTER TABLE songs ADD COLUMN synced_from TEXT;
    "#,
    // v13: a song whose file can't be found is kept (greyed out) instead of deleted, and comes back when the
    // file does; tags the person edited in the app are no longer overwritten by the file's own tags.
    r#"
    ALTER TABLE songs ADD COLUMN missing_since INTEGER;
    ALTER TABLE songs ADD COLUMN tags_edited INTEGER NOT NULL DEFAULT 0;
    -- songs of a playlist from another device that aren't on this one (yet): kept in place and sent back
    CREATE TABLE playlist_pending (
        playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        after_song  INTEGER,
        seq         INTEGER NOT NULL,
        entry       TEXT NOT NULL
    );
    "#,
];

pub fn open(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<()> {
    let current: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        conn.execute_batch(&format!(
            "BEGIN; {sql} PRAGMA user_version = {}; COMMIT;",
            i + 1
        ))?;
    }
    Ok(())
}
