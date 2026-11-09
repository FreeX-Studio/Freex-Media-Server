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

//! The server settings page: reading and changing `config.toml`, and restarting the server.

use crate::AppState;
use crate::auth;
use crate::config::Config;
use crate::http::{ApiError, ApiResult, Ctx, JsonBody, ok};
use crate::i18n::Text;
use crate::settings::{self, Field, Kind};
use axum::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::Path;
use std::sync::Arc;

/// The configuration as the file holds it now (it may differ from the running one).
fn saved(app: &AppState) -> Result<Config, ApiError> {
    Config::load(&app.config_path).map_err(|e| {
        tracing::warn!("{e}");
        ApiError::from_text(StatusCode::INTERNAL_SERVER_ERROR, Text::new("error.configInvalid").arg(e))
    })
}

fn writable(app: &AppState) -> bool {
    let p = &app.config_path;
    let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let file_ok = std::fs::OpenOptions::new().append(true).open(p).is_ok();
    let dir_ok = std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly());
    file_ok && dir_ok
}

fn describe(app: &AppState) -> Result<Value, ApiError> {
    let file = saved(app)?;
    let running = &app.config;
    let mut values = serde_json::Map::new();
    let mut secrets = serde_json::Map::new();
    let mut pending = false;
    for f in settings::FIELDS {
        let v = f.current(&file);
        if v != f.current(running) {
            pending = true;
        }
        match f.kind {
            Kind::Secret => {
                secrets.insert(f.key.to_string(), json!(!v.is_empty()));
            }
            _ => {
                values.insert(f.key.to_string(), f.typed(&v));
            }
        }
    }
    Ok(json!({
        "file": app.config_path.to_string_lossy(),
        "writable": writable(app),
        "pendingRestart": pending,
        "fields": settings::FIELDS.iter().map(Field::to_public).collect::<Vec<_>>(),
        "values": values,
        "secrets": secrets,
        "paths": {
            "data.directory": std::path::absolute(running.data_dir()).unwrap_or_default().to_string_lossy(),
            "data.cacheDirectory": std::path::absolute(running.cache_dir()).unwrap_or_default().to_string_lossy(),
            "transcoder.directory": std::path::absolute(running.transcode_dir()).unwrap_or_default().to_string_lossy(),
        },
        "running": {"port": running.server.port, "tls": app.tls},
    }))
}

pub async fn get_settings(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    ctx.admin()?;
    ok(describe(&app)?)
}

pub async fn save_settings(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>, body: JsonBody) -> ApiResult {
    let u = ctx.admin()?;
    let password = body.string("currentPassword");
    let hash = u.password_hash.clone();
    let password_ok = tokio::task::spawn_blocking(move || auth::verify_password(&password, &hash)).await.unwrap_or(false);
    if !password_ok {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "error.wrongCurrentPassword"));
    }
    if !writable(&app) {
        return Err(ApiError::new(StatusCode::CONFLICT, "error.configReadOnly").arg(app.config_path.to_string_lossy().into_owned()));
    }
    let file = saved(&app)?;
    let mut changes: Vec<(&Field, String)> = Vec::new();
    let mut next: HashMap<&str, String> = settings::FIELDS.iter().map(|f| (f.key, f.current(&file))).collect();
    let submitted = body.get("values").and_then(Value::as_object).cloned().unwrap_or_default();
    for (key, v) in &submitted {
        let f = settings::field(key).ok_or_else(|| ApiError::bad_request("error.unknownSetting").arg(key.clone()))?;
        if f.kind == Kind::Secret && v.is_null() {
            continue; // unchanged
        }
        let value = settings::validate(f, v).map_err(|t| ApiError::from_text(StatusCode::BAD_REQUEST, t))?;
        if value != f.current(&file) {
            changes.push((f, value.clone()));
        }
        next.insert(f.key, value);
    }
    if changes.is_empty() {
        return ok(describe(&app)?);
    }
    check_network(&app, &next)?;
    check_tls(&app, &next, &changes)?;
    if changes.iter().any(|(f, _)| f.key == "transcoder.directory") {
        check_transcode_directory(&app, &next["transcoder.directory"])?;
    }
    settings::write(&app.config_path, &changes).map_err(|e| {
        tracing::warn!("Cannot write {}: {e}", app.config_path.display());
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "error.configWriteFailed").arg(e)
    })?;
    let keys: Vec<&str> = changes.iter().map(|(f, _)| f.key).collect();
    tracing::info!("Settings changed by '{}': {}", u.username, keys.join(", "));
    ok(describe(&app)?)
}

/// A changed address must belong to this machine, a changed port must be free.
fn check_network(app: &AppState, next: &HashMap<&str, String>) -> Result<(), ApiError> {
    let address = next["server.bindAddress"].trim();
    let port: u16 = next["server.port"].trim().parse().map_err(|_| ApiError::bad_request("error.addressInvalid").arg(address))?;
    let ip: IpAddr = address.parse().map_err(|_| ApiError::bad_request("error.addressInvalid").arg(address))?;
    if address != app.config.server.bind && !ip.is_unspecified() && !ip.is_loopback() {
        // Binding to the address (on an ephemeral port) tells whether it belongs to this machine.
        if TcpListener::bind(SocketAddr::new(ip, 0)).is_err() {
            return Err(ApiError::bad_request("error.addressNotLocal").arg(address));
        }
    }
    // The current port is held by this server itself, so only a different port can be tested.
    if port != app.config.server.port
        && let Err(e) = TcpListener::bind(SocketAddr::new(ip, port))
    {
        let key = if port < 1024 && cfg!(unix) { "error.portPrivileged" } else { "error.portUnavailable" };
        return Err(ApiError::bad_request(key).arg(i64::from(port)).arg(e.to_string()));
    }
    Ok(())
}

/// New certificate files must load, otherwise HTTPS would not start after the restart.
fn check_tls(app: &AppState, next: &HashMap<&str, String>, changes: &[(&Field, String)]) -> Result<(), ApiError> {
    if !changes.iter().any(|(f, _)| f.key == "server.tlsCertificate" || f.key == "server.tlsKey") {
        return Ok(());
    }
    let (cert, key) = (next["server.tlsCertificate"].trim(), next["server.tlsKey"].trim());
    if cert.is_empty() && key.is_empty() {
        return Ok(());
    }
    if cert.is_empty() != key.is_empty() {
        return Err(ApiError::bad_request("error.tlsBothFiles"));
    }
    let (cert, key) = (app.config.resolve(Path::new(cert)), app.config.resolve(Path::new(key)));
    for p in [&cert, &key] {
        if !p.is_file() {
            return Err(ApiError::bad_request("error.tlsFileMissing").arg(p.to_string_lossy().into_owned()));
        }
    }
    crate::tls::server_config(&cert, &key).map(|_| ()).map_err(|e| ApiError::bad_request("error.tlsInvalid").arg(e))
}

/// The transcoding directory must exist (or be creatable) and be writable for the server. Under
/// the hardened systemd service only a few paths are writable; for others a drop-in is needed.
fn check_transcode_directory(app: &AppState, value: &str) -> Result<(), ApiError> {
    let v = Path::new(value.trim());
    let dir = if v.is_absolute() { v.to_path_buf() } else { app.config.cache_dir().join(v) };
    let probe = dir.join(".freex-write-test");
    let result = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&probe, "ok")).and_then(|()| std::fs::remove_file(&probe));
    result.map_err(|e| {
        let why: Text = match e.kind() {
            std::io::ErrorKind::PermissionDenied => Text::new("error.accessDenied"),
            std::io::ErrorKind::NotFound => Text::new("error.noSuchPath"),
            _ => Text::new("text.plain").arg(e.to_string()),
        };
        ApiError::bad_request("error.directoryNotWritable").arg(dir.to_string_lossy().into_owned()).arg(why)
    })
}

/// Ends the server loop, which starts again with the configuration file as it is now.
pub async fn restart(State(app): State<Arc<AppState>>, Extension(ctx): Extension<Ctx>) -> ApiResult {
    let u = ctx.admin()?;
    tracing::info!("Restart requested by '{}'", u.username);
    // Answer first; the restart closes the server this request runs on.
    let notify = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        notify.restart.notify_one();
    });
    ok(json!({"restarting": true}))
}
