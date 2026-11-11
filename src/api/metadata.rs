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
        "contentRating": if it.content_rating.is_empty() { md.content_rating.clone() } else { it.content_rating.clone() },
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
        "previews": app.previews.is_some(),
        "previewsBusy": app.previews.as_ref().is_some_and(|p| p.busy()),
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
        let Some(it) = idx.item(&id).filter(|it| super::media::may_see(&idx, u, it)) else { continue };
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

// ====================================================================== editing by hand

use crate::metadata::edits::{Edit, FIELDS};

/// An entry that can be edited: not an extra and not the stand-in of a version.
fn editable(app: &AppState, id: &str) -> Result<Arc<Item>, ApiError> {
    let it = app.catalog.snapshot().item(id).cloned().ok_or_else(ApiError::not_found)?;
    if it.kind == Kind::Extra || it.version_of.is_some() {
        return Err(ApiError::not_found());
    }
    Ok(it)
}

fn edit_view(it: &Item, e: &Edit) -> Value {
    let edited: Vec<&str> = FIELDS
        .iter()
        .copied()
        .filter(|f| match *f {
            "title" => e.title.is_some(),
            "sortTitle" => e.sort_title.is_some(),
            "originalTitle" => e.original_title.is_some(),
            "year" => e.year.is_some(),
            "summary" => e.summary.is_some(),
            "tagline" => e.tagline.is_some(),
            "genres" => e.genres.is_some(),
            "rating" => e.rating.is_some(),
            _ => e.content_rating.is_some(),
        })
        .collect();
    json!({
        "values": {
            "title": it.title, "sortTitle": e.sort_title.clone().unwrap_or_default(), "originalTitle": it.original_title,
            "year": it.year, "summary": it.summary, "tagline": it.tagline, "genres": it.genres,
            "rating": it.rating, "contentRating": it.content_rating,
        },
        "edited": edited,
        "poster": e.poster.is_some(),
        "backdrop": e.backdrop.is_some(),
    })
}

/// The fields of an entry as they are now, and which of them were edited by hand.
pub async fn get_edit(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    ctx.admin()?;
    let it = editable(&app, &id)?;
    ok(edit_view(&it, &app.metadata.edit(&it.id).unwrap_or_default()))
}

/// `{values: {title: "…", year: 1999, …}, reset: ["summary", …]}`: sets fields and resets others
/// to what the files and the online metadata say.
pub async fn put_edit(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    body: JsonBody,
) -> ApiResult {
    ctx.admin()?;
    let it = editable(&app, &id)?;
    let mut e = app.metadata.edit(&it.id).unwrap_or_default();
    let invalid = |f: &str| ApiError::bad_request("error.editField").arg(f.to_string());
    let text = |v: &Value, f: &str, max: usize| -> Result<String, ApiError> {
        let s = v.as_str().ok_or_else(|| invalid(f))?.trim().to_string();
        if s.chars().count() > max || s.chars().any(|c| c.is_control() && c != '\n') { Err(invalid(f)) } else { Ok(s) }
    };
    if let Some(values) = body.get("values").and_then(Value::as_object) {
        for (k, v) in values {
            match k.as_str() {
                "title" => {
                    let t = text(v, k, 300)?;
                    if t.is_empty() {
                        return Err(invalid(k));
                    }
                    e.title = Some(t);
                }
                "sortTitle" => e.sort_title = Some(text(v, k, 300)?).filter(|s| !s.is_empty()),
                "originalTitle" => e.original_title = Some(text(v, k, 300)?),
                "summary" => e.summary = Some(text(v, k, 10_000)?),
                "tagline" => e.tagline = Some(text(v, k, 500)?),
                "genres" => {
                    let g = text(v, k, 500)?;
                    let list: Vec<&str> = g.split(',').map(str::trim).filter(|x| !x.is_empty()).collect();
                    e.genres = Some(list.join(", "));
                }
                "contentRating" => e.content_rating = Some(text(v, k, 20)?),
                "year" => e.year = Some(v.as_i64().filter(|y| (0..=3000).contains(y)).ok_or_else(|| invalid(k))? as i32),
                "rating" => {
                    let r = v.as_f64().filter(|r| (0.0..=10.0).contains(r)).ok_or_else(|| invalid(k))?;
                    e.rating = Some((r * 10.0).round() / 10.0);
                }
                _ => return Err(invalid(k)),
            }
        }
    }
    let mut reset = false;
    if let Some(list) = body.get("reset").and_then(Value::as_array) {
        for f in list.iter().filter_map(Value::as_str) {
            reset = true;
            match f {
                "title" => e.title = None,
                "sortTitle" => e.sort_title = None,
                "originalTitle" => e.original_title = None,
                "year" => e.year = None,
                "summary" => e.summary = None,
                "tagline" => e.tagline = None,
                "genres" => e.genres = None,
                "rating" => e.rating = None,
                "contentRating" => e.content_rating = None,
                _ => return Err(invalid(f)),
            }
        }
    }
    app.metadata.set_edit(&app.db, &it.id, e.clone());
    apply_edits(&app, &it.library_id, reset);
    let it = editable(&app, &id)?;
    ok(edit_view(&it, &e))
}

/// Puts edits into the index at once; a reset field needs the values from the files again,
/// which a rescan of the library brings.
fn apply_edits(app: &Arc<AppState>, library_id: &str, rescan: bool) {
    app.metadata.reapply(app, library_id);
    if rescan {
        crate::media::scanner::Scanner::scan(app, library_id);
    }
}

/// Uploads a poster or background (raw image body) for an entry; it wins over all other artwork.
pub async fn put_artwork(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((id, kind)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> ApiResult {
    ctx.admin()?;
    let it = editable(&app, &id)?;
    if kind != "poster" && kind != "backdrop" {
        return Err(ApiError::not_found());
    }
    let jpeg = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ApiError> {
        let img = super::profile::decode_image(&body)?;
        // Large enough for any screen, small enough for the cache.
        let img =
            if img.width() > 2560 || img.height() > 2560 { img.resize(2560, 2560, image::imageops::FilterType::CatmullRom) } else { img };
        let mut out = Vec::new();
        img.to_rgb8()
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg)
            .map_err(|_| ApiError::bad_request("error.imageUnreadable"))?;
        Ok(out)
    })
    .await
    .map_err(|_| ApiError::bad_request("error.imageUnreadable"))??;
    let dir = app.config.data_dir().join("artwork");
    tokio::fs::create_dir_all(&dir).await.map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "error.imageUnreadable"))?;
    // A new name for every upload, so browsers do not keep the old picture.
    let file = dir.join(format!("{}-{kind}-{:x}.jpg", it.id, crate::auth::now_ms()));
    tokio::fs::write(&file, &jpeg).await.map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "error.imageUnreadable"))?;
    let mut e = app.metadata.edit(&it.id).unwrap_or_default();
    let slot = if kind == "poster" { &mut e.poster } else { &mut e.backdrop };
    if let Some(old) = slot.replace(file.to_string_lossy().into_owned()) {
        let _ = tokio::fs::remove_file(old).await;
    }
    app.metadata.set_edit(&app.db, &it.id, e.clone());
    apply_edits(&app, &it.library_id, false);
    let fresh = editable(&app, &id)?;
    ok(edit_view(&fresh, &e))
}

/// Removes an uploaded poster or background; the artwork from the folder or TMDB comes back.
pub async fn delete_artwork(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path((id, kind)): Path<(String, String)>,
) -> ApiResult {
    ctx.admin()?;
    let it = editable(&app, &id)?;
    let mut e = app.metadata.edit(&it.id).unwrap_or_default();
    let old = match kind.as_str() {
        "poster" => e.poster.take(),
        "backdrop" => e.backdrop.take(),
        _ => return Err(ApiError::not_found()),
    };
    if let Some(old) = old {
        let _ = tokio::fs::remove_file(old).await;
    }
    app.metadata.set_edit(&app.db, &it.id, e.clone());
    apply_edits(&app, &it.library_id, true);
    ok(edit_view(&it, &e))
}
