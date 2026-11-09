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

//! Online metadata from TMDB: titles, descriptions, ratings, age ratings, cast and crew,
//! trailers, artwork and movie series. Stored per library entry in the database; images in
//! `<data>/metadata`. Local `.nfo` data wins: online data then only fills gaps.
//!
//! Finding the right entry, in this order: an id pinned with "fix match", a TMDB, IMDb or TVDB
//! id from the `.nfo` or the name, a search with several variants of the title (see
//! [`matching`]), and for shows TVmaze, whose IMDb/TVDB ids TMDB then resolves exactly.

pub mod edits;
pub mod matching;
pub mod tmdb;

use crate::AppState;
use crate::auth::now_ms;
use crate::db::Db;
use crate::i18n::Text;
use crate::media::names::sort_title;
use crate::media::{Item, Kind};
use matching::Candidate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use tmdb::{MetaError, MetaResult, Tmdb, TvMaze, year_of};

const REFRESH_AFTER_MS: i64 = 30 * 86_400_000;
const RETRY_FAILED_AFTER_MS: i64 = 3 * 86_400_000;
const CAST_SIZE: usize = 20;
/// A match at least this good ends the search early (close title and the right year).
const SURE_SCORE: f64 = 8.0;

/// One cast member; `image` is a local file or None.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Person {
    pub id: i64,
    pub name: String,
    pub role: String,
    pub image: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Metadata {
    pub tmdb_id: i64,
    pub title: String,
    pub original_title: String,
    pub summary: String,
    pub tagline: String,
    pub year: i32,
    pub genres: String,
    pub rating: f64,
    pub content_rating: String,
    pub studio: String,
    pub air_date: String,
    pub trailer_key: String,
    pub collection_id: i64,
    pub collection_name: String,
    pub cast: Vec<Person>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub updated_at: i64,
    /// Set by "fix match" (the id stays) or "remove match" (never searched again).
    pub locked: bool,
    /// Last time nothing was found; retried after a while.
    pub failed_at: i64,
    /// Title and year the last search was for; a renamed entry is searched again at once.
    pub searched_for: String,
    /// How the entry was found: "pinned", "id", "imdb", "tvdb", "search", "tvmaze".
    pub source: String,
}

impl Metadata {
    pub fn matched(&self) -> bool {
        self.tmdb_id > 0
    }
}

/// A TMDB movie series.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Collection {
    pub name: String,
    pub summary: String,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
}

pub struct Service {
    tmdb: Option<Tmdb>,
    tvmaze: Option<TvMaze>,
    region: String,
    dir: PathBuf,
    items: RwLock<HashMap<String, Arc<Metadata>>>,
    collections: RwLock<HashMap<i64, Collection>>,
    queued: Mutex<HashSet<String>>,
    serial: tokio::sync::Mutex<()>,
    last_error: Mutex<Option<Text>>,
    /// Metadata edited by hand, by item id.
    edits: RwLock<HashMap<String, edits::Edit>>,
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}

fn opt(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().filter(|x| !x.is_empty()).map(str::to_string)
}

fn names(v: &Value) -> String {
    v.as_array().map(|a| a.iter().map(|g| s(g, "name")).filter(|n| !n.is_empty()).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

fn round1(v: &Value) -> f64 {
    v.as_f64().map_or(0.0, |r| (r * 10.0).round() / 10.0)
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

/// Key of what an entry is searched by; when it changes, the entry is searched again.
fn search_key(it: &Item) -> String {
    format!("{}|{}|{}|{}", matching::normalize(&it.title), it.year, it.imdb_hint, it.tmdb_hint)
}

impl Service {
    /// `tmdb` None: no key, only stored metadata is applied.
    pub fn new(db: &Db, data_dir: &Path, tmdb: Option<Tmdb>, tvmaze: Option<TvMaze>, region: &str) -> Service {
        let items = db
            .metadata()
            .into_iter()
            .filter_map(|(id, data)| serde_json::from_str::<Metadata>(&data).ok().map(|m| (id, Arc::new(m))))
            .collect();
        let collections = db.tmdb_collections().into_iter().filter_map(|(id, d)| serde_json::from_str(&d).ok().map(|c| (id, c))).collect();
        Service {
            tmdb,
            tvmaze,
            region: region.to_uppercase(),
            dir: data_dir.join("metadata"),
            items: RwLock::new(items),
            collections: RwLock::new(collections),
            queued: Mutex::default(),
            serial: tokio::sync::Mutex::new(()),
            last_error: Mutex::default(),
            edits: RwLock::new(db.item_edits().into_iter().filter_map(|(id, d)| serde_json::from_str(&d).ok().map(|e| (id, e))).collect()),
        }
    }

    pub fn edit(&self, item_id: &str) -> Option<edits::Edit> {
        self.edits.read().unwrap().get(item_id).cloned()
    }

    /// Stores the edits of an entry (an empty edit removes them).
    pub fn set_edit(&self, db: &Db, item_id: &str, e: edits::Edit) {
        if e.is_empty() {
            db.save_item_edit(item_id, None);
            self.edits.write().unwrap().remove(item_id);
        } else {
            db.save_item_edit(item_id, Some(&serde_json::to_string(&e).expect("serializable")));
            self.edits.write().unwrap().insert(item_id.to_string(), e);
        }
    }

    pub fn enabled(&self) -> bool {
        self.tmdb.is_some()
    }

    pub fn busy(&self) -> bool {
        !self.queued.lock().unwrap().is_empty()
    }

    pub fn last_error(&self) -> Option<Text> {
        self.last_error.lock().unwrap().clone()
    }

    pub fn get(&self, item_id: &str) -> Option<Arc<Metadata>> {
        self.items.read().unwrap().get(item_id).cloned()
    }

    pub fn collection(&self, id: i64) -> Option<Collection> {
        self.collections.read().unwrap().get(&id).cloned()
    }

    fn put(&self, db: &Db, item_id: &str, md: Metadata) {
        db.save_metadata(item_id, Some(&serde_json::to_string(&md).expect("serializable")));
        self.items.write().unwrap().insert(item_id.to_string(), Arc::new(md));
    }

    fn remove(&self, db: &Db, item_id: &str) {
        db.save_metadata(item_id, None);
        self.items.write().unwrap().remove(item_id);
    }

    /// Merges stored metadata into the entries of a library (after a scan). No network.
    /// Edits made by hand are applied last, so they win.
    pub fn apply_all(&self, items: &mut [Item]) {
        let all = self.items.read().unwrap();
        let edits = self.edits.read().unwrap();
        for it in items.iter_mut() {
            if let Some(md) = all.get(&it.id) {
                apply(it, md);
            }
            if let Some(e) = edits.get(&it.id) {
                e.apply(it);
            }
        }
    }

    /// People by TMDB id: one of the entries that lists them.
    pub fn person(&self, id: i64) -> Option<Person> {
        self.items.read().unwrap().values().flat_map(|m| m.cast.iter()).find(|p| p.id == id).cloned()
    }

    /// Entries whose cast contains a person.
    pub fn items_with_person(&self, id: i64) -> Vec<String> {
        self.items.read().unwrap().iter().filter(|(_, m)| m.cast.iter().any(|p| p.id == id)).map(|(k, _)| k.clone()).collect()
    }

    // ================================================================== refreshing

    /// Queues fetching metadata for the entries of a library that have none (or old, or all
    /// with `force`); does nothing without a key or if the library is already queued.
    pub fn refresh_library(app: &Arc<AppState>, library_id: &str, force: bool) {
        if app.metadata.tmdb.is_none() || !app.metadata.queued.lock().unwrap().insert(library_id.to_string()) {
            return;
        }
        let (app, id) = (app.clone(), library_id.to_string());
        tokio::spawn(async move {
            {
                let _turn = app.metadata.serial.lock().await;
                app.metadata.refresh_now(&app, &id, force).await;
            }
            app.metadata.queued.lock().unwrap().remove(&id);
        });
    }

    async fn refresh_now(&self, app: &AppState, library_id: &str, force: bool) {
        let Some(lib) = app.db.library(library_id) else { return };
        let now = now_ms();
        let top: Vec<Arc<Item>> = app.catalog.snapshot().top_level(library_id).to_vec();
        let mut done = 0;
        let mut failed = 0;
        for it in top {
            let md = self.get(&it.id);
            let due = match &md {
                _ if force => md.as_ref().is_none_or(|m| !m.locked || m.matched()),
                None => true,
                Some(m) if m.locked && !m.matched() => false, // "remove match"
                Some(m) if m.matched() => now - m.updated_at > REFRESH_AFTER_MS,
                Some(m) => now - m.failed_at > RETRY_FAILED_AFTER_MS || m.searched_for != search_key(&it),
            };
            if !due {
                continue;
            }
            let pinned = md.as_ref().filter(|m| m.locked).map_or(0, |m| m.tmdb_id);
            match self.refresh_item(app, &it, pinned).await {
                Ok(true) => done += 1,
                Ok(false) => failed += 1,
                Err(e) => {
                    tracing::warn!("Metadata for '{}': {}", it.title, e.0.render("en"));
                    let stop = e.key_rejected();
                    *self.last_error.lock().unwrap() = Some(e.0);
                    if stop {
                        break;
                    }
                    continue;
                }
            }
            *self.last_error.lock().unwrap() = None;
        }
        if done + failed > 0 {
            tracing::info!("Metadata of '{}': {done} entries updated, {failed} without a match", lib.name);
            self.reapply(app, library_id);
        }
    }

    /// Applies the stored metadata to the current entries of a library.
    pub fn reapply(&self, app: &AppState, library_id: &str) {
        app.catalog.update_library(&app.db, library_id, |items| self.apply_all(items));
    }

    /// Fetches one movie or show (with its seasons and episodes). Ok(false): no match found.
    async fn refresh_item(&self, app: &AppState, it: &Item, pinned: i64) -> MetaResult<bool> {
        let tmdb = self.tmdb.as_ref().ok_or_else(|| MetaError::new("error.tmdbNoKey"))?;
        let kind = if it.kind == Kind::Movie { "movie" } else { "tv" };
        let found = if pinned > 0 { Some((pinned, "pinned")) } else { self.find(tmdb, it, kind).await? };
        let Some((id, source)) = found else {
            let mut md = self.get(&it.id).map(|m| Metadata::clone(&m)).unwrap_or_default();
            md.failed_at = now_ms();
            md.tmdb_id = 0;
            md.searched_for = search_key(it);
            self.put(&app.db, &it.id, md);
            tracing::info!("No TMDB match for '{}' ({})", it.title, it.year);
            return Ok(false);
        };
        let ok =
            if it.kind == Kind::Movie { self.fetch_movie(app, tmdb, it, id).await? } else { self.fetch_show(app, tmdb, it, id).await? };
        let mut md = self.get(&it.id).map(|m| Metadata::clone(&m)).unwrap_or_default();
        if !ok {
            // The id does not exist (any more) at TMDB.
            md = Metadata { tmdb_id: 0, failed_at: now_ms(), ..Metadata::default() };
        }
        md.locked = pinned > 0 && ok;
        md.source = source.to_string();
        md.searched_for = search_key(it);
        self.put(&app.db, &it.id, md);
        Ok(ok)
    }

    /// Re-fetches one entry now ("refresh metadata", "fix match"); for a season or episode its show.
    pub async fn refresh_one(&self, app: &AppState, it: &Item) -> MetaResult<()> {
        if self.tmdb.is_none() {
            return Err(MetaError::new("error.tmdbNoKey"));
        }
        let top = top_of(app, it);
        let pinned = self.get(&top.id).filter(|m| m.locked).map_or(0, |m| m.tmdb_id);
        let _turn = self.serial.lock().await;
        self.refresh_item(app, &top, pinned).await?;
        self.reapply(app, &top.library_id);
        Ok(())
    }

    /// Pins an entry to a TMDB id and fetches it.
    pub async fn pin(&self, app: &AppState, it: &Item, tmdb_id: i64) -> MetaResult<()> {
        let top = top_of(app, it);
        self.put(&app.db, &top.id, Metadata { tmdb_id, locked: true, ..Metadata::default() });
        self.refresh_one(app, &top).await
    }

    /// Removes the online metadata of an entry and its seasons and episodes, for good.
    pub fn unpin(&self, app: &AppState, it: &Item) {
        let top = top_of(app, it);
        let idx = app.catalog.snapshot();
        let mut stack = vec![top.id.clone()];
        while let Some(id) = stack.pop() {
            for c in idx.children(&id) {
                self.remove(&app.db, &c.id);
                stack.push(c.id.clone());
            }
        }
        self.put(&app.db, &top.id, Metadata { locked: true, failed_at: now_ms(), searched_for: search_key(&top), ..Metadata::default() });
    }

    // ------------------------------------------------------------------ finding

    /// The TMDB id of an entry and how it was found, or None.
    async fn find(&self, tmdb: &Tmdb, it: &Item, kind: &str) -> MetaResult<Option<(i64, &'static str)>> {
        if it.tmdb_hint > 0 {
            return Ok(Some((it.tmdb_hint, "id")));
        }
        if !it.imdb_hint.is_empty()
            && let Some(id) = self.find_external(tmdb, &it.imdb_hint, "imdb_id", kind).await?
        {
            return Ok(Some((id, "imdb")));
        }
        if it.tvdb_hint > 0
            && kind == "tv"
            && let Some(id) = self.find_external(tmdb, &it.tvdb_hint.to_string(), "tvdb_id", kind).await?
        {
            return Ok(Some((id, "tvdb")));
        }
        if let Some(id) = self.search_best(tmdb, it, kind).await? {
            return Ok(Some((id, "search")));
        }
        if kind == "tv"
            && let Some(id) = self.find_via_tvmaze(tmdb, it).await?
        {
            return Ok(Some((id, "tvmaze")));
        }
        Ok(None)
    }

    /// TMDB's id for an IMDb or TVDB id.
    async fn find_external(&self, tmdb: &Tmdb, id: &str, source: &str, kind: &str) -> MetaResult<Option<i64>> {
        let Some(r) = tmdb.get(&format!("/find/{id}"), &[("external_source", source), ("language", &tmdb.language)]).await? else {
            return Ok(None);
        };
        let key = if kind == "movie" { "movie_results" } else { "tv_results" };
        Ok(list(&r[key]).first().and_then(|m| m["id"].as_i64()))
    }

    /// Searches with several variants of the title, with and without the year and in English, and
    /// returns the best believable result.
    async fn search_best(&self, tmdb: &Tmdb, it: &Item, kind: &str) -> MetaResult<Option<i64>> {
        let variants = matching::query_variants(&it.title, &it.original_title);
        let wanted: Vec<String> = variants.clone();
        let mut all: Vec<Candidate> = Vec::new();
        let mut attempts: Vec<(String, i32, String)> = Vec::new();
        for (i, v) in variants.iter().take(4).enumerate() {
            if it.year > 0 {
                attempts.push((v.clone(), it.year, tmdb.language.clone()));
            }
            if i == 0 || it.year == 0 {
                attempts.push((v.clone(), 0, tmdb.language.clone()));
            }
        }
        if !tmdb.language.starts_with("en") {
            attempts.push((it.title.clone(), 0, "en-US".into()));
        }
        for (query, year, lang) in attempts {
            all.extend(self.search(tmdb, kind, &query, year, &lang).await?);
            if let Some((id, score)) = matching::best(&all, &wanted, it.year)
                && score >= SURE_SCORE
            {
                return Ok(Some(id));
            }
        }
        Ok(matching::best(&all, &wanted, it.year).map(|b| b.0))
    }

    async fn search(&self, tmdb: &Tmdb, kind: &str, query: &str, year: i32, lang: &str) -> MetaResult<Vec<Candidate>> {
        let y = year.to_string();
        let mut p: Vec<(&str, &str)> = vec![("query", query), ("language", lang), ("include_adult", "false")];
        if year > 0 {
            p.push((if kind == "tv" { "first_air_date_year" } else { "year" }, &y));
        }
        let Some(r) = tmdb.get(&format!("/search/{kind}"), &p).await? else { return Ok(Vec::new()) };
        let tv = kind == "tv";
        Ok(list(&r["results"])
            .iter()
            .take(20)
            .map(|m| Candidate {
                id: m["id"].as_i64().unwrap_or(0),
                title: s(m, if tv { "name" } else { "title" }),
                original_title: s(m, if tv { "original_name" } else { "original_title" }),
                year: year_of(&s(m, if tv { "first_air_date" } else { "release_date" })),
                popularity: m["popularity"].as_f64().unwrap_or(0.0),
                vote_count: m["vote_count"].as_i64().unwrap_or(0),
            })
            .filter(|c| c.id > 0)
            .collect())
    }

    /// TVmaze knows many shows under the names their files use; its IMDb or TVDB id then leads
    /// to the TMDB entry.
    async fn find_via_tvmaze(&self, tmdb: &Tmdb, show: &Item) -> MetaResult<Option<i64>> {
        let Some(tvmaze) = &self.tvmaze else { return Ok(None) };
        let found = tvmaze.search(&show.title).await;
        let candidates: Vec<Candidate> = found
            .iter()
            .enumerate()
            .map(|(i, s)| Candidate {
                id: i as i64,
                title: s.name.clone(),
                original_title: String::new(),
                year: s.year,
                popularity: 0.0,
                vote_count: 1,
            })
            .collect();
        let Some((i, _)) = matching::best(&candidates, std::slice::from_ref(&show.title), show.year) else { return Ok(None) };
        let ids = &found[i as usize];
        if let Some(imdb) = &ids.imdb
            && let Some(id) = self.find_external(tmdb, imdb, "imdb_id", "tv").await?
        {
            return Ok(Some(id));
        }
        if let Some(tvdb) = ids.tvdb {
            return self.find_external(tmdb, &tvdb.to_string(), "tvdb_id", "tv").await;
        }
        Ok(None)
    }

    /// Search for "fix match": TMDB's results for a query.
    pub async fn search_for_match(&self, kind: Kind, query: &str, year: i32) -> MetaResult<Vec<Value>> {
        let tmdb = self.tmdb.as_ref().ok_or_else(|| MetaError::new("error.tmdbNoKey"))?;
        let k = if kind == Kind::Movie { "movie" } else { "tv" };
        let y = year.to_string();
        let mut p: Vec<(&str, &str)> = vec![("query", query), ("language", &tmdb.language)];
        if year > 0 {
            p.push((if k == "tv" { "first_air_date_year" } else { "year" }, &y));
        }
        let Some(r) = tmdb.get(&format!("/search/{k}"), &p).await? else { return Ok(Vec::new()) };
        let tv = k == "tv";
        Ok(list(&r["results"])
            .iter()
            .take(12)
            .map(|m| {
                serde_json::json!({
                    "id": m["id"].as_i64().unwrap_or(0),
                    "title": s(m, if tv { "name" } else { "title" }),
                    "originalTitle": s(m, if tv { "original_name" } else { "original_title" }),
                    "year": year_of(&s(m, if tv { "first_air_date" } else { "release_date" })),
                    "summary": s(m, "overview"),
                    "poster": opt(m, "poster_path"),
                })
            })
            .collect())
    }

    /// A small TMDB poster for the "fix match" results, cached.
    pub async fn search_image(&self, tmdb_path: &str) -> Option<PathBuf> {
        let tmdb = self.tmdb.as_ref()?;
        let ok = tmdb_path.ends_with(".jpg") || tmdb_path.ends_with(".png");
        let target = self.dir.join("search").join(tmdb_path.trim_start_matches('/'));
        (ok && tmdb.download(tmdb_path, "w154", &target).await).then_some(target)
    }

    // ------------------------------------------------------------------ fetching

    async fn image(&self, tmdb: &Tmdb, path: Option<String>, size: &str, target: PathBuf) -> Option<String> {
        let p = path?;
        tmdb.download(&p, size, &target).await.then(|| target.to_string_lossy().into_owned())
    }

    async fn fetch_person(&self, tmdb: &Tmdb, c: &Value, role: String) -> Person {
        let id = c["id"].as_i64().unwrap_or(0);
        let image = self.image(tmdb, opt(c, "profile_path"), "w185", self.dir.join("people").join(format!("{id}.jpg"))).await;
        Person { id, name: s(c, "name"), role, image }
    }

    async fn credits(&self, tmdb: &Tmdb, credits: &Value, n: &mut Metadata) {
        for c in list(&credits["cast"]).iter().take(CAST_SIZE) {
            let p = self.fetch_person(tmdb, c, s(c, "character")).await;
            n.cast.push(p);
        }
        for c in list(&credits["crew"]) {
            let (job, name) = (s(c, "job"), s(c, "name"));
            if job == "Director" && !n.directors.contains(&name) {
                n.directors.push(name.clone());
            }
            if matches!(job.as_str(), "Screenplay" | "Writer" | "Novel") && n.writers.len() < 4 && !n.writers.contains(&name) {
                n.writers.push(name);
            }
        }
    }

    fn trailer(&self, tmdb: &Tmdb, videos: &Value) -> String {
        let mut best = (String::new(), -1);
        for v in list(&videos["results"]) {
            if s(v, "site") != "YouTube" {
                continue;
            }
            let key = s(v, "key");
            let valid = (6..=20).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
            let mut score = match s(v, "type").as_str() {
                "Trailer" => 4,
                "Teaser" => 2,
                _ => 0,
            };
            if s(v, "iso_639_1") == tmdb.lang2() {
                score += 3;
            }
            if v["official"].as_bool() == Some(true) {
                score += 1;
            }
            if valid && score > best.1 {
                best = (key, score);
            }
        }
        best.0
    }

    fn cert_label(&self, cert: &str) -> String {
        if self.region == "DE" && !cert.is_empty() && cert.bytes().all(|b| b.is_ascii_digit()) {
            format!("FSK {cert}")
        } else {
            cert.to_string()
        }
    }

    async fn english_overview(&self, tmdb: &Tmdb, path: &str) -> String {
        if tmdb.language.starts_with("en") {
            return String::new();
        }
        tmdb.get(path, &[("language", "en-US")]).await.ok().flatten().map(|v| s(&v, "overview")).unwrap_or_default()
    }

    async fn fetch_movie(&self, app: &AppState, tmdb: &Tmdb, it: &Item, id: i64) -> MetaResult<bool> {
        let videos = format!("{},en,null", tmdb.lang2());
        let path = format!("/movie/{id}");
        let params = [
            ("language", tmdb.language.as_str()),
            ("append_to_response", "credits,videos,release_dates"),
            ("include_video_language", &videos),
        ];
        let Some(m) = tmdb.get(&path, &params).await? else { return Ok(false) };
        let mut n = Metadata {
            tmdb_id: id,
            title: s(&m, "title"),
            original_title: s(&m, "original_title"),
            summary: s(&m, "overview"),
            tagline: s(&m, "tagline"),
            year: year_of(&s(&m, "release_date")),
            air_date: s(&m, "release_date"),
            genres: names(&m["genres"]),
            rating: round1(&m["vote_average"]),
            studio: list(&m["production_companies"]).first().map(|c| s(c, "name")).unwrap_or_default(),
            trailer_key: self.trailer(tmdb, &m["videos"]),
            updated_at: now_ms(),
            ..Metadata::default()
        };
        if n.summary.is_empty() {
            n.summary = self.english_overview(tmdb, &path).await;
        }
        // Age rating of the region, US as fallback.
        let mut fallback = String::new();
        'outer: for r in list(&m["release_dates"]["results"]) {
            let country = s(r, "iso_3166_1");
            for d in list(&r["release_dates"]) {
                let cert = s(d, "certification").trim().to_string();
                if cert.is_empty() {
                    continue;
                }
                if country == self.region {
                    n.content_rating = self.cert_label(&cert);
                    break 'outer;
                }
                if country == "US" && fallback.is_empty() {
                    fallback = cert;
                }
            }
        }
        if n.content_rating.is_empty() {
            n.content_rating = fallback;
        }
        if let Some(cid) = m["belongs_to_collection"]["id"].as_i64() {
            n.collection_id = cid;
            n.collection_name = s(&m["belongs_to_collection"], "name");
            self.fetch_collection(app, tmdb, cid).await;
        }
        self.credits(tmdb, &m["credits"], &mut n).await;
        let dir = self.dir.join("items").join(&it.id);
        n.poster = self.image(tmdb, opt(&m, "poster_path"), "w500", dir.join("poster.jpg")).await;
        n.backdrop = self.image(tmdb, opt(&m, "backdrop_path"), "w1280", dir.join("backdrop.jpg")).await;
        self.put(&app.db, &it.id, n);
        Ok(true)
    }

    async fn fetch_collection(&self, app: &AppState, tmdb: &Tmdb, id: i64) {
        if self.collections.read().unwrap().contains_key(&id) {
            return;
        }
        let Ok(Some(c)) = tmdb.get(&format!("/collection/{id}"), &[("language", &tmdb.language)]).await else { return };
        let dir = self.dir.join("collections").join(id.to_string());
        let coll = Collection {
            name: s(&c, "name"),
            summary: s(&c, "overview"),
            poster: self.image(tmdb, opt(&c, "poster_path"), "w500", dir.join("poster.jpg")).await,
            backdrop: self.image(tmdb, opt(&c, "backdrop_path"), "w1280", dir.join("backdrop.jpg")).await,
        };
        app.db.save_tmdb_collection(id, &serde_json::to_string(&coll).expect("serializable"));
        self.collections.write().unwrap().insert(id, coll);
    }

    async fn fetch_show(&self, app: &AppState, tmdb: &Tmdb, show: &Item, id: i64) -> MetaResult<bool> {
        let videos = format!("{},en,null", tmdb.lang2());
        let path = format!("/tv/{id}");
        let params = [
            ("language", tmdb.language.as_str()),
            ("append_to_response", "credits,videos,content_ratings"),
            ("include_video_language", &videos),
        ];
        let Some(m) = tmdb.get(&path, &params).await? else { return Ok(false) };
        let mut n = Metadata {
            tmdb_id: id,
            title: s(&m, "name"),
            original_title: s(&m, "original_name"),
            summary: s(&m, "overview"),
            tagline: s(&m, "tagline"),
            year: year_of(&s(&m, "first_air_date")),
            air_date: s(&m, "first_air_date"),
            genres: names(&m["genres"]),
            rating: round1(&m["vote_average"]),
            studio: list(&m["networks"]).first().map(|c| s(c, "name")).unwrap_or_default(),
            trailer_key: self.trailer(tmdb, &m["videos"]),
            updated_at: now_ms(),
            ..Metadata::default()
        };
        if n.summary.is_empty() {
            n.summary = self.english_overview(tmdb, &path).await;
        }
        for r in list(&m["content_ratings"]["results"]) {
            if s(r, "iso_3166_1") == self.region {
                n.content_rating = self.cert_label(&s(r, "rating"));
            }
        }
        self.credits(tmdb, &m["credits"], &mut n).await;
        n.directors.extend(list(&m["created_by"]).iter().map(|c| s(c, "name")));
        let dir = self.dir.join("items").join(&show.id);
        n.poster = self.image(tmdb, opt(&m, "poster_path"), "w500", dir.join("poster.jpg")).await;
        n.backdrop = self.image(tmdb, opt(&m, "backdrop_path"), "w1280", dir.join("backdrop.jpg")).await;
        self.put(&app.db, &show.id, n);

        // Episode counts of the TMDB seasons, for absolute episode numbers ("Episode 105").
        let counts: Vec<(i32, i32)> = list(&m["seasons"])
            .iter()
            .map(|x| (x["season_number"].as_i64().unwrap_or(-1) as i32, x["episode_count"].as_i64().unwrap_or(0) as i32))
            .filter(|(s, _)| *s > 0)
            .collect();
        let mut seasons_cache: HashMap<i32, Option<Value>> = HashMap::new();
        let idx = app.catalog.snapshot();
        for season in idx.children(&show.id) {
            let sv = self.season(tmdb, id, season.index, &mut seasons_cache).await?;
            if let Some(sv) = &sv {
                let mut title = if season.index == 0 { "Specials".to_string() } else { s(sv, "name") };
                let generic = title.split_once(' ').is_some_and(|(w, n)| {
                    ["season", "staffel"].contains(&w.to_lowercase().as_str()) && n.bytes().all(|b| b.is_ascii_digit())
                });
                if generic {
                    title = format!("Staffel {}", season.index);
                }
                let sm = Metadata {
                    tmdb_id: sv["id"].as_i64().unwrap_or(0),
                    title,
                    summary: s(sv, "overview"),
                    year: year_of(&s(sv, "air_date")),
                    air_date: s(sv, "air_date"),
                    poster: self
                        .image(tmdb, opt(sv, "poster_path"), "w500", self.dir.join("items").join(&season.id).join("poster.jpg"))
                        .await,
                    updated_at: now_ms(),
                    ..Metadata::default()
                };
                self.put(&app.db, &season.id, sm);
            }
            for ep in idx.children(&season.id) {
                let mut found = sv.as_ref().and_then(|v| episode_of(v, ep.index)).cloned();
                // Absolute numbering: "Episode 105" in a single folder.
                if found.is_none()
                    && season.index == 1
                    && ep.index > 0
                    && let Some((sn, en)) = absolute(&counts, ep.index)
                {
                    let other = self.season(tmdb, id, sn, &mut seasons_cache).await?;
                    found = other.as_ref().and_then(|v| episode_of(v, en)).cloned();
                }
                let Some(e) = found else { continue };
                let mut em = Metadata {
                    tmdb_id: e["id"].as_i64().unwrap_or(0),
                    title: s(&e, "name"),
                    summary: s(&e, "overview"),
                    air_date: s(&e, "air_date"),
                    year: year_of(&s(&e, "air_date")),
                    rating: round1(&e["vote_average"]),
                    updated_at: now_ms(),
                    ..Metadata::default()
                };
                for c in list(&e["crew"]) {
                    if s(c, "job") == "Director" {
                        em.directors.push(s(c, "name"));
                    }
                    if s(c, "department") == "Writing" && em.writers.len() < 3 {
                        em.writers.push(s(c, "name"));
                    }
                }
                for c in list(&e["guest_stars"]).iter().take(10) {
                    let p = self.fetch_person(tmdb, c, s(c, "character")).await;
                    em.cast.push(p);
                }
                em.poster = self.image(tmdb, opt(&e, "still_path"), "w500", self.dir.join("items").join(&ep.id).join("still.jpg")).await;
                self.put(&app.db, &ep.id, em);
            }
        }
        Ok(true)
    }

    async fn season(&self, tmdb: &Tmdb, show: i64, n: i32, cache: &mut HashMap<i32, Option<Value>>) -> MetaResult<Option<Value>> {
        if let Some(v) = cache.get(&n) {
            return Ok(v.clone());
        }
        let v = tmdb.get(&format!("/tv/{show}/season/{n}"), &[("language", &tmdb.language)]).await?;
        cache.insert(n, v.clone());
        Ok(v)
    }
}

fn episode_of(season: &Value, number: i32) -> Option<&Value> {
    list(&season["episodes"]).iter().find(|e| e["episode_number"].as_i64() == Some(i64::from(number)))
}

/// Season and episode for an absolute episode number, from the episode counts of the seasons;
/// None when the number is within the first season (then it is not absolute).
fn absolute(counts: &[(i32, i32)], number: i32) -> Option<(i32, i32)> {
    let mut rest = number;
    for (i, &(season, count)) in counts.iter().enumerate() {
        if rest <= count {
            return (i > 0).then_some((season, rest));
        }
        rest -= count;
    }
    None
}

/// The movie or show an entry belongs to.
fn top_of(app: &AppState, it: &Item) -> Item {
    let idx = app.catalog.snapshot();
    let mut cur = idx.item(&it.id).map_or_else(|| it.clone(), |x| Item::clone(x));
    while let Some(p) = idx.parent(&cur) {
        cur = Item::clone(p);
    }
    cur
}

/// Merges metadata into an entry. Data from a `.nfo` file wins; online data then only fills gaps.
pub fn apply(it: &mut Item, md: &Metadata) {
    let fill_only = it.has_nfo;
    let generic_episode = it.kind == Kind::Episode && it.title.starts_with("Episode ");
    if !md.title.is_empty() && (!fill_only || it.title.is_empty() || generic_episode) {
        it.title = md.title.clone();
        match it.kind {
            Kind::Episode => it.sort_title = format!("{:04} {}", it.index, sort_title(&it.title)),
            Kind::Season => {}
            _ => it.sort_title = sort_title(&it.title),
        }
    }
    if !md.original_title.is_empty() && md.original_title != it.title && (!fill_only || it.original_title.is_empty()) {
        it.original_title = md.original_title.clone();
    }
    if !md.summary.is_empty() && (!fill_only || it.summary.is_empty()) {
        it.summary = md.summary.clone();
    }
    if !md.tagline.is_empty() && (!fill_only || it.tagline.is_empty()) {
        it.tagline = md.tagline.clone();
    }
    if md.year > 0 && (it.year == 0 || (!fill_only && it.kind != Kind::Episode)) {
        it.year = md.year;
    }
    if !md.genres.is_empty() && (!fill_only || it.genres.is_empty()) {
        it.genres = md.genres.clone();
    }
    if md.rating > 0.0 && (!fill_only || it.rating == 0.0) {
        it.rating = md.rating;
    }
    if !md.content_rating.is_empty() && (!fill_only || it.content_rating.is_empty()) {
        it.content_rating = md.content_rating.clone();
    }
    let exists = |p: &Option<String>| p.as_deref().is_some_and(|p| Path::new(p).is_file());
    // Own artwork next to the files wins; online artwork replaces only missing or online artwork.
    let online = |p: &Option<String>| p.as_deref().is_some_and(|p| p.contains(&format!("{0}metadata{0}", std::path::MAIN_SEPARATOR)));
    if (it.poster_path.is_none() || online(&it.poster_path)) && exists(&md.poster) {
        it.poster_path = md.poster.clone();
    }
    if (it.backdrop_path.is_none() || online(&it.backdrop_path)) && exists(&md.backdrop) {
        it.backdrop_path = md.backdrop.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_numbers() {
        let counts = [(1, 12), (2, 10), (3, 8)];
        assert_eq!(absolute(&counts, 5), None);
        assert_eq!(absolute(&counts, 13), Some((2, 1)));
        assert_eq!(absolute(&counts, 25), Some((3, 3)));
        assert_eq!(absolute(&counts, 40), None);
    }

    #[test]
    fn nfo_wins() {
        let mut it = Item::new("x".into(), "l", Kind::Movie);
        it.title = "Mein Titel".into();
        it.has_nfo = true;
        let md = Metadata { title: "TMDB Titel".into(), summary: "Text".into(), year: 2001, ..Metadata::default() };
        apply(&mut it, &md);
        assert_eq!((it.title.as_str(), it.summary.as_str(), it.year), ("Mein Titel", "Text", 2001));
        let mut ep = Item::new("e".into(), "l", Kind::Episode);
        ep.title = "Episode 3".into();
        ep.index = 3;
        ep.has_nfo = true;
        apply(&mut ep, &md);
        assert_eq!(ep.title, "TMDB Titel");
        assert_eq!(ep.sort_title, "0003 tmdb titel");
    }
}
