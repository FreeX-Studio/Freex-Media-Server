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

//! Browsing: library contents, the rows of the home screen and of a library, item pages,
//! search, artwork, plus the folder picker and scan triggers for admins.

use crate::AppState;
use crate::db::{Library, Progress, User};
use crate::http::{ApiError, ApiResult, Ctx, ok};
use crate::i18n::Text;
use crate::media::catalog::Index;
use crate::media::scanner::Scanner;
use crate::media::{Item, Kind};
use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Entries per row.
const HUB_SIZE: usize = 24;
pub(super) const IMAGE_CACHE_SECONDS: u32 = 86_400;
const SEARCH_LIMIT: usize = 40;

type Params = Query<HashMap<String, String>>;

/// The index as one user sees it, with their watch state.
pub(super) struct View<'a> {
    app: &'a AppState,
    idx: &'a Index,
    progress: HashMap<String, Progress>,
    watchlist: HashSet<String>,
}

impl<'a> View<'a> {
    pub(super) fn new(app: &'a AppState, idx: &'a Index, user: &User) -> View<'a> {
        View { app, idx, progress: app.db.progress_of(&user.id), watchlist: app.db.watchlist(&user.id).into_iter().collect() }
    }

    fn progress(&self, id: &str) -> Progress {
        self.progress.get(id).copied().unwrap_or_default()
    }

    /// Item as JSON with watch state, show and season of episodes and episode counts.
    pub(super) fn decorate(&self, it: &Item, full: bool) -> Value {
        let mut m = it.to_public();
        m["artVersion"] = json!(self.art_version(it));
        if !full && it.kind != Kind::Movie {
            m.as_object_mut().expect("object").remove("summary");
        }
        if it.kind.playable() {
            m["progress"] = self.progress(&it.id).to_json();
        }
        if matches!(it.kind, Kind::Movie | Kind::Show) {
            m["onWatchlist"] = json!(self.watchlist.contains(&it.id));
        }
        if it.kind == Kind::Episode
            && let Some(season) = self.idx.parent(it)
        {
            m["seasonIndex"] = json!(season.index);
            m["seasonId"] = json!(season.id);
        }
        if let Some(show) = self.idx.show_of(it) {
            m["showTitle"] = json!(show.title);
            m["showId"] = json!(show.id);
        }
        if matches!(it.kind, Kind::Show | Kind::Season) {
            let (leaves, viewed) = self.count_leaves(it);
            m["leafCount"] = json!(leaves);
            m["viewedLeafCount"] = json!(viewed);
            m["childCount"] = json!(self.idx.children(&it.id).len());
        }
        if full {
            super::metadata::extend(self.app, self.idx, it, &mut m);
            if it.kind == Kind::Episode
                && let Some(mk) = self.app.markers.as_ref().and_then(|s| s.get(it))
            {
                let seg = |s: crate::markers::Segment| json!({"startMs": s.start_ms, "endMs": s.end_ms});
                let mut mm = json!({});
                if let Some(i) = mk.intro {
                    mm["intro"] = seg(i);
                }
                if let Some(c) = mk.credits {
                    mm["credits"] = seg(c);
                }
                m["markers"] = mm;
            }
            if let Some(l) = self.app.previews.as_ref().and_then(|p| p.layout(it)) {
                let mut v = serde_json::to_value(l).expect("serializable");
                // Changes with the file, for the image URL.
                v["version"] = json!(format!("{:x}", it.file.as_ref().map_or(0, |f| f.modified)));
                m["previews"] = v;
            }
        }
        m
    }

    pub(super) fn decorate_all<'b>(&self, items: impl Iterator<Item = &'b Arc<Item>>) -> Vec<Value> {
        items.map(|it| self.decorate(it, false)).collect()
    }

    /// Changes whenever the artwork of an item or its parents changes; FreeX Web adds it to image
    /// URLs, so browsers do not keep showing an old picture.
    fn art_version(&self, it: &Item) -> String {
        let mut parts = String::new();
        let mut cur = Some(it);
        for _ in 0..3 {
            let Some(c) = cur else { break };
            parts.push_str(c.poster_path.as_deref().unwrap_or("-"));
            parts.push('\0');
            parts.push_str(c.backdrop_path.as_deref().unwrap_or("-"));
            parts.push('\0');
            cur = self.idx.parent(c).map(Arc::as_ref);
        }
        crate::auth::sha256(&parts)[..8].to_string()
    }

    /// Episodes below a show or season, and how many of them are watched.
    fn count_leaves(&self, it: &Item) -> (usize, usize) {
        let mut counts = (0, 0);
        for c in self.idx.children(&it.id) {
            if c.kind == Kind::Episode {
                counts.0 += 1;
                if self.progress(&c.id).watched {
                    counts.1 += 1;
                }
            } else {
                let (a, b) = self.count_leaves(c);
                counts = (counts.0 + a, counts.1 + b);
            }
        }
        counts
    }

    fn fully_watched(&self, it: &Item) -> bool {
        if it.kind.playable() {
            self.progress(&it.id).watched
        } else {
            let (leaves, viewed) = self.count_leaves(it);
            leaves > 0 && leaves == viewed
        }
    }

    fn any_watched(&self, it: &Item) -> bool {
        if it.kind.playable() { self.progress(&it.id).touched() } else { self.count_leaves(it).1 > 0 }
    }

    /// First unwatched episode after the last watched one ("Up next").
    fn next_episode(&self, show: &Item) -> Option<Arc<Item>> {
        let eps = self.idx.episodes_of(show);
        let after = eps.iter().rposition(|e| self.progress(&e.id).watched).map_or(0, |i| i + 1);
        eps[after..].iter().find(|e| !self.progress(&e.id).watched).cloned()
    }

    /// The episode after this one, for "play next".
    fn following_episode(&self, episode: &Item) -> Option<Arc<Item>> {
        if episode.kind != Kind::Episode {
            return None;
        }
        let eps = self.idx.episodes_of(self.idx.show_of(episode)?);
        let i = eps.iter().position(|e| e.id == episode.id)?;
        eps.get(i + 1).cloned()
    }

    /// Started, unfinished items (newest first), optionally of one library.
    fn started(&self, user: &User, library: Option<&str>) -> Vec<Arc<Item>> {
        let mut list: Vec<(&Arc<Item>, i64)> = self
            .progress
            .iter()
            .filter(|(_, p)| !p.watched && p.offset_ms > 0)
            .filter_map(|(id, p)| self.idx.item(id).map(|it| (it, p.updated_at)))
            .filter(|(it, _)| user.may_access(&it.library_id) && library.is_none_or(|l| it.library_id == l))
            .collect();
        list.sort_by_key(|e| std::cmp::Reverse(e.1));
        list.into_iter().map(|(it, _)| it.clone()).collect()
    }
}

/// A row; the title is already in the user's language (library and genre names stay as they are).
fn hub(key: &str, title: String, library_id: Option<&str>, style: &str, items: Vec<Value>) -> Value {
    json!({"key": key, "title": title, "libraryId": library_id, "style": style, "items": items})
}

fn by_title(a: &Arc<Item>, b: &Arc<Item>) -> Ordering {
    a.sort_title.cmp(&b.sort_title)
}

/// The library of a path parameter if the user may see it; 404 otherwise.
fn visible_library(app: &AppState, user: &User, id: &str) -> Result<Library, ApiError> {
    app.db.library(id).filter(|l| user.may_access(&l.id)).ok_or_else(ApiError::not_found)
}

/// An item the user may see; 404 otherwise, as if it did not exist.
pub(super) fn visible_library_of(app: &AppState, user: &User, id: &str) -> Result<Library, ApiError> {
    visible_library(app, user, id)
}

pub(super) fn visible_item(idx: &Index, user: &User, id: &str) -> Result<Arc<Item>, ApiError> {
    idx.item(id).filter(|it| user.may_access(&it.library_id)).cloned().ok_or_else(ApiError::not_found)
}

pub fn library_view(app: &AppState, l: &Library, with_paths: bool) -> Value {
    l.to_public(app.catalog.snapshot().top_level(&l.id).len() as i64, app.scanner.is_scanning(&l.id), with_paths)
}

// ====================================================================== library contents

pub async fn library_items(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    Query(q): Params,
) -> ApiResult {
    let u = ctx.user()?;
    let l = visible_library(&app, u, &id)?;
    let idx = app.catalog.snapshot();
    let view = View::new(&app, &idx, u);
    let sort = q.get("sort").map_or("title", String::as_str);
    let desc = q.get("order").map_or(sort != "title", |o| o == "desc");
    let unwatched_only = q.get("unwatched").is_some_and(|v| v == "1");
    let genre = q.get("genre").map_or("", String::as_str);

    let mut items: Vec<Arc<Item>> = idx.top_level(&l.id).to_vec();
    let cmp = |a: &Arc<Item>, b: &Arc<Item>| -> Ordering {
        match sort {
            "added" => a.added_at.cmp(&b.added_at),
            "year" => a.year.cmp(&b.year).then_with(|| by_title(a, b)),
            "rating" => a.rating.total_cmp(&b.rating).then_with(|| by_title(a, b)),
            "duration" => a.duration_ms().cmp(&b.duration_ms()),
            _ => by_title(a, b),
        }
    };
    items.sort_by(|a, b| if desc { cmp(b, a) } else { cmp(a, b) });

    let mut genres = BTreeSet::new();
    let mut out = Vec::new();
    for it in &items {
        genres.extend(it.genre_list().map(str::to_string));
        if !genre.is_empty() && !it.has_genre(genre) {
            continue;
        }
        if unwatched_only && view.fully_watched(it) {
            continue;
        }
        out.push(view.decorate(it, false));
    }
    ok(json!({"library": library_view(&app, &l, u.admin), "genres": genres, "items": out}))
}

pub async fn item(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let it = visible_item(&idx, u, &id)?;
    let view = View::new(&app, &idx, u);
    let mut m = view.decorate(&it, true);
    if matches!(it.kind, Kind::Show | Kind::Season) {
        m["children"] = json!(view.decorate_all(idx.children(&it.id).iter()));
        if it.kind == Kind::Show
            && let Some(next) = view.next_episode(&it)
        {
            m["onDeck"] = view.decorate(&next, false);
        }
    }
    if let Some(next) = view.following_episode(&it) {
        m["next"] = view.decorate(&next, false);
    }
    ok(m)
}

pub async fn children(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let it = visible_item(&idx, u, &id)?;
    let view = View::new(&app, &idx, u);
    ok(json!(view.decorate_all(idx.children(&it.id).iter())))
}

// ====================================================================== rows

/// The home screen: continue watching, up next, recently added per library.
pub async fn hubs(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let view = View::new(&app, &idx, u);
    let mut hubs = Vec::new();

    let started = view.started(u, None);
    let shows_in_progress: HashSet<&str> = started.iter().take(HUB_SIZE).filter_map(|it| idx.show_of(it)).map(|s| s.id.as_str()).collect();
    let cont = view.decorate_all(started.iter().take(HUB_SIZE));
    if !cont.is_empty() {
        hubs.push(hub("continue", ctx.t(&Text::new("hub.continue")), None, "landscape", cont));
    }

    // Up next: shows with watched episodes, ordered by the last activity.
    let mut activity: HashMap<&str, i64> = HashMap::new();
    for (id, p) in view.progress.iter().filter(|(_, p)| p.watched) {
        let Some(it) = idx.item(id).filter(|it| it.kind == Kind::Episode && u.may_access(&it.library_id)) else { continue };
        if let Some(show) = idx.show_of(it) {
            let e = activity.entry(show.id.as_str()).or_insert(p.updated_at);
            *e = (*e).max(p.updated_at);
        }
    }
    let mut shows: Vec<(&str, i64)> = activity.into_iter().collect();
    shows.sort_by_key(|e| std::cmp::Reverse(e.1));
    let mut deck = Vec::new();
    for (show_id, _) in shows {
        if deck.len() >= HUB_SIZE {
            break;
        }
        if shows_in_progress.contains(show_id) {
            continue;
        }
        if let Some(next) = idx.item(show_id).and_then(|s| view.next_episode(s)) {
            deck.push(view.decorate(&next, false));
        }
    }
    if !deck.is_empty() {
        hubs.push(hub("ondeck", ctx.t(&Text::new("hub.upNext")), None, "landscape", deck));
    }

    let watchlist = super::lists::watchlist_items(&app, &idx, u, HUB_SIZE);
    if !watchlist.is_empty() {
        let mut h = hub("watchlist", ctx.t(&Text::new("hub.watchlist")), None, "poster", watchlist);
        h["href"] = json!("/watchlist");
        hubs.push(h);
    }

    for l in app.db.libraries().iter().filter(|l| u.may_access(&l.id)) {
        let mut top: Vec<&Arc<Item>> = idx.top_level(&l.id).iter().collect();
        top.sort_by_key(|it| std::cmp::Reverse(it.added_at));
        let recent = view.decorate_all(top.into_iter().take(HUB_SIZE));
        if !recent.is_empty() {
            let title = ctx.t(&Text::new("hub.recentlyAddedIn").arg(l.name.clone()));
            hubs.push(hub(&format!("recent-{}", l.id), title, Some(&l.id), "poster", recent));
        }
    }
    ok(json!(hubs))
}

/// The "Recommended" tab of a library: rows built from its own content.
pub async fn library_hubs(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let l = visible_library(&app, u, &id)?;
    let idx = app.catalog.snapshot();
    let view = View::new(&app, &idx, u);
    let top = idx.top_level(&l.id);
    let mut hubs = Vec::new();

    let cont = view.decorate_all(view.started(u, Some(&l.id)).iter().take(HUB_SIZE));
    if !cont.is_empty() {
        hubs.push(hub("continue", ctx.t(&Text::new("hub.continue")), Some(&l.id), "landscape", cont));
    }
    if l.kind == "shows" {
        let mut deck = Vec::new();
        for show in top {
            if deck.len() >= HUB_SIZE {
                break;
            }
            let (leaves, viewed) = view.count_leaves(show);
            if viewed == 0 || viewed == leaves {
                continue; // not started or finished
            }
            if let Some(next) = view.next_episode(show) {
                deck.push(view.decorate(&next, false));
            }
        }
        if !deck.is_empty() {
            hubs.push(hub("ondeck", ctx.t(&Text::new("hub.upNext")), Some(&l.id), "landscape", deck));
        }
    }

    let mut sorted =
        |key: &str, title: String, sort: Option<&str>, order: &dyn Fn(&Arc<Item>, &Arc<Item>) -> Ordering, keep: &dyn Fn(&Item) -> bool| {
            let mut list: Vec<&Arc<Item>> = top.iter().filter(|it| keep(it)).collect();
            if list.len() < 2 {
                return;
            }
            list.sort_by(|a, b| order(a, b));
            let mut h = hub(key, title, Some(&l.id), "poster", view.decorate_all(list.into_iter().take(HUB_SIZE)));
            h["sort"] = json!(sort);
            hubs.push(h);
        };
    sorted("recent", ctx.t(&Text::new("hub.recentlyAdded")), Some("added"), &|a, b| b.added_at.cmp(&a.added_at), &|_| true);
    sorted(
        "released",
        ctx.t(&Text::new("hub.recentlyReleased")),
        Some("year"),
        &|a, b| b.year.cmp(&a.year).then_with(|| by_title(a, b)),
        &|it| it.year > 0,
    );
    let unwatched_title = ctx.t(&Text::new(if l.kind == "shows" { "hub.notStarted" } else { "hub.unwatched" }));
    sorted("unwatched", unwatched_title, None, &|a, b| b.added_at.cmp(&a.added_at), &|it| !view.any_watched(it));
    sorted("rated", ctx.t(&Text::new("hub.topRated")), Some("rating"), &|a, b| b.rating.total_cmp(&a.rating), &|it| it.rating > 0.0);

    // The three most common genres get their own row.
    let mut genre_count: HashMap<String, usize> = HashMap::new();
    for it in top {
        for g in it.genre_list() {
            *genre_count.entry(g.to_string()).or_default() += 1;
        }
    }
    let mut genres: Vec<(String, usize)> = genre_count.into_iter().filter(|(_, n)| *n >= 3).collect();
    genres.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (g, _) in genres.into_iter().take(3) {
        sorted(&format!("genre-{g}"), g.clone(), None, &by_title, &|it| it.has_genre(&g));
    }
    // Movie series, before the genre rows.
    let collections = super::lists::collections_of(&app, &idx, u, &l.id);
    if !collections.is_empty() {
        let genre_rows = hubs.iter().filter(|h| h["key"].as_str().is_some_and(|k| k.starts_with("genre-"))).count();
        let mut h = hub("collections", ctx.t(&Text::new("hub.collections")), Some(&l.id), "collection", collections);
        h["href"] = json!(format!("/library/{}/collections", l.id));
        hubs.insert(hubs.len() - genre_rows, h);
    }
    ok(json!(hubs))
}

// ====================================================================== search

pub async fn search(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Query(q): Params) -> ApiResult {
    let u = ctx.user()?;
    let query = q.get("q").map(|s| s.trim().to_lowercase()).unwrap_or_default();
    let (mut movies, mut shows, mut episodes) = (Vec::new(), Vec::new(), Vec::new());
    if query.chars().count() >= 2 {
        let idx = app.catalog.snapshot();
        let view = View::new(&app, &idx, u);
        let mut hits: Vec<&Arc<Item>> = idx
            .all()
            .filter(|it| u.may_access(&it.library_id))
            .filter(|it| it.title.to_lowercase().contains(&query) || it.original_title.to_lowercase().contains(&query))
            .collect();
        hits.sort_by(|a, b| by_title(a, b));
        for it in hits {
            let list = match it.kind {
                Kind::Movie => &mut movies,
                Kind::Show => &mut shows,
                Kind::Episode => &mut episodes,
                Kind::Season => continue,
            };
            if list.len() < SEARCH_LIMIT {
                list.push(view.decorate(it, false));
            }
        }
    }
    ok(json!({"movies": movies, "shows": shows, "episodes": episodes}))
}

// ====================================================================== artwork

/// Where an image comes from: a file, or a frame of a video.
enum Source {
    File(PathBuf),
    Frame(Arc<Item>),
}

fn existing(p: &Option<String>) -> Option<Source> {
    p.as_deref().map(PathBuf::from).filter(|p| p.is_file()).map(Source::File)
}

/// `poster` is portrait, `backdrop` the large background, `thumb` a landscape still. Missing art
/// falls back to the parent's art or a frame from the video.
fn poster(idx: &Index, it: &Arc<Item>) -> Option<Source> {
    if it.kind != Kind::Episode
        && let Some(s) = existing(&it.poster_path)
    {
        return Some(s);
    }
    match it.kind {
        Kind::Season | Kind::Episode => poster(idx, idx.parent(it)?),
        Kind::Movie => Some(Source::Frame(it.clone())),
        Kind::Show => thumb(idx, idx.episodes_of(it).first()?),
    }
}

fn backdrop(idx: &Index, it: &Arc<Item>) -> Option<Source> {
    if let Some(s) = existing(&it.backdrop_path) {
        return Some(s);
    }
    match it.kind {
        Kind::Season | Kind::Episode => backdrop(idx, idx.parent(it)?),
        Kind::Movie => Some(Source::Frame(it.clone())),
        Kind::Show => thumb(idx, idx.episodes_of(it).first()?),
    }
}

fn thumb(idx: &Index, it: &Arc<Item>) -> Option<Source> {
    match it.kind {
        Kind::Episode => existing(&it.poster_path).or_else(|| Some(Source::Frame(it.clone()))),
        Kind::Movie => existing(&it.backdrop_path).or_else(|| Some(Source::Frame(it.clone()))),
        _ => backdrop(idx, it),
    }
}

pub async fn image(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((id, kind)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult {
    let u = ctx.user()?;
    let it = visible_item(&app.catalog.snapshot(), u, &id)?;
    let path = image_path(&app, &it, &kind).await.ok_or_else(ApiError::not_found)?;
    crate::files::send(&path, IMAGE_CACHE_SECONDS, &headers).await
}

/// The seek preview sprite sheet of a movie or episode; its grid is `previews` in the item view.
pub async fn previews(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    let u = ctx.user()?;
    let it = visible_item(&app.catalog.snapshot(), u, &id)?;
    let (path, _) = app.previews.as_ref().and_then(|p| p.get(&it)).ok_or_else(ApiError::not_found)?;
    crate::files::send(&path, IMAGE_CACHE_SECONDS, &headers).await
}

/// The file of an item's artwork (`poster`, `backdrop`, `thumb`), made from the video if needed.
pub(super) async fn image_path(app: &AppState, it: &Arc<Item>, kind: &str) -> Option<PathBuf> {
    let source = {
        let idx = app.catalog.snapshot();
        match kind {
            "poster" => poster(&idx, it),
            "backdrop" => backdrop(&idx, it),
            "thumb" => thumb(&idx, it),
            _ => None,
        }
    };
    match source? {
        Source::File(p) => Some(p),
        Source::Frame(it) => app.thumbs.frame(&it).await,
    }
}

// ====================================================================== administration

pub async fn scan_library(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    ctx.admin()?;
    let l = app.db.library(&id).ok_or_else(ApiError::not_found)?;
    Scanner::scan(&app, &l.id);
    Ok((StatusCode::ACCEPTED, axum::Json(json!({"queued": true}))).into_response())
}

pub async fn scan_all(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    ctx.admin()?;
    Scanner::scan_all(&app);
    Ok((StatusCode::ACCEPTED, axum::Json(json!({"queued": true}))).into_response())
}

/// Folders of the server for the library dialog; without `path` the starting points.
pub async fn browse_folders(Extension(ctx): Extension<Ctx>, Query(q): Params) -> ApiResult {
    ctx.admin()?;
    let requested = q.get("path").cloned().unwrap_or_default();
    let out = tokio::task::spawn_blocking(move || list_folders(&requested)).await.map_err(|_| ApiError::not_found())??;
    ok(out)
}

fn list_folders(requested: &str) -> Result<Value, ApiError> {
    let entry = |name: &str, path: &str| json!({"name": name, "path": path});
    if requested.is_empty() {
        let mut seen = HashSet::new();
        let mut dirs = Vec::new();
        for root in roots() {
            if seen.insert(root.clone()) {
                dirs.push(entry(&root, &root));
            }
        }
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok();
        for common in home.into_iter().chain(["/mnt", "/media", "/srv", "/data"].map(String::from)) {
            if std::path::Path::new(&common).is_dir() && seen.insert(common.clone()) {
                dirs.push(entry(&common, &common));
            }
        }
        return Ok(json!({"path": "", "parent": null, "folders": dirs}));
    }
    let p = std::path::absolute(requested).map_err(|_| ApiError::not_found())?;
    if !p.is_dir() {
        return Err(ApiError::not_found());
    }
    let read = std::fs::read_dir(&p).map_err(|_| ApiError::new(StatusCode::FORBIDDEN, "error.folderUnreadable"))?;
    let mut dirs: Vec<(String, String)> = read
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().to_string_lossy().into_owned()))
        .filter(|(n, _)| !n.starts_with('.'))
        .collect();
    dirs.sort_by_key(|(n, _)| n.to_lowercase());
    dirs.truncate(2000);
    let parent = p.parent().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(json!({
        "path": p.to_string_lossy(),
        "parent": parent,
        "folders": dirs.iter().map(|(n, path)| entry(n, path)).collect::<Vec<_>>(),
    }))
}

/// File system roots: `/` on Unix, the drives on Windows.
fn roots() -> Vec<String> {
    if cfg!(windows) {
        (b'A'..=b'Z').map(|d| format!("{}:\\", d as char)).filter(|d| std::path::Path::new(d).is_dir()).collect()
    } else {
        vec!["/".to_string()]
    }
}
