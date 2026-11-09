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

//! Server status and the library definitions.

use super::media::library_view;
use crate::AppState;
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, created, no_content, ok};
use crate::media::scanner::Scanner;
use axum::Extension;
use axum::extract::{Path, State};
use serde_json::{Value, json};
use std::sync::Arc;

pub const SERVER_NAME: &str = "FreeX Media Server";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn status(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let mut m = json!({
        "name": app.config.server.name,
        "setupRequired": !app.db.has_users(),
        "loggedIn": ctx.user.is_some(),
    });
    // Versions only for signed-in users: an anonymous visitor does not need to know what runs here.
    if let Some(u) = &ctx.user {
        m["version"] = json!(VERSION);
        m["server"] = json!({"name": SERVER_NAME, "version": VERSION});
        m["web"] = app.web.version.clone();
        if u.admin {
            m["tls"] = json!(ctx.secure);
            m["ffprobe"] = json!(app.tools.ffprobe_ok);
            m["ffmpeg"] = json!(app.tools.ffmpeg_ok);
            m["transcoder"] = json!(app.transcoder.is_some());
            m["activeTranscodes"] = json!(app.transcoder.as_ref().map_or(0, |t| t.active_sessions()));
            m["playing"] = json!(app.activity.count());
        }
    }
    ok(m)
}

pub async fn libraries(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let u = ctx.user()?;
    let list: Vec<Value> = app.db.libraries().iter().filter(|l| u.may_access(&l.id)).map(|l| library_view(&app, l, u.admin)).collect();
    ok(json!(list))
}

pub async fn create_library(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    ctx.admin()?;
    let name = body.string("name").trim().to_string();
    if name.is_empty() || name.chars().count() > 64 {
        return Err(ApiError::bad_request("error.nameRequired").arg(64usize));
    }
    let kind = library_type(body.str("type").unwrap_or("")).ok_or_else(|| ApiError::bad_request("error.libraryType"))?;
    let paths = valid_paths(body.get("paths"))?;
    let l = app.db.add_library(&name, kind, &paths);
    tracing::info!("Library '{}' ({}) created: {}", l.name, l.kind, l.paths.join(", "));
    Scanner::scan(&app, &l.id);
    created(library_view(&app, &l, true))
}

pub async fn update_library(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    body: JsonBody,
) -> ApiResult {
    ctx.admin()?;
    let mut l = app.db.library(&id).ok_or_else(ApiError::not_found)?;
    if body.has("name") {
        let name = body.string("name").trim().to_string();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(ApiError::bad_request("error.invalidName"));
        }
        l.name = name;
    }
    let rescan = body.has("paths");
    if rescan {
        l.paths = valid_paths(body.get("paths"))?;
    }
    app.db.save_library(&l);
    if rescan {
        Scanner::scan(&app, &l.id);
    }
    ok(library_view(&app, &l, true))
}

pub async fn delete_library(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    ctx.admin()?;
    if !app.db.remove_library(&id) {
        return Err(ApiError::not_found());
    }
    app.catalog.remove_library(&id);
    no_content()
}

fn library_type(s: &str) -> Option<&'static str> {
    match s.trim().to_lowercase().as_str() {
        "movies" | "movie" | "filme" => Some("movies"),
        "shows" | "show" | "series" | "serien" | "tv" => Some("shows"),
        _ => None,
    }
}

/// Existing folders, made absolute, without duplicates; at least one.
fn valid_paths(raw: Option<&Value>) -> Result<Vec<String>, ApiError> {
    let mut paths: Vec<String> = Vec::new();
    for v in raw.and_then(Value::as_array).into_iter().flatten() {
        let s = v.as_str().unwrap_or("").trim();
        if s.is_empty() {
            continue;
        }
        let p = std::path::absolute(s).unwrap_or_else(|_| s.into());
        if !p.is_dir() {
            return Err(ApiError::bad_request("error.folderNotFound").arg(s));
        }
        let p = p.to_string_lossy().into_owned();
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    if paths.is_empty() {
        return Err(ApiError::bad_request("error.folderRequired"));
    }
    Ok(paths)
}
