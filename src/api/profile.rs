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

//! Profile pictures and the activity view of admins (running playbacks and background tasks).

use super::media::View;
use crate::AppState;
use crate::auth::now_ms;
use crate::http::{ApiError, ApiResult, Ctx, ok};
use crate::i18n::Text;
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use image::imageops::FilterType;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

pub const MAX_UPLOAD: usize = 10 * 1024 * 1024;
const AVATAR_SIZE: u32 = 256;
const MIN_SIDE: u32 = 32;
/// Checked before decoding, so a small file claiming huge dimensions cannot exhaust memory.
const MAX_PIXELS: u64 = 50_000_000;
/// The URL carries `?v=<version>`, so the picture can be cached for long.
const AVATAR_CACHE_SECONDS: u32 = 30 * 24 * 3600;

fn avatar_file(app: &AppState, user_id: &str) -> Result<PathBuf, ApiError> {
    if user_id.is_empty() || user_id.len() > 64 || !user_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::not_found());
    }
    Ok(app.config.data_dir().join("avatars").join(format!("{user_id}.png")))
}

/// Deletes the picture of an account that is being deleted.
pub fn remove_avatar(app: &AppState, user_id: &str) {
    if let Ok(p) = avatar_file(app, user_id) {
        let _ = std::fs::remove_file(p);
    }
}

/// Decodes JPEG, PNG, GIF, BMP or WebP, crops the centre square and returns a 256 × 256 PNG.
pub fn to_avatar(data: &[u8]) -> Result<Vec<u8>, ApiError> {
    let reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| ApiError::bad_request("error.imageUnreadable"))?;
    if reader.format().is_none() {
        return Err(ApiError::bad_request("error.imageType"));
    }
    let (w, h) = reader.into_dimensions().map_err(|_| ApiError::bad_request("error.imageUnreadable"))?;
    if w < MIN_SIDE || h < MIN_SIDE {
        return Err(ApiError::bad_request("error.imageTooSmall").arg(i64::from(MIN_SIDE)));
    }
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(ApiError::bad_request("error.imageTooLarge"));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| ApiError::bad_request("error.imageUnreadable"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(w);
    limits.max_image_height = Some(h);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| ApiError::bad_request("error.imageUnreadable"))?;
    let side = w.min(h);
    let square = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let out = square.resize_exact(AVATAR_SIZE, AVATAR_SIZE, FilterType::CatmullRom).to_rgba8();
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| ApiError::bad_request("error.imageUnreadable"))?;
    Ok(png)
}

pub async fn upload_avatar(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: Bytes) -> ApiResult {
    let mut u = ctx.user()?.clone();
    let png = tokio::task::spawn_blocking(move || to_avatar(&body)).await.map_err(|_| ApiError::bad_request("error.imageUnreadable"))??;
    let file = avatar_file(&app, &u.id)?;
    if let Some(dir) = file.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    let tmp = file.with_extension("tmp");
    let written = tokio::fs::write(&tmp, &png).await.is_ok() && tokio::fs::rename(&tmp, &file).await.is_ok();
    if !written {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, "error.imageUnreadable"));
    }
    u.avatar_version = now_ms();
    app.db.update_user(&u);
    ok(u.to_public())
}

pub async fn delete_avatar(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let mut u = ctx.user()?.clone();
    remove_avatar(&app, &u.id);
    u.avatar_version = 0;
    app.db.update_user(&u);
    ok(u.to_public())
}

pub async fn avatar(
    State(app): State<Arc<AppState>>,
    Extension(ctx): Extension<Ctx>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult {
    ctx.user()?;
    let u = app.db.user(&id).filter(|u| u.avatar_version > 0).ok_or_else(ApiError::not_found)?;
    crate::files::send(&avatar_file(&app, &u.id)?, AVATAR_CACHE_SECONDS, &headers).await
}

/// What the server is doing right now: playbacks with user, device and mode, and scans.
pub async fn activity(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let viewer = ctx.admin()?;
    let idx = app.catalog.snapshot();
    let view = View::new(&app, &idx, viewer);
    let sessions: Vec<_> = app
        .activity
        .list()
        .into_iter()
        .filter_map(|p| {
            let it = idx.item(&p.item_id)?;
            let user = app.db.user(&p.user_id).map(|u| json!({"id": u.id, "username": u.username, "avatar": u.avatar_version}));
            let transcoding = p.session.as_deref().is_some_and(|s| app.transcoder.as_ref().is_some_and(|t| t.session(s).is_some()));
            Some(json!({
                "client": p.client,
                "user": user,
                "item": view.decorate(it, false),
                "mode": p.mode,
                "reason": ctx.t(&p.reason),
                "quality": p.quality,
                "state": p.state,
                "offsetMs": p.offset_ms,
                "durationMs": it.duration_ms(),
                "device": p.device,
                "userAgent": p.user_agent,
                "ip": p.ip,
                "startedAt": p.started_at,
                "transcoding": transcoding,
            }))
        })
        .collect();
    let tasks: Vec<_> = app
        .db
        .libraries()
        .iter()
        .filter(|l| app.scanner.is_scanning(&l.id))
        .map(|l| json!({"type": "scan", "title": ctx.t(&Text::new("activity.scanning").arg(l.name.clone()))}))
        .collect();
    ok(json!({"sessions": sessions, "tasks": tasks, "transcodes": app.transcoder.as_ref().map_or(0, |t| t.active_sessions())}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 80, 40]));
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn makes_square_avatars() {
        let out = to_avatar(&png(400, 300)).unwrap();
        let img = image::load_from_memory(&out).unwrap();
        assert_eq!((img.width(), img.height()), (AVATAR_SIZE, AVATAR_SIZE));
        assert_eq!(to_avatar(&png(20, 20)).unwrap_err().text.key, "error.imageTooSmall");
        assert_eq!(to_avatar(b"not an image").unwrap_err().text.key, "error.imageType");
    }
}
