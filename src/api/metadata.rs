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

//! Online metadata in the API: the extra fields of item pages (cast, crew, trailer, age rating,
//! movie series), "fix match", people and the movie series of a library.

use super::media::{IMAGE_CACHE_SECONDS, View, visible_item, visible_library_of};
use crate::AppState;
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, ok};
use crate::media::catalog::Index;
use crate::media::{Item, Kind};
use crate::metadata::tmdb::MetaError;
use crate::metadata::{Metadata, Service};
use axum::Extension;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

fn tmdb_error(e: MetaError) -> ApiError {
    let status = if e.0.key == "error.tmdbNoKey" { StatusCode::BAD_REQUEST } else { StatusCode::BAD_GATEWAY };
    ApiError::from_text(status, e.0)
}

fn cast_of(md: &Metadata) -> Vec<Value> {
    md.cast.iter().map(|p| json!({"id": p.id, "name": p.name, "role": p.role, "hasImage": p.image.is_some()})).collect()
}

pub(super) fn collection_id(tmdb: i64, library_id: &str) -> String {
    format!("t{tmdb}-{library_id}")
}

pub(super) fn collection_name(app: &AppState, md: &Metadata) -> String {
    app.metadata.collection(md.collection_id).map(|c| c.name).filter(|n| !n.is_empty()).unwrap_or_else(|| md.collection_name.clone())
}

/// Adds the online metadata to the full view of an item.
pub(super) fn extend(app: &AppState, idx: &Index, it: &Item, m: &mut Value) {
    let show_md = || idx.show_of(it).and_then(|s| app.metadata.get(&s.id));
    // Movie series and own collections the entry belongs to.
    let mut colls = Vec::new();
    if let Some(md) = app.metadata.get(&it.id).filter(|m| m.collection_id > 0) {
        colls.push(json!({"id": collection_id(md.collection_id, &it.library_id), "name": collection_name(app, &md)}));
    }
    colls.extend(app.db.collections_with(&it.id).into_iter().map(|(id, name)| json!({"id": id, "name": name})));
    if matches!(it.kind, Kind::Movie | Kind::Show) {
        m["collections"] = json!(colls);
    }
    let Some(md) = app.metadata.get(&it.id) else {
        // An episode without its own metadata shows the cast of its show.
        if it.kind == Kind::Episode
            && let Some(show) = show_md()
        {
            m["metadata"] = json!({"cast": cast_of(&show)});
        }
        return;
    };
    let mut cast = cast_of(&md);
    if cast.is_empty() && it.kind == Kind::Episode {
        cast = show_md().map(|s| cast_of(&s)).unwrap_or_default();
    }
    m["metadata"] = json!({
        "tmdbId": md.tmdb_id,
        "matched": md.matched(),
        "locked": md.locked,
        "source": md.source,
        "contentRating": md.content_rating,
        "studio": md.studio,
        "airDate": md.air_date,
        "trailer": md.trailer_key,
        "directors": md.directors,
        "writers": md.writers,
        "cast": cast,
    });
}

// ====================================================================== fix match

pub async fn search(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult {
    let u = ctx.admin()?;
    let it = visible_item(&app.catalog.snapshot(), u, &id)?;
    let top = top_kind(&it);
    let query = q.get("q").cloned().unwrap_or_else(|| it.title.clone()).trim().to_string();
    if query.is_empty() {
        return Err(ApiError::bad_request("error.queryRequired"));
    }
    let year = q.get("year").and_then(|y| y.parse().ok()).unwrap_or(0);
    ok(json!(app.metadata.search_for_match(top, &query, year).await.map_err(tmdb_error)?))
}

/// Movie for movies, show for shows, seasons and episodes.
fn top_kind(it: &Item) -> Kind {
    if it.kind == Kind::Movie { Kind::Movie } else { Kind::Show }
}

pub async fn search_image(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> ApiResult {
    ctx.admin()?;
    let path = q.get("path").ok_or_else(ApiError::not_found)?;
    let file = app.metadata.search_image(path).await.ok_or_else(ApiError::not_found)?;
    crate::files::send(&file, IMAGE_CACHE_SECONDS, &headers).await
}

fn full_view(app: &AppState, ctx: &Ctx, id: &str) -> ApiResult {
    let u = ctx.user()?;
    let idx = app.catalog.snapshot();
    let it = visible_item(&idx, u, id)?;
    ok(View::new(app, &idx, u).decorate(&it, true))
}

pub async fn pin(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>, body: JsonBody) -> ApiResult {
    let u = ctx.admin()?;
    let it = visible_item(&app.catalog.snapshot(), u, &id)?;
    let tmdb_id =
        body.get("tmdbId").and_then(Value::as_i64).filter(|n| *n > 0).ok_or_else(|| ApiError::bad_request("error.tmdbIdMissing"))?;
    app.metadata.pin(&app, &it, tmdb_id).await.map_err(tmdb_error)?;
    tracing::info!("'{}' pinned to TMDB {tmdb_id} by '{}'", it.title, u.username);
    full_view(&app, &ctx, &id)
}

pub async fn refresh(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let u = ctx.admin()?;
    let it = visible_item(&app.catalog.snapshot(), u, &id)?;
    app.metadata.refresh_one(&app, &it).await.map_err(tmdb_error)?;
    full_view(&app, &ctx, &id)
}

/// Removes the online metadata; a rescan restores the data of the files.
pub async fn unpin(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let u = ctx.admin()?;
    let it = visible_item(&app.catalog.snapshot(), u, &id)?;
    app.metadata.unpin(&app, &it);
    crate::media::scanner::Scanner::scan(&app, &it.library_id);
    Ok((StatusCode::ACCEPTED, axum::Json(json!({"ok": true, "hint": ctx.t(&crate::i18n::Text::new("metadata.resetOnNextScan"))})))
        .into_response())
}

pub async fn refresh_library(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let u = ctx.admin()?;
    let l = visible_library_of(&app, u, &id)?;
    Service::refresh_library(&app, &l.id, true);
    Ok((StatusCode::ACCEPTED, axum::Json(json!({"queued": app.metadata.enabled()}))).into_response())
}

pub async fn status(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    ctx.admin()?;
    ok(json!({
        "tmdb": app.metadata.enabled(),
        "metadataBusy": app.metadata.busy(),
        "metadataError": app.metadata.last_error().map(|t| ctx.t(&t)),
        "markers": app.markers.is_some(),
        "markersBusy": app.markers.as_ref().is_some_and(|m| m.busy()),
    }))
}

// ====================================================================== people

pub async fn person(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(pid): Path<String>) -> ApiResult {
    let u = ctx.user()?;
    let pid: i64 = pid.parse().map_err(|_| ApiError::not_found())?;
    let p = app.metadata.person(pid).ok_or_else(ApiError::not_found)?;
    let idx = app.catalog.snapshot();
    let mut found: Vec<Arc<Item>> = Vec::new();
    for id in app.metadata.items_with_person(pid) {
        let Some(it) = idx.item(&id).filter(|it| u.may_access(&it.library_id)) else { continue };
        // Guest roles in episodes link to the show.
        let top = idx.show_of(it).unwrap_or(it).clone();
        if !found.iter().any(|f| f.id == top.id) {
            found.push(top);
        }
    }
    found.sort_by_key(|it| std::cmp::Reverse(it.year));
    let view = View::new(&app, &idx, u);
    ok(json!({"id": p.id, "name": p.name, "hasImage": p.image.is_some(), "items": view.decorate_all(found.iter())}))
}

pub async fn person_image(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(pid): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    ctx.user()?;
    let pid: i64 = pid.parse().map_err(|_| ApiError::not_found())?;
    let image = app.metadata.person(pid).and_then(|p| p.image).ok_or_else(ApiError::not_found)?;
    crate::files::send(std::path::Path::new(&image), IMAGE_CACHE_SECONDS, &headers).await
}
