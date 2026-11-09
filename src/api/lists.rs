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

//! Lists: collections of a library (TMDB movie series, derived, and hand-made ones), playlists
//! of each user and the watchlist.

use super::media::{IMAGE_CACHE_SECONDS, View, visible_item, visible_library_of};
use super::metadata::{collection_id, collection_name};
use crate::AppState;
use crate::db::{OwnCollection, User};
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, created, no_content, ok};
use crate::media::catalog::Index;
use crate::media::{Item, Kind};
use crate::metadata::Metadata;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;

const MAX_NAME: usize = 80;
const MAX_PLAYLISTS: usize = 200;
const MAX_PLAYLIST_ITEMS: usize = 5000;

fn valid_name(raw: &str, key: &str) -> Result<String, ApiError> {
    let n = raw.trim();
    if n.is_empty() || n.chars().count() > MAX_NAME {
        let e = ApiError::bad_request(key);
        return Err(if key == "error.nameRequired" { e.arg(MAX_NAME) } else { e });
    }
    Ok(n.to_string())
}

// ====================================================================== collections

fn collection_view(id: &str, library_id: &str, name: &str, count: usize, auto: bool) -> Value {
    json!({"id": id, "type": "collection", "libraryId": library_id, "title": name, "childCount": count, "auto": auto})
}

/// Collections of a library as row entries: movie series with at least two movies there, and
/// the hand-made ones (counting the entries the user may see).
pub(super) fn collections_of(app: &AppState, idx: &Index, user: &User, library_id: &str) -> Vec<Value> {
    let mut by_series: BTreeMap<i64, (Arc<Metadata>, usize)> = BTreeMap::new();
    for it in idx.top_level(library_id) {
        if let Some(md) = app.metadata.get(&it.id).filter(|m| m.collection_id > 0) {
            by_series.entry(md.collection_id).or_insert((md, 0)).1 += 1;
        }
    }
    let mut out: Vec<Value> = by_series
        .into_iter()
        .filter(|(_, (_, n))| *n >= 2)
        .map(|(cid, (md, n))| collection_view(&collection_id(cid, library_id), library_id, &collection_name(app, &md), n, true))
        .collect();
    for c in app.db.collections_of(library_id) {
        let visible = c.item_ids.iter().filter(|id| idx.item(id).is_some_and(|it| user.may_access(&it.library_id))).count();
        out.push(collection_view(&c.id, library_id, &c.name, visible, false));
    }
    out.sort_by_key(|c| c["title"].as_str().unwrap_or("").to_lowercase());
    out
}

pub async fn library_collections(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let l = visible_library_of(&app, u, &id)?;
    ok(json!(collections_of(&app, &app.catalog.snapshot(), u, &l.id)))
}

/// A collection with the entries the user may see, in order.
struct Resolved {
    id: String,
    library_id: String,
    name: String,
    summary: String,
    auto: bool,
    poster: Option<String>,
    backdrop: Option<String>,
    items: Vec<Arc<Item>>,
}

fn resolve(app: &AppState, idx: &Index, user: &User, cid: &str) -> Result<Resolved, ApiError> {
    let r = if let Some((tmdb, library_id)) = cid.strip_prefix('t').and_then(|r| r.split_once('-')) {
        // A movie series: its movies in the library, by year.
        let tmdb: i64 = tmdb.parse().map_err(|_| ApiError::not_found())?;
        let mut items: Vec<Arc<Item>> = Vec::new();
        let mut first: Option<Arc<Metadata>> = None;
        for it in idx.top_level(library_id) {
            if let Some(md) = app.metadata.get(&it.id).filter(|m| m.collection_id == tmdb) {
                items.push(it.clone());
                first.get_or_insert(md);
            }
        }
        let first = first.ok_or_else(ApiError::not_found)?;
        items.sort_by(|a, b| a.year.cmp(&b.year).then_with(|| a.sort_title.cmp(&b.sort_title)));
        let c = app.metadata.collection(tmdb).unwrap_or_default();
        Resolved {
            id: cid.to_string(),
            library_id: library_id.to_string(),
            name: collection_name(app, &first),
            summary: c.summary,
            auto: true,
            poster: c.poster,
            backdrop: c.backdrop,
            items,
        }
    } else {
        let c = app.db.collection(cid).ok_or_else(ApiError::not_found)?;
        let items = c.item_ids.iter().filter_map(|id| idx.item(id).cloned()).collect();
        Resolved { id: c.id, library_id: c.library_id, name: c.name, summary: c.summary, auto: false, poster: None, backdrop: None, items }
    };
    if !user.may_access(&r.library_id) {
        return Err(ApiError::not_found());
    }
    let items = r.items.into_iter().filter(|it| user.may_access(&it.library_id)).collect();
    Ok(Resolved { items, ..r })
}

pub async fn collection(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(cid): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let r = resolve(&app, &idx, u, &cid)?;
    let view = View::new(&app, &idx, u);
    let items = view.decorate_all(r.items.iter());
    let watched = items.iter().filter(|v| v["progress"]["watched"] == true).count();
    let mut m = collection_view(&r.id, &r.library_id, &r.name, r.items.len(), r.auto);
    m["summary"] = json!(r.summary);
    m["items"] = json!(items);
    m["watchedCount"] = json!(watched);
    ok(m)
}

pub async fn collection_image(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((cid, kind)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult {
    let u = ctx.user()?;
    let r = resolve(&app, &app.catalog.snapshot(), u, &cid)?;
    let backdrop = kind == "backdrop";
    let own = if backdrop { r.backdrop.clone() } else { r.poster.clone() };
    // The collection's own artwork, otherwise that of its first entry with some.
    let candidates =
        own.into_iter().chain(r.items.iter().filter_map(|it| if backdrop { it.backdrop_path.clone() } else { it.poster_path.clone() }));
    for p in candidates {
        let p = std::path::PathBuf::from(p);
        if p.is_file() {
            return crate::files::send(&p, IMAGE_CACHE_SECONDS, &headers).await;
        }
    }
    Err(ApiError::not_found())
}

/// A hand-made collection; movie series cannot be changed.
fn own_collection(app: &AppState, cid: &str) -> Result<OwnCollection, ApiError> {
    app.db.collection(cid).ok_or_else(|| ApiError::new(axum::http::StatusCode::NOT_FOUND, "error.collectionReadOnly"))
}

/// Adds a movie or show of the collection's library.
fn add_to_collection(app: &AppState, c: &mut OwnCollection, item_id: &str) -> Result<(), ApiError> {
    let idx = app.catalog.snapshot();
    let it = idx.item(item_id).ok_or_else(|| ApiError::bad_request("error.unknownItem"))?;
    if it.library_id != c.library_id || it.parent_id.is_some() {
        return Err(ApiError::bad_request("error.sameLibraryOnly"));
    }
    if !c.item_ids.contains(&it.id) {
        c.item_ids.push(it.id.clone());
    }
    app.db.save_collection(c);
    Ok(())
}

pub async fn create_collection(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    ctx.admin()?;
    let l = app.db.library(body.str("libraryId").unwrap_or("")).ok_or_else(|| ApiError::bad_request("error.libraryMissing"))?;
    let name = valid_name(body.str("name").unwrap_or(""), "error.nameRequired")?;
    let mut c = app.db.create_collection(&l.id, &name);
    if let Some(item) = body.str("itemId") {
        add_to_collection(&app, &mut c, item)?;
    }
    created(json!({"id": c.id, "name": c.name}))
}

pub async fn update_collection(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(cid): Path<String>,
    body: JsonBody,
) -> ApiResult {
    ctx.admin()?;
    let mut c = own_collection(&app, &cid)?;
    if body.has("name") {
        c.name = valid_name(body.str("name").unwrap_or(""), "error.invalidName")?;
    }
    if body.has("summary") {
        c.summary = body.string("summary").trim().chars().take(4000).collect();
    }
    app.db.save_collection(&c);
    ok(json!({"id": c.id, "name": c.name}))
}

pub async fn delete_collection(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(cid): Path<String>) -> ApiResult {
    ctx.admin()?;
    let c = own_collection(&app, &cid)?;
    app.db.delete_collection(&c.id);
    no_content()
}

pub async fn add_collection_item(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(cid): Path<String>,
    body: JsonBody,
) -> ApiResult {
    ctx.admin()?;
    let mut c = own_collection(&app, &cid)?;
    add_to_collection(&app, &mut c, body.str("itemId").unwrap_or(""))?;
    no_content()
}

pub async fn remove_collection_item(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((cid, item)): Path<(String, String)>,
) -> ApiResult {
    ctx.admin()?;
    let mut c = own_collection(&app, &cid)?;
    c.item_ids.retain(|i| *i != item);
    app.db.save_collection(&c);
    no_content()
}

// ====================================================================== playlists

fn playlist_view(app: &AppState, p: &crate::db::Playlist, u: &User, with_items: bool) -> Value {
    let idx = app.catalog.snapshot();
    // Entries of libraries that are no longer shared are left out.
    let visible: Vec<Arc<Item>> =
        p.item_ids.iter().filter_map(|id| idx.item(id)).filter(|it| u.may_access(&it.library_id)).cloned().collect();
    let mut m = json!({
        "id": p.id,
        "type": "playlist",
        "title": p.name,
        "childCount": p.item_ids.len(),
        "durationMs": visible.iter().map(|it| it.duration_ms()).sum::<i64>(),
        "firstItemId": visible.first().map(|it| it.id.clone()),
        "updatedAt": p.updated_at,
    });
    if with_items {
        m["items"] = json!(View::new(app, &idx, u).decorate_all(visible.iter()));
    }
    m
}

/// A show or season adds all its episodes in order.
fn playable_ids(idx: &Index, it: &Item) -> Vec<String> {
    match it.kind {
        Kind::Movie | Kind::Episode => vec![it.id.clone()],
        Kind::Show => idx.episodes_of(it).iter().map(|e| e.id.clone()).collect(),
        Kind::Season => idx.children(&it.id).iter().map(|e| e.id.clone()).collect(),
        Kind::Extra => Vec::new(),
    }
}

fn own_playlist(app: &AppState, u: &User, pid: &str) -> Result<crate::db::Playlist, ApiError> {
    app.db.playlist(&u.id, pid).ok_or_else(ApiError::not_found)
}

pub async fn playlists(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let u = ctx.user()?;
    ok(json!(app.db.playlists_of(&u.id).iter().map(|p| playlist_view(&app, p, u, false)).collect::<Vec<_>>()))
}

pub async fn playlist(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(pid): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    ok(playlist_view(&app, &own_playlist(&app, u, &pid)?, u, true))
}

pub async fn create_playlist(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let u = ctx.user()?;
    let name = valid_name(body.str("name").unwrap_or(""), "error.nameRequired")?;
    if app.db.playlists_of(&u.id).len() >= MAX_PLAYLISTS {
        return Err(ApiError::bad_request("error.tooManyPlaylists"));
    }
    let ids = match body.str("itemId") {
        Some(id) => {
            let idx = app.catalog.snapshot();
            let it = visible_item(&idx, u, id).map_err(|_| ApiError::bad_request("error.unknownItem"))?;
            playable_ids(&idx, &it)
        }
        None => Vec::new(),
    };
    let p = app.db.create_playlist(&u.id, &name, ids);
    created(playlist_view(&app, &p, u, false))
}

pub async fn update_playlist(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(pid): Path<String>,
    body: JsonBody,
) -> ApiResult {
    let u = ctx.user()?;
    let mut p = own_playlist(&app, u, &pid)?;
    if body.has("name") {
        p.name = valid_name(body.str("name").unwrap_or(""), "error.invalidName")?;
    }
    if let Some(order) = body.get("itemIds").and_then(Value::as_array) {
        // Reordering: accepted only if it is the same entries in another order.
        let next: Vec<String> = order.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
        let (mut a, mut b) = (next.clone(), p.item_ids.clone());
        a.sort();
        b.sort();
        if a == b {
            p.item_ids = next;
        }
    }
    app.db.save_playlist(&mut p);
    ok(playlist_view(&app, &p, u, false))
}

pub async fn delete_playlist(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(pid): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let p = own_playlist(&app, u, &pid)?;
    app.db.delete_playlist(&p.id);
    no_content()
}

pub async fn add_playlist_item(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(pid): Path<String>,
    body: JsonBody,
) -> ApiResult {
    let u = ctx.user()?;
    let mut p = own_playlist(&app, u, &pid)?;
    let idx = app.catalog.snapshot();
    let it = visible_item(&idx, u, body.str("itemId").unwrap_or("")).map_err(|_| ApiError::bad_request("error.unknownItem"))?;
    let ids = playable_ids(&idx, &it);
    if p.item_ids.len() + ids.len() > MAX_PLAYLIST_ITEMS {
        return Err(ApiError::bad_request("error.playlistTooLong"));
    }
    let added = ids.len();
    p.item_ids.extend(ids);
    app.db.save_playlist(&mut p);
    ok(json!({"added": added}))
}

pub async fn remove_playlist_item(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((pid, index)): Path<(String, String)>,
) -> ApiResult {
    let u = ctx.user()?;
    let mut p = own_playlist(&app, u, &pid)?;
    let i: usize = index.parse().map_err(|_| ApiError::not_found())?;
    if i >= p.item_ids.len() {
        return Err(ApiError::not_found());
    }
    p.item_ids.remove(i);
    app.db.save_playlist(&mut p);
    no_content()
}

// ====================================================================== watchlist

/// Watchlist entries the user may see, last added first.
pub(super) fn watchlist_items(app: &AppState, idx: &Index, u: &User, limit: usize) -> Vec<Value> {
    let view = View::new(app, idx, u);
    app.db
        .watchlist(&u.id)
        .iter()
        .filter_map(|id| idx.item(id))
        .filter(|it| u.may_access(&it.library_id))
        .take(limit)
        .map(|it| view.decorate(it, false))
        .collect()
}

pub async fn watchlist(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let u = ctx.user()?;
    ok(json!(watchlist_items(&app, &app.catalog.snapshot(), u, usize::MAX)))
}

pub async fn watchlist_add(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    set_watchlist(&app, &ctx, &id, true)
}

pub async fn watchlist_remove(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    set_watchlist(&app, &ctx, &id, false)
}

fn set_watchlist(app: &AppState, ctx: &Ctx, id: &str, on: bool) -> ApiResult {
    let u = ctx.user()?;
    let it = visible_item(&app.catalog.snapshot(), u, id)?;
    if !matches!(it.kind, Kind::Movie | Kind::Show) {
        return Err(ApiError::bad_request("error.moviesAndShowsOnly"));
    }
    app.db.set_watchlist(&u.id, &it.id, on);
    ok(json!({"onWatchlist": on}))
}
