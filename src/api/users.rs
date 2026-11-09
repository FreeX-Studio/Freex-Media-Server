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

//! User management for admins: accounts, admin rights, passwords, two-factor reset and the
//! libraries shared with each user.

use super::account::{valid_password, valid_username};
use crate::AppState;
use crate::auth::{self, now_ms};
use crate::db::User;
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, created, no_content, ok};
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::sync::Arc;

pub async fn list(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    ctx.admin()?;
    ok(json!(app.db.users().iter().map(User::to_public).collect::<Vec<_>>()))
}

pub async fn create(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let admin = ctx.admin()?;
    let username = valid_username(body.str("username").unwrap_or(""))?;
    let password = valid_password(&app, body.str("password").unwrap_or(""), &username)?;
    let mut u = app
        .db
        .add_user(&username, &auth::hash_password(&password), body.bool("admin").unwrap_or(false))
        .ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "error.usernameTaken").arg(username.clone()))?;
    if let Some(v) = body.get("libraries") {
        apply_libraries(&app, &mut u, v)?;
        app.db.update_user(&u);
    }
    tracing::info!("User '{}' created by '{}'", username, admin.username);
    created(u.to_public())
}

pub async fn update(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>, body: JsonBody) -> ApiResult {
    let me = ctx.admin()?;
    let mut u = app.db.user(&id).ok_or_else(ApiError::not_found)?;
    let mut end_sessions = false;
    if let Some(admin) = body.bool("admin") {
        if !admin && u.admin && app.db.admin_count() <= 1 {
            return Err(ApiError::bad_request("error.lastAdmin"));
        }
        u.admin = admin;
    }
    if let Some(v) = body.get("libraries") {
        apply_libraries(&app, &mut u, v)?;
        tracing::info!(
            "Libraries of '{}' set by '{}': {}",
            u.username,
            me.username,
            if u.all_libraries { "all".to_string() } else { u.library_ids.join(", ") }
        );
    }
    let password = body.string("password");
    if !password.is_empty() {
        u.password_hash = auth::hash_password(&valid_password(&app, &password, &u.username)?);
        u.password_changed_at = now_ms();
        end_sessions = true;
    }
    if body.bool("resetTwoFactor") == Some(true) {
        u.totp_enabled = false;
        u.totp_secret = None;
        u.recovery_codes.clear();
        end_sessions = true;
    }
    app.db.update_user(&u);
    if end_sessions {
        // An admin changing their own account stays signed in on this device.
        let keep = if u.id == me.id { ctx.token.as_deref() } else { None };
        app.db.remove_sessions_of(&u.id, keep);
    }
    tracing::info!("User '{}' changed by '{}'", u.username, me.username);
    ok(u.to_public())
}

pub async fn delete(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, Path(id): Path<String>) -> ApiResult {
    let me = ctx.admin()?;
    if id == me.id {
        return Err(ApiError::bad_request("error.deleteSelf"));
    }
    let u = app.db.user(&id).ok_or_else(ApiError::not_found)?;
    app.db.remove_user(&id);
    super::profile::remove_avatar(&app, &id);
    tracing::info!("User '{}' deleted by '{}'", u.username, me.username);
    no_content()
}

/// `"all"` or a list of library ids.
fn apply_libraries(app: &AppState, u: &mut User, v: &Value) -> Result<(), ApiError> {
    if v.as_str() == Some("all") {
        u.all_libraries = true;
        u.library_ids.clear();
        return Ok(());
    }
    let list = v.as_array().ok_or_else(|| ApiError::bad_request("error.librariesValue"))?;
    let mut ids: Vec<String> = Vec::new();
    for e in list {
        let id = e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string());
        if app.db.library(&id).is_none() {
            return Err(ApiError::bad_request("error.unknownLibrary").arg(id));
        }
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    u.all_libraries = false;
    u.library_ids = ids;
    Ok(())
}
