/*
 * Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
 *
 * This file is part of the MirraNET project and is licensed under the
 * GNU Lesser General Public License v3.0 (LGPLv3).
 *
 * You may use, distribute and modify this code under the terms
 * of the LGPLv3 license. You should have received a copy of the
 * license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
 * or contact: niklas.linz@mirranet.de
 */

//! The SQLite database `freex.db`: users, sessions, login history, libraries, the library index
//! and watch state.
//!
//! The schema is versioned with `PRAGMA user_version`; [`MIGRATIONS`] are applied in order at
//! start and never edited afterwards. Every access is short, so one connection behind a mutex
//! serves the whole server.

use crate::auth::{now_ms, sha256};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// Schema changes, one per version. Append only.
const MIGRATIONS: &[&str] = &[
    // 1: accounts, sessions, libraries
    r#"
    CREATE TABLE users (
        id TEXT PRIMARY KEY,
        username TEXT NOT NULL UNIQUE COLLATE NOCASE,
        password_hash TEXT NOT NULL,
        admin INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        password_changed_at INTEGER NOT NULL,
        totp_secret TEXT,
        totp_enabled INTEGER NOT NULL DEFAULT 0,
        totp_last_step INTEGER NOT NULL DEFAULT 0,
        recovery_codes TEXT NOT NULL DEFAULT '[]',
        avatar_version INTEGER NOT NULL DEFAULT 0,
        all_libraries INTEGER NOT NULL DEFAULT 1,
        audio_language TEXT NOT NULL DEFAULT '',
        subtitle_mode TEXT NOT NULL DEFAULT 'foreign',
        subtitle_language TEXT NOT NULL DEFAULT '',
        ui_language TEXT NOT NULL DEFAULT '',
        unpinned TEXT NOT NULL DEFAULT '[]'
    );
    CREATE TABLE libraries (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        type TEXT NOT NULL,
        paths TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        last_scan INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE user_libraries (
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
        PRIMARY KEY (user_id, library_id)
    );
    CREATE TABLE sessions (
        id TEXT PRIMARY KEY,
        token_hash TEXT NOT NULL UNIQUE,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        created_at INTEGER NOT NULL,
        last_seen INTEGER NOT NULL,
        ip TEXT NOT NULL,
        user_agent TEXT NOT NULL
    );
    CREATE TABLE login_events (
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        time INTEGER NOT NULL,
        ip TEXT NOT NULL,
        user_agent TEXT NOT NULL,
        success INTEGER NOT NULL,
        detail TEXT NOT NULL
    );
    CREATE INDEX login_events_user ON login_events(user_id, time);
    "#,
    // 2: library index and watch state
    r#"
    CREATE TABLE items (
        id TEXT PRIMARY KEY,
        library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
        data TEXT NOT NULL
    );
    CREATE INDEX items_library ON items(library_id);
    CREATE TABLE progress (
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        item_id TEXT NOT NULL,
        offset_ms INTEGER NOT NULL DEFAULT 0,
        watched INTEGER NOT NULL DEFAULT 0,
        view_count INTEGER NOT NULL DEFAULT 0,
        updated_at INTEGER NOT NULL,
        PRIMARY KEY (user_id, item_id)
    );
    "#,
    // 3: watch history
    r#"
    CREATE TABLE watch_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        item_id TEXT NOT NULL,
        watched_at INTEGER NOT NULL,
        title TEXT NOT NULL,
        parent_title TEXT NOT NULL
    );
    CREATE INDEX watch_history_user ON watch_history(user_id, watched_at);
    CREATE INDEX watch_history_time ON watch_history(watched_at);
    "#,
    // 4: online metadata
    r#"
    CREATE TABLE metadata (
        item_id TEXT PRIMARY KEY,
        data TEXT NOT NULL
    );
    CREATE TABLE tmdb_collections (
        id INTEGER PRIMARY KEY,
        data TEXT NOT NULL
    );
    "#,
    // 5: own collections, playlists, watchlist
    r#"
    CREATE TABLE collections (
        id TEXT PRIMARY KEY,
        library_id TEXT NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        summary TEXT NOT NULL DEFAULT '',
        created_at INTEGER NOT NULL
    );
    CREATE TABLE collection_items (
        collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        item_id TEXT NOT NULL,
        PRIMARY KEY (collection_id, position)
    );
    CREATE INDEX collection_items_item ON collection_items(item_id);
    CREATE TABLE playlists (
        id TEXT PRIMARY KEY,
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE TABLE playlist_items (
        playlist_id TEXT NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
        position INTEGER NOT NULL,
        item_id TEXT NOT NULL,
        PRIMARY KEY (playlist_id, position)
    );
    CREATE TABLE watchlist (
        user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
        item_id TEXT NOT NULL,
        added_at INTEGER NOT NULL,
        PRIMARY KEY (user_id, item_id)
    );
    "#,
    // 6: intro and credits markers
    r#"
    CREATE TABLE markers (
        item_id TEXT PRIMARY KEY,
        file_modified INTEGER NOT NULL,
        intro_start INTEGER,
        intro_end INTEGER,
        credits_start INTEGER,
        credits_end INTEGER,
        analyzed_at INTEGER NOT NULL
    );
    "#,
    // 7: smart collections keep a library filter instead of a list of entries
    r#"
    ALTER TABLE collections ADD COLUMN filter TEXT NOT NULL DEFAULT '';
    "#,
    // 8: parental controls and switching users with a PIN
    r#"
    ALTER TABLE users ADD COLUMN max_age INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE users ADD COLUMN allow_unrated INTEGER NOT NULL DEFAULT 0;
    ALTER TABLE users ADD COLUMN pin_hash TEXT;
    "#,
    // 9: default quality per network
    r#"
    ALTER TABLE users ADD COLUMN quality_home TEXT NOT NULL DEFAULT 'auto';
    ALTER TABLE users ADD COLUMN quality_away TEXT NOT NULL DEFAULT 'auto';
    "#,
    // 10: how FreeX Web shows text subtitles
    r#"
    ALTER TABLE users ADD COLUMN subtitle_style TEXT NOT NULL DEFAULT '';
    "#,
    // 11: webhooks
    r#"
    CREATE TABLE webhooks (
        id TEXT PRIMARY KEY,
        url TEXT NOT NULL,
        events TEXT NOT NULL,
        secret TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    "#,
    // 12: metadata edited by hand
    r#"
    CREATE TABLE item_edits (
        item_id TEXT PRIMARY KEY,
        data TEXT NOT NULL
    );
    "#,
];

/// Login events kept per user.
const LOGIN_EVENTS_KEPT: i64 = 50;

pub struct Db {
    /// The one connection that writes.
    conn: Mutex<Connection>,
    /// Connections that only read. With WAL they run next to a write, so a long write (a scan
    /// storing a library) does not hold up the sign-in check and the pages of everyone else.
    readers: Vec<Mutex<Connection>>,
    next_reader: std::sync::atomic::AtomicUsize,
}

/// Read connections; few, since SQLite reads are short.
const READERS: usize = 4;

#[derive(Clone, Debug, Default)]
pub struct User {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub admin: bool,
    pub created_at: i64,
    pub password_changed_at: i64,
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub totp_last_step: i64,
    pub recovery_codes: Vec<String>,
    pub avatar_version: i64,
    pub all_libraries: bool,
    pub library_ids: Vec<String>,
    pub audio_language: String,
    pub subtitle_mode: String,
    pub subtitle_language: String,
    pub ui_language: String,
    pub unpinned: Vec<String>,
    /// Highest age rating the user may see; 0 = no limit. Not applied to admins.
    pub max_age: i32,
    /// With an age limit: whether titles without an age rating are shown.
    pub allow_unrated: bool,
    /// Hash of the PIN for switching to this account on a shared screen.
    pub pin_hash: Option<String>,
    /// Quality for playback in the home network and from outside: `auto` or a quality id.
    pub quality_home: String,
    pub quality_away: String,
    /// Subtitle look as `size=l&background=box&position=bottom&color=white` (see [`SUBTITLE_STYLE`]).
    pub subtitle_style: String,
}

/// The settings of the subtitle look and their values; the first value is the default.
pub const SUBTITLE_STYLE: [(&str, &[&str]); 4] = [
    ("size", &["m", "s", "l", "xl"]),
    ("background", &["box", "outline"]),
    ("position", &["bottom", "high"]),
    ("color", &["white", "yellow"]),
];

impl User {
    /// The subtitle look with every setting filled in.
    pub fn subtitle_style_json(&self) -> Value {
        let set: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(self.subtitle_style.as_bytes()).into_owned().collect();
        let mut m = serde_json::Map::new();
        for (key, values) in SUBTITLE_STYLE {
            let v = set.get(key).map(String::as_str).filter(|v| values.contains(v)).unwrap_or(values[0]);
            m.insert(key.to_string(), json!(v));
        }
        Value::Object(m)
    }
}

impl User {
    /// The account as the API shows it (no secrets).
    pub fn to_public(&self) -> Value {
        json!({
            "id": self.id,
            "username": self.username,
            "admin": self.admin,
            "createdAt": self.created_at,
            "twoFactor": self.totp_enabled,
            "recoveryCodesLeft": if self.totp_enabled { self.recovery_codes.len() } else { 0 },
            "passwordChangedAt": self.password_changed_at,
            "avatar": self.avatar_version,
            "maxAge": self.max_age,
            "allowUnrated": self.allow_unrated,
            "pin": self.pin_hash.is_some(),
            "libraries": if self.admin || self.all_libraries { json!("all") } else { json!(self.library_ids) },
            "preferences": {
                "audioLanguage": self.audio_language,
                "subtitleMode": self.subtitle_mode,
                "subtitleLanguage": self.subtitle_language,
                "uiLanguage": self.ui_language,
                "unpinned": self.unpinned,
                "qualityHome": self.quality_home,
                "qualityAway": self.quality_away,
                "subtitleStyle": self.subtitle_style_json(),
            },
        })
    }

    /// Whether the user may see a library: admins and users with all libraries see every one.
    pub fn may_access(&self, library_id: &str) -> bool {
        self.admin || self.all_libraries || self.library_ids.iter().any(|l| l == library_id)
    }

    /// The age limit that applies: none for admins.
    pub fn age_limit(&self) -> Option<i32> {
        (!self.admin && self.max_age > 0).then_some(self.max_age)
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub created_at: i64,
    pub last_seen: i64,
    pub ip: String,
    pub user_agent: String,
}

#[derive(Clone, Debug)]
pub struct LoginEvent {
    pub time: i64,
    pub ip: String,
    pub user_agent: String,
    pub success: bool,
    pub detail: String,
}

/// Watch state of one item for one user.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Progress {
    pub offset_ms: i64,
    pub watched: bool,
    pub view_count: i64,
    pub updated_at: i64,
}

impl Progress {
    pub fn to_json(self) -> Value {
        json!({"offsetMs": self.offset_ms, "watched": self.watched, "viewCount": self.view_count, "updatedAt": self.updated_at})
    }

    /// Started or finished.
    pub fn touched(self) -> bool {
        self.watched || self.offset_ms > 0
    }
}

/// A collection made by hand (movie series from TMDB are not stored, they are derived).
#[derive(Clone, Debug)]
pub struct OwnCollection {
    pub id: String,
    pub library_id: String,
    pub name: String,
    pub summary: String,
    pub item_ids: Vec<String>,
    pub created_at: i64,
    /// A smart collection's library filter (a query string, see `api::filter`); empty for a
    /// collection of chosen entries.
    pub filter: String,
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub item_ids: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One finished playback. Title and parent title are a snapshot, so the entry stays readable
/// when the item later leaves the library.
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub id: i64,
    pub user_id: String,
    pub item_id: String,
    pub watched_at: i64,
    pub title: String,
    pub parent_title: String,
}

#[derive(Clone, Debug)]
pub struct Library {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub paths: Vec<String>,
    pub created_at: i64,
    pub last_scan: i64,
}

impl Library {
    pub fn to_public(&self, item_count: i64, scanning: bool, with_paths: bool) -> Value {
        let mut m = json!({
            "id": self.id,
            "name": self.name,
            "type": self.kind,
            "createdAt": self.created_at,
            "lastScan": self.last_scan,
            "scanning": scanning,
            "itemCount": item_count,
        });
        if with_paths {
            m["paths"] = json!(self.paths);
        }
        m
    }
}

const USER_COLUMNS: &str = "id, username, password_hash, admin, created_at, password_changed_at, totp_secret, \
    totp_enabled, totp_last_step, recovery_codes, avatar_version, all_libraries, audio_language, subtitle_mode, \
    subtitle_language, ui_language, unpinned, max_age, allow_unrated, pin_hash, quality_home, quality_away, subtitle_style";

fn json_list(s: String) -> Vec<String> {
    serde_json::from_str(&s).unwrap_or_default()
}

fn user_from_row(r: &Row) -> rusqlite::Result<User> {
    Ok(User {
        id: r.get(0)?,
        username: r.get(1)?,
        password_hash: r.get(2)?,
        admin: r.get(3)?,
        created_at: r.get(4)?,
        password_changed_at: r.get(5)?,
        totp_secret: r.get(6)?,
        totp_enabled: r.get(7)?,
        totp_last_step: r.get(8)?,
        recovery_codes: json_list(r.get(9)?),
        avatar_version: r.get(10)?,
        all_libraries: r.get(11)?,
        library_ids: Vec::new(),
        audio_language: r.get(12)?,
        subtitle_mode: r.get(13)?,
        subtitle_language: r.get(14)?,
        ui_language: r.get(15)?,
        unpinned: json_list(r.get(16)?),
        max_age: r.get(17)?,
        allow_unrated: r.get(18)?,
        pin_hash: r.get(19)?,
        quality_home: r.get(20)?,
        quality_away: r.get(21)?,
        subtitle_style: r.get(22)?,
    })
}

fn library_from_row(r: &Row) -> rusqlite::Result<Library> {
    Ok(Library {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        paths: json_list(r.get(3)?),
        created_at: r.get(4)?,
        last_scan: r.get(5)?,
    })
}

/// Public id of a session: derived from the token, so the token itself never leaves the database layer.
pub fn session_id(token: &str) -> String {
    sha256(&format!("session:{token}"))[..16].to_string()
}

impl Db {
    /// Opens (or creates) the database and brings its schema up to date.
    pub fn open(path: &Path) -> Result<Db, String> {
        let conn = Connection::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
            .map_err(|e| e.to_string())?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let version = usize::try_from(version).unwrap_or(0);
        // The Java server (0.14 and earlier) kept a differently shaped database under the same name;
        // its data is not taken over, and it must not be changed by accident.
        let java: bool = conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'recovery_codes'", [], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
            .unwrap_or(false);
        if java {
            return Err(format!(
                "{} belongs to the Java version of FreeX Media Server. Move it away (or point [data] directory to a new folder) and set the server up again; see docs/installation.md",
                path.display()
            ));
        }
        if version > MIGRATIONS.len() {
            return Err(format!("the database is from a newer FreeX Media Server (schema {version})"));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute_batch(sql).map_err(|e| format!("schema {}: {e}", i + 1))?;
            tx.pragma_update(None, "user_version", (i + 1) as i64).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            tracing::info!("Database schema updated to version {}", i + 1);
        }
        restrict_permissions(path);
        let mut readers = Vec::new();
        for _ in 0..READERS {
            let r =
                Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
                    .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
            r.execute_batch("PRAGMA busy_timeout = 5000;").map_err(|e| e.to_string())?;
            readers.push(Mutex::new(r));
        }
        Ok(Db { conn: Mutex::new(conn), readers, next_reader: std::sync::atomic::AtomicUsize::new(0) })
    }

    // ------------------------------------------------------------------ webhooks

    pub fn webhooks(&self) -> Vec<crate::webhooks::Hook> {
        let c = self.r();
        let mut st = c.prepare_cached("SELECT id, url, events, secret, created_at FROM webhooks ORDER BY created_at").unwrap();
        st.query_map([], |r| {
            let events: String = r.get(2)?;
            Ok(crate::webhooks::Hook {
                id: r.get(0)?,
                url: r.get(1)?,
                events: events.split(',').filter(|e| !e.is_empty()).map(str::to_string).collect(),
                secret: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    pub fn save_webhook(&self, h: &crate::webhooks::Hook) {
        let _ = self.c().execute(
            "INSERT INTO webhooks (id, url, events, secret, created_at) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET url = ?2, events = ?3, secret = ?4",
            params![h.id, h.url, h.events.join(","), h.secret, h.created_at],
        );
    }

    // ------------------------------------------------------------------ edited metadata

    pub fn item_edits(&self) -> Vec<(String, String)> {
        let c = self.r();
        let mut st = c.prepare_cached("SELECT item_id, data FROM item_edits").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect()
    }

    /// Stores the edits of an entry; `None` removes them.
    pub fn save_item_edit(&self, item_id: &str, data: Option<&str>) {
        let c = self.c();
        let _ = match data {
            Some(d) => c.execute(
                "INSERT INTO item_edits (item_id, data) VALUES (?1, ?2) ON CONFLICT(item_id) DO UPDATE SET data = ?2",
                params![item_id, d],
            ),
            None => c.execute("DELETE FROM item_edits WHERE item_id = ?1", [item_id]),
        };
    }

    pub fn delete_webhook(&self, id: &str) -> bool {
        self.c().execute("DELETE FROM webhooks WHERE id = ?1", [id]).unwrap_or(0) > 0
    }

    /// Writes a consistent copy of the database to `path` (which must not exist yet).
    pub fn backup_to(&self, path: &Path) -> Result<(), String> {
        let target = path.to_str().ok_or("path is not UTF-8")?;
        self.c().execute("VACUUM INTO ?1", [target]).map(|_| ()).map_err(|e| e.to_string())
    }

    fn c(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A connection for reading: a free reader if there is one, otherwise the next in turn.
    fn r(&self) -> MutexGuard<'_, Connection> {
        if self.readers.is_empty() {
            return self.c();
        }
        let start = self.next_reader.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        for i in 0..self.readers.len() {
            if let Ok(g) = self.readers[(start + i) % self.readers.len()].try_lock() {
                return g;
            }
        }
        self.readers[start % self.readers.len()].lock().unwrap_or_else(|e| e.into_inner())
    }

    // ------------------------------------------------------------------ users

    pub fn has_users(&self) -> bool {
        self.r().query_row("SELECT EXISTS(SELECT 1 FROM users)", [], |r| r.get(0)).unwrap_or(false)
    }

    fn load_libraries_of(c: &Connection, mut u: User) -> User {
        if !u.all_libraries {
            let mut st = c.prepare_cached("SELECT library_id FROM user_libraries WHERE user_id = ?1 ORDER BY library_id").unwrap();
            u.library_ids = st.query_map([&u.id], |r| r.get(0)).unwrap().filter_map(Result::ok).collect();
        }
        u
    }

    pub fn users(&self) -> Vec<User> {
        let c = self.r();
        let mut st = c.prepare_cached(&format!("SELECT {USER_COLUMNS} FROM users ORDER BY username COLLATE NOCASE")).unwrap();
        let users: Vec<User> = st.query_map([], user_from_row).unwrap().filter_map(Result::ok).collect();
        users.into_iter().map(|u| Self::load_libraries_of(&c, u)).collect()
    }

    pub fn user(&self, id: &str) -> Option<User> {
        let c = self.r();
        let u = c.query_row(&format!("SELECT {USER_COLUMNS} FROM users WHERE id = ?1"), [id], user_from_row).optional().ok().flatten()?;
        Some(Self::load_libraries_of(&c, u))
    }

    pub fn user_by_name(&self, username: &str) -> Option<User> {
        let c = self.r();
        let u = c
            .query_row(&format!("SELECT {USER_COLUMNS} FROM users WHERE username = ?1"), [username], user_from_row)
            .optional()
            .ok()
            .flatten()?;
        Some(Self::load_libraries_of(&c, u))
    }

    /// Creates an account; None if the name is taken.
    pub fn add_user(&self, username: &str, password_hash: &str, admin: bool) -> Option<User> {
        let id = crate::auth::id();
        let now = now_ms();
        let inserted = self.c().execute(
            "INSERT INTO users (id, username, password_hash, admin, created_at, password_changed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, username, password_hash, admin, now],
        );
        match inserted {
            Ok(_) => self.user(&id),
            Err(_) => None,
        }
    }

    /// Writes every field of an account, including its shared libraries.
    pub fn update_user(&self, u: &User) {
        let mut c = self.c();
        let tx = c.transaction().unwrap();
        tx.execute(
            "UPDATE users SET username = ?2, password_hash = ?3, admin = ?4, password_changed_at = ?5, totp_secret = ?6, \
             totp_enabled = ?7, totp_last_step = ?8, recovery_codes = ?9, avatar_version = ?10, all_libraries = ?11, \
             audio_language = ?12, subtitle_mode = ?13, subtitle_language = ?14, ui_language = ?15, unpinned = ?16, \
             max_age = ?17, allow_unrated = ?18, pin_hash = ?19, quality_home = ?20, quality_away = ?21, subtitle_style = ?22 WHERE id = ?1",
            params![
                u.id,
                u.username,
                u.password_hash,
                u.admin,
                u.password_changed_at,
                u.totp_secret,
                u.totp_enabled,
                u.totp_last_step,
                serde_json::to_string(&u.recovery_codes).unwrap(),
                u.avatar_version,
                u.all_libraries,
                u.audio_language,
                u.subtitle_mode,
                u.subtitle_language,
                u.ui_language,
                serde_json::to_string(&u.unpinned).unwrap(),
                u.max_age,
                u.allow_unrated,
                u.pin_hash,
                u.quality_home,
                u.quality_away,
                u.subtitle_style,
            ],
        )
        .unwrap();
        tx.execute("DELETE FROM user_libraries WHERE user_id = ?1", [&u.id]).unwrap();
        if !u.all_libraries {
            for l in &u.library_ids {
                tx.execute("INSERT OR IGNORE INTO user_libraries (user_id, library_id) VALUES (?1, ?2)", params![u.id, l]).unwrap();
            }
        }
        tx.commit().unwrap();
    }

    pub fn remove_user(&self, id: &str) -> bool {
        self.c().execute("DELETE FROM users WHERE id = ?1", [id]).unwrap_or(0) > 0
    }

    pub fn admin_count(&self) -> i64 {
        self.r().query_row("SELECT COUNT(*) FROM users WHERE admin = 1", [], |r| r.get(0)).unwrap_or(0)
    }

    // ------------------------------------------------------------------ sessions

    /// Starts a session and returns its token (shown to the client once, stored as a hash).
    pub fn create_session(&self, user_id: &str, ip: &str, user_agent: &str) -> String {
        let token = crate::auth::token();
        let now = now_ms();
        self.c()
            .execute(
                "INSERT INTO sessions (id, token_hash, user_id, created_at, last_seen, ip, user_agent) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6)",
                params![session_id(&token), sha256(&token), user_id, now, ip, truncate(user_agent, 300)],
            )
            .unwrap();
        token
    }

    /// The user of a valid token; refreshes the session's last use (at most once a minute).
    pub fn user_for_token(&self, token: &str, max_idle_ms: i64, ip: &str) -> Option<User> {
        let hash = sha256(token);
        let now = now_ms();
        // Every request comes here: read on a reader, write (rarely) on the writer.
        let (user_id, last_seen): (String, i64) = self
            .r()
            .query_row("SELECT user_id, last_seen FROM sessions WHERE token_hash = ?1", [&hash], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .ok()
            .flatten()?;
        if now - last_seen > max_idle_ms {
            let _ = self.c().execute("DELETE FROM sessions WHERE token_hash = ?1", [&hash]);
            return None;
        }
        if now - last_seen > 60_000 {
            let _ = self.c().execute("UPDATE sessions SET last_seen = ?2, ip = ?3 WHERE token_hash = ?1", params![hash, now, ip]);
        }
        self.user(&user_id)
    }

    pub fn sessions_of(&self, user_id: &str) -> Vec<Session> {
        let c = self.r();
        let mut st = c
            .prepare_cached("SELECT id, created_at, last_seen, ip, user_agent FROM sessions WHERE user_id = ?1 ORDER BY last_seen DESC")
            .unwrap();
        st.query_map([user_id], |r| {
            Ok(Session { id: r.get(0)?, created_at: r.get(1)?, last_seen: r.get(2)?, ip: r.get(3)?, user_agent: r.get(4)? })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    pub fn remove_session(&self, token: &str) {
        let _ = self.c().execute("DELETE FROM sessions WHERE token_hash = ?1", [sha256(token)]);
    }

    pub fn remove_session_by_id(&self, user_id: &str, id: &str) -> bool {
        self.c().execute("DELETE FROM sessions WHERE user_id = ?1 AND id = ?2", [user_id, id]).unwrap_or(0) > 0
    }

    /// Ends all sessions of a user except the one with `keep_token`; returns how many ended.
    pub fn remove_sessions_of(&self, user_id: &str, keep_token: Option<&str>) -> usize {
        let keep = keep_token.map(sha256).unwrap_or_default();
        self.c().execute("DELETE FROM sessions WHERE user_id = ?1 AND token_hash != ?2", [user_id, &keep]).unwrap_or(0)
    }

    /// Removes sessions unused for longer than allowed.
    pub fn remove_expired_sessions(&self, max_idle_ms: i64) -> usize {
        self.c().execute("DELETE FROM sessions WHERE last_seen < ?1", [now_ms() - max_idle_ms]).unwrap_or(0)
    }

    // ------------------------------------------------------------------ login history

    pub fn add_login_event(&self, user_id: &str, e: &LoginEvent) {
        let c = self.c();
        let _ = c.execute(
            "INSERT INTO login_events (user_id, time, ip, user_agent, success, detail) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![user_id, e.time, e.ip, truncate(&e.user_agent, 300), e.success, e.detail],
        );
        let _ = c.execute(
            "DELETE FROM login_events WHERE user_id = ?1 AND time < (SELECT time FROM login_events WHERE user_id = ?1 \
             ORDER BY time DESC LIMIT 1 OFFSET ?2)",
            params![user_id, LOGIN_EVENTS_KEPT - 1],
        );
    }

    pub fn login_events(&self, user_id: &str) -> Vec<LoginEvent> {
        let c = self.r();
        let mut st = c
            .prepare_cached("SELECT time, ip, user_agent, success, detail FROM login_events WHERE user_id = ?1 ORDER BY time DESC")
            .unwrap();
        st.query_map([user_id], |r| {
            Ok(LoginEvent { time: r.get(0)?, ip: r.get(1)?, user_agent: r.get(2)?, success: r.get(3)?, detail: r.get(4)? })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    // ------------------------------------------------------------------ libraries

    pub fn libraries(&self) -> Vec<Library> {
        let c = self.r();
        let mut st =
            c.prepare_cached("SELECT id, name, type, paths, created_at, last_scan FROM libraries ORDER BY name COLLATE NOCASE").unwrap();
        st.query_map([], library_from_row).unwrap().filter_map(Result::ok).collect()
    }

    pub fn library(&self, id: &str) -> Option<Library> {
        self.r()
            .query_row("SELECT id, name, type, paths, created_at, last_scan FROM libraries WHERE id = ?1", [id], library_from_row)
            .optional()
            .ok()
            .flatten()
    }

    pub fn add_library(&self, name: &str, kind: &str, paths: &[String]) -> Library {
        let l = Library {
            id: crate::auth::id(),
            name: name.to_string(),
            kind: kind.to_string(),
            paths: paths.to_vec(),
            created_at: now_ms(),
            last_scan: 0,
        };
        self.save_library(&l);
        l
    }

    pub fn save_library(&self, l: &Library) {
        self.c()
            .execute(
                "INSERT INTO libraries (id, name, type, paths, created_at, last_scan) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                 ON CONFLICT(id) DO UPDATE SET name = ?2, paths = ?4, last_scan = ?6",
                params![l.id, l.name, l.kind, serde_json::to_string(&l.paths).unwrap(), l.created_at, l.last_scan],
            )
            .unwrap();
    }

    pub fn remove_library(&self, id: &str) -> bool {
        self.c().execute("DELETE FROM libraries WHERE id = ?1", [id]).unwrap_or(0) > 0
    }

    pub fn set_last_scan(&self, id: &str, time: i64) {
        let _ = self.c().execute("UPDATE libraries SET last_scan = ?2 WHERE id = ?1", params![id, time]);
    }

    // ------------------------------------------------------------------ library index

    /// All stored items as JSON.
    pub fn items(&self) -> Vec<String> {
        let c = self.r();
        let mut st = c.prepare("SELECT data FROM items").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().filter_map(Result::ok).collect()
    }

    /// Replaces the items of a library with `(id, json)` pairs; false if the library no longer exists.
    pub fn replace_items(&self, library_id: &str, items: &[(String, String)]) -> bool {
        let mut c = self.c();
        let result = (|| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            // Only what changed is written: a rescan of a large, unchanged library touches nothing.
            let keep: std::collections::HashSet<&str> = items.iter().map(|(id, _)| id.as_str()).collect();
            let old: Vec<String> = {
                let mut st = tx.prepare("SELECT id FROM items WHERE library_id = ?1")?;
                st.query_map([library_id], |r| r.get(0))?.filter_map(Result::ok).collect()
            };
            {
                let mut del = tx.prepare("DELETE FROM items WHERE id = ?1")?;
                for id in old.iter().filter(|id| !keep.contains(id.as_str())) {
                    del.execute([id])?;
                }
                let mut st = tx.prepare(
                    "INSERT INTO items (id, library_id, data) VALUES (?1, ?2, ?3) \
                     ON CONFLICT(id) DO UPDATE SET library_id = excluded.library_id, data = excluded.data \
                     WHERE items.data IS NOT excluded.data OR items.library_id IS NOT excluded.library_id",
                )?;
                for (id, data) in items {
                    st.execute(params![id, library_id, data])?;
                }
            }
            tx.commit()
        })();
        if let Err(e) = &result {
            tracing::warn!("Cannot store the index of library {library_id}: {e}");
        }
        result.is_ok()
    }

    // ------------------------------------------------------------------ watch state

    pub fn progress(&self, user_id: &str, item_id: &str) -> Progress {
        self.r()
            .query_row(
                "SELECT offset_ms, watched, view_count, updated_at FROM progress WHERE user_id = ?1 AND item_id = ?2",
                [user_id, item_id],
                |r| Ok(Progress { offset_ms: r.get(0)?, watched: r.get(1)?, view_count: r.get(2)?, updated_at: r.get(3)? }),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub fn set_progress(&self, user_id: &str, item_id: &str, p: Progress) {
        let r = self.c().execute(
            "INSERT INTO progress (user_id, item_id, offset_ms, watched, view_count, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(user_id, item_id) DO UPDATE SET offset_ms = ?3, watched = ?4, view_count = ?5, updated_at = ?6",
            params![user_id, item_id, p.offset_ms, p.watched, p.view_count, p.updated_at],
        );
        if let Err(e) = r {
            tracing::warn!("Cannot store watch state: {e}");
        }
    }

    /// Watch state of a user by item id.
    pub fn progress_of(&self, user_id: &str) -> HashMap<String, Progress> {
        let c = self.r();
        let mut st =
            c.prepare_cached("SELECT item_id, offset_ms, watched, view_count, updated_at FROM progress WHERE user_id = ?1").unwrap();
        st.query_map([user_id], |r| {
            Ok((r.get(0)?, Progress { offset_ms: r.get(1)?, watched: r.get(2)?, view_count: r.get(3)?, updated_at: r.get(4)? }))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }
}

impl Db {
    // ------------------------------------------------------------------ watch history

    pub fn add_history(&self, user_id: &str, item_id: &str, time: i64, title: &str, parent_title: &str) {
        let _ = self.c().execute(
            "INSERT INTO watch_history (user_id, item_id, watched_at, title, parent_title) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![user_id, item_id, time, truncate(title, 300), truncate(parent_title, 300)],
        );
    }

    /// Newest first; `user_id` None = all users.
    pub fn history(&self, user_id: Option<&str>, offset: i64, limit: i64) -> Vec<HistoryEntry> {
        let c = self.r();
        let mut st = c
            .prepare_cached(
                "SELECT id, user_id, item_id, watched_at, title, parent_title FROM watch_history \
                 WHERE (?1 IS NULL OR user_id = ?1) ORDER BY watched_at DESC, id DESC LIMIT ?2 OFFSET ?3",
            )
            .unwrap();
        st.query_map(params![user_id, limit, offset], |r| {
            Ok(HistoryEntry {
                id: r.get(0)?,
                user_id: r.get(1)?,
                item_id: r.get(2)?,
                watched_at: r.get(3)?,
                title: r.get(4)?,
                parent_title: r.get(5)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    /// All entries since a time (ms), oldest first, for the statistics of the dashboard.
    pub fn history_since(&self, since: i64) -> Vec<HistoryEntry> {
        let c = self.r();
        let mut st = c
            .prepare_cached(
                "SELECT id, user_id, item_id, watched_at, title, parent_title FROM watch_history \
                 WHERE watched_at >= ?1 ORDER BY watched_at",
            )
            .unwrap();
        st.query_map([since], |r| {
            Ok(HistoryEntry {
                id: r.get(0)?,
                user_id: r.get(1)?,
                item_id: r.get(2)?,
                watched_at: r.get(3)?,
                title: r.get(4)?,
                parent_title: r.get(5)?,
            })
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    /// Deletes one entry of the user; false if there is none with that id.
    pub fn remove_history(&self, user_id: &str, id: i64) -> bool {
        self.c().execute("DELETE FROM watch_history WHERE id = ?1 AND user_id = ?2", params![id, user_id]).unwrap_or(0) > 0
    }

    pub fn clear_history(&self, user_id: &str) -> usize {
        self.c().execute("DELETE FROM watch_history WHERE user_id = ?1", [user_id]).unwrap_or(0)
    }

    // ------------------------------------------------------------------ online metadata

    /// `(item id, json)` of all stored metadata.
    pub fn metadata(&self) -> Vec<(String, String)> {
        let c = self.r();
        let mut st = c.prepare("SELECT item_id, data FROM metadata").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect()
    }

    pub fn save_metadata(&self, item_id: &str, data: Option<&str>) {
        let c = self.c();
        let _ = match data {
            Some(d) => {
                c.execute("INSERT INTO metadata (item_id, data) VALUES (?1, ?2) ON CONFLICT(item_id) DO UPDATE SET data = ?2", [item_id, d])
            }
            None => c.execute("DELETE FROM metadata WHERE item_id = ?1", [item_id]),
        };
    }

    pub fn tmdb_collections(&self) -> Vec<(i64, String)> {
        let c = self.r();
        let mut st = c.prepare("SELECT id, data FROM tmdb_collections").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect()
    }

    pub fn save_tmdb_collection(&self, id: i64, data: &str) {
        let _ = self
            .c()
            .execute("INSERT INTO tmdb_collections (id, data) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET data = ?2", params![id, data]);
    }

    /// Stores one changed entry of the library index.
    pub fn save_item(&self, id: &str, data: &str) {
        let _ = self.c().execute("UPDATE items SET data = ?2 WHERE id = ?1", [id, data]);
    }
}

impl Db {
    // ------------------------------------------------------------------ own collections

    fn items_of(c: &Connection, table: &str, key: &str, id: &str) -> Vec<String> {
        let sql = format!("SELECT item_id FROM {table} WHERE {key} = ?1 ORDER BY position");
        let mut st = c.prepare_cached(&sql).unwrap();
        st.query_map([id], |r| r.get(0)).unwrap().filter_map(Result::ok).collect()
    }

    fn write_items(c: &Connection, table: &str, key: &str, id: &str, items: &[String]) -> rusqlite::Result<()> {
        c.execute(&format!("DELETE FROM {table} WHERE {key} = ?1"), [id])?;
        let mut st = c.prepare_cached(&format!("INSERT INTO {table} ({key}, position, item_id) VALUES (?1, ?2, ?3)"))?;
        for (i, item) in items.iter().enumerate() {
            st.execute(params![id, i as i64, item])?;
        }
        Ok(())
    }

    fn collection_rows(&self, filter: &str, arg: &str) -> Vec<OwnCollection> {
        let c = self.r();
        let sql = format!("SELECT id, library_id, name, summary, created_at, filter FROM collections WHERE {filter} = ?1");
        let rows: Vec<OwnCollection> = {
            let mut st = c.prepare_cached(&sql).unwrap();
            st.query_map([arg], |r| {
                Ok(OwnCollection {
                    id: r.get(0)?,
                    library_id: r.get(1)?,
                    name: r.get(2)?,
                    summary: r.get(3)?,
                    item_ids: Vec::new(),
                    created_at: r.get(4)?,
                    filter: r.get(5)?,
                })
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect()
        };
        rows.into_iter()
            .map(|mut o| {
                o.item_ids = Db::items_of(&c, "collection_items", "collection_id", &o.id);
                o
            })
            .collect()
    }

    pub fn collections_of(&self, library_id: &str) -> Vec<OwnCollection> {
        let mut v = self.collection_rows("library_id", library_id);
        v.sort_by_key(|c| c.name.to_lowercase());
        v
    }

    pub fn collection(&self, id: &str) -> Option<OwnCollection> {
        self.collection_rows("id", id).pop()
    }

    pub fn create_collection(&self, library_id: &str, name: &str, filter: &str) -> OwnCollection {
        let c = OwnCollection {
            id: crate::auth::id(),
            library_id: library_id.to_string(),
            name: name.to_string(),
            summary: String::new(),
            item_ids: Vec::new(),
            created_at: now_ms(),
            filter: filter.to_string(),
        };
        self.c()
            .execute(
                "INSERT INTO collections (id, library_id, name, summary, created_at, filter) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![c.id, c.library_id, c.name, c.summary, c.created_at, c.filter],
            )
            .unwrap();
        c
    }

    pub fn save_collection(&self, o: &OwnCollection) {
        let mut c = self.c();
        let r = (|| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            tx.execute(
                "UPDATE collections SET name = ?2, summary = ?3, filter = ?4 WHERE id = ?1",
                params![o.id, o.name, o.summary, o.filter],
            )?;
            Db::write_items(&tx, "collection_items", "collection_id", &o.id, &o.item_ids)?;
            tx.commit()
        })();
        if let Err(e) = r {
            tracing::warn!("Cannot store collection {}: {e}", o.id);
        }
    }

    pub fn delete_collection(&self, id: &str) {
        let _ = self.c().execute("DELETE FROM collections WHERE id = ?1", [id]);
    }

    /// `(id, name)` of the own collections that contain an item.
    pub fn collections_with(&self, item_id: &str) -> Vec<(String, String)> {
        let c = self.r();
        let mut st = c
            .prepare_cached(
                "SELECT DISTINCT c.id, c.name FROM collections c JOIN collection_items i ON i.collection_id = c.id \
                 WHERE i.item_id = ?1 ORDER BY c.name",
            )
            .unwrap();
        st.query_map([item_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().filter_map(Result::ok).collect()
    }

    // ------------------------------------------------------------------ playlists

    fn playlist_rows(&self, sql: &str, args: &[&str]) -> Vec<Playlist> {
        let c = self.r();
        let rows: Vec<Playlist> = {
            let mut st = c.prepare_cached(sql).unwrap();
            st.query_map(rusqlite::params_from_iter(args.iter()), |r| {
                Ok(Playlist {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    name: r.get(2)?,
                    item_ids: Vec::new(),
                    created_at: r.get(3)?,
                    updated_at: r.get(4)?,
                })
            })
            .unwrap()
            .filter_map(Result::ok)
            .collect()
        };
        rows.into_iter()
            .map(|mut p| {
                p.item_ids = Db::items_of(&c, "playlist_items", "playlist_id", &p.id);
                p
            })
            .collect()
    }

    /// Playlists of a user, last changed first.
    pub fn playlists_of(&self, user_id: &str) -> Vec<Playlist> {
        self.playlist_rows(
            "SELECT id, user_id, name, created_at, updated_at FROM playlists WHERE user_id = ?1 ORDER BY updated_at DESC",
            &[user_id],
        )
    }

    /// A playlist of the user; None for playlists of others.
    pub fn playlist(&self, user_id: &str, id: &str) -> Option<Playlist> {
        self.playlist_rows("SELECT id, user_id, name, created_at, updated_at FROM playlists WHERE id = ?1 AND user_id = ?2", &[id, user_id])
            .pop()
    }

    pub fn create_playlist(&self, user_id: &str, name: &str, item_ids: Vec<String>) -> Playlist {
        let now = now_ms();
        let mut p = Playlist {
            id: crate::auth::id(),
            user_id: user_id.to_string(),
            name: name.to_string(),
            item_ids,
            created_at: now,
            updated_at: now,
        };
        self.c()
            .execute(
                "INSERT INTO playlists (id, user_id, name, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![p.id, p.user_id, p.name, p.created_at, p.updated_at],
            )
            .unwrap();
        self.save_playlist(&mut p);
        p
    }

    pub fn save_playlist(&self, p: &mut Playlist) {
        p.updated_at = now_ms();
        let mut c = self.c();
        let r = (|| -> rusqlite::Result<()> {
            let tx = c.transaction()?;
            tx.execute("UPDATE playlists SET name = ?2, updated_at = ?3 WHERE id = ?1", params![p.id, p.name, p.updated_at])?;
            Db::write_items(&tx, "playlist_items", "playlist_id", &p.id, &p.item_ids)?;
            tx.commit()
        })();
        if let Err(e) = r {
            tracing::warn!("Cannot store playlist {}: {e}", p.id);
        }
    }

    pub fn delete_playlist(&self, id: &str) {
        let _ = self.c().execute("DELETE FROM playlists WHERE id = ?1", [id]);
    }

    // ------------------------------------------------------------------ watchlist

    /// Item ids on the user's watchlist, last added first.
    pub fn watchlist(&self, user_id: &str) -> Vec<String> {
        let c = self.r();
        let mut st = c.prepare_cached("SELECT item_id FROM watchlist WHERE user_id = ?1 ORDER BY added_at DESC, rowid DESC").unwrap();
        st.query_map([user_id], |r| r.get(0)).unwrap().filter_map(Result::ok).collect()
    }

    pub fn set_watchlist(&self, user_id: &str, item_id: &str, on: bool) {
        let c = self.c();
        let _ = if on {
            c.execute(
                "INSERT INTO watchlist (user_id, item_id, added_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(user_id, item_id) DO UPDATE SET added_at = excluded.added_at",
                params![user_id, item_id, now_ms()],
            )
        } else {
            c.execute("DELETE FROM watchlist WHERE user_id = ?1 AND item_id = ?2", [user_id, item_id])
        };
    }
}

impl Db {
    // ------------------------------------------------------------------ markers

    pub fn markers(&self) -> HashMap<String, crate::markers::Markers> {
        use crate::markers::{Markers, Segment};
        let c = self.r();
        let mut st = c
            .prepare("SELECT item_id, file_modified, intro_start, intro_end, credits_start, credits_end, analyzed_at FROM markers")
            .unwrap();
        let seg = |s: Option<i64>, e: Option<i64>| Some(Segment { start_ms: s?, end_ms: e? });
        st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Markers {
                    file_modified: r.get(1)?,
                    intro: seg(r.get(2)?, r.get(3)?),
                    credits: seg(r.get(4)?, r.get(5)?),
                    analyzed_at: r.get(6)?,
                },
            ))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }

    pub fn save_markers(&self, item_id: &str, m: &crate::markers::Markers) {
        let _ = self.c().execute(
            "INSERT INTO markers (item_id, file_modified, intro_start, intro_end, credits_start, credits_end, analyzed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(item_id) DO UPDATE SET file_modified = ?2, intro_start = ?3, \
             intro_end = ?4, credits_start = ?5, credits_end = ?6, analyzed_at = ?7",
            params![
                item_id,
                m.file_modified,
                m.intro.map(|s| s.start_ms),
                m.intro.map(|s| s.end_ms),
                m.credits.map(|s| s.start_ms),
                m.credits.map(|s| s.end_ms),
                m.analyzed_at
            ],
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The database holds password hashes: readable for the server's user only.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for p in [path.to_path_buf(), path.with_extension("db-wal"), path.with_extension("db-shm")] {
            if p.exists() {
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path; // Windows: the installer restricts the whole data folder
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        let dir = std::env::temp_dir().join(format!("freex-test-{}", crate::auth::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Db::open(&dir.join("freex.db")).unwrap()
    }

    #[test]
    fn users_and_sessions() {
        let db = db();
        assert!(!db.has_users());
        let u = db.add_user("Alice", "hash", true).unwrap();
        assert!(db.add_user("alice", "hash", false).is_none(), "names are unique regardless of case");
        let token = db.create_session(&u.id, "127.0.0.1", "test");
        assert_eq!(db.user_for_token(&token, 60_000, "127.0.0.1").unwrap().id, u.id);
        assert!(db.user_for_token("wrong", 60_000, "127.0.0.1").is_none());
        assert_eq!(db.sessions_of(&u.id)[0].id, session_id(&token));
        db.remove_session(&token);
        assert!(db.user_for_token(&token, 60_000, "127.0.0.1").is_none());
    }

    #[test]
    fn library_sharing() {
        let db = db();
        let lib = db.add_library("Movies", "movies", &["/m".into()]);
        let mut u = db.add_user("bob", "hash", false).unwrap();
        assert!(u.may_access(&lib.id));
        u.all_libraries = false;
        u.library_ids = vec![];
        db.update_user(&u);
        assert!(!db.user(&u.id).unwrap().may_access(&lib.id));
        u.library_ids = vec![lib.id.clone()];
        db.update_user(&u);
        assert!(db.user(&u.id).unwrap().may_access(&lib.id));
        db.remove_library(&lib.id);
        assert!(db.user(&u.id).unwrap().library_ids.is_empty(), "sharing goes with the library");
    }

    #[test]
    fn login_history_is_capped() {
        let db = db();
        let u = db.add_user("carol", "hash", false).unwrap();
        for i in 0..60 {
            db.add_login_event(
                &u.id,
                &LoginEvent { time: i, ip: "x".into(), user_agent: "y".into(), success: true, detail: "password".into() },
            );
        }
        let events = db.login_events(&u.id);
        assert_eq!(events.len() as i64, LOGIN_EVENTS_KEPT);
        assert_eq!(events[0].time, 59);
    }
}
