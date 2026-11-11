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

//! Webhooks: an HTTP POST with a JSON body to addresses the admins set up, when a playback
//! starts, pauses, resumes or stops, when a title counts as watched, and when new titles arrive.
//! For home automation (dim the lights when a film starts) and notifications. Every request is
//! signed with HMAC-SHA256 over the body, so the receiver can check it came from this server.

use crate::AppState;
use crate::auth::now_ms;
use crate::db::{Db, User};
use crate::media::catalog::Index;
use crate::media::{Item, Kind};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Events a webhook can subscribe to.
pub const EVENTS: [&str; 6] = ["play", "pause", "resume", "stop", "watched", "library.new"];

#[derive(Clone, Debug)]
pub struct Hook {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub secret: String,
    pub created_at: i64,
}

/// The outcome of the last delivery to a hook.
#[derive(Clone, Debug)]
pub struct Delivery {
    pub at: i64,
    pub event: String,
    pub ok: bool,
    /// HTTP status, or the error.
    pub detail: String,
}

pub struct Webhooks {
    http: reqwest::Client,
    hooks: RwLock<Vec<Hook>>,
    last: Mutex<HashMap<String, Delivery>>,
}

impl Webhooks {
    pub fn new(db: &Db) -> Webhooks {
        let http = reqwest::Client::builder()
            .user_agent(concat!("FreeX-Media-Server/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("HTTP client");
        Webhooks { http, hooks: RwLock::new(db.webhooks()), last: Mutex::default() }
    }

    pub fn reload(&self, db: &Db) {
        *self.hooks.write().unwrap() = db.webhooks();
    }

    pub fn last(&self, id: &str) -> Option<Delivery> {
        self.last.lock().unwrap().get(id).cloned()
    }

    /// Sends an event to every hook that wants it, in the background.
    pub fn emit(app: &Arc<AppState>, event: &str, data: Value) {
        let hooks: Vec<Hook> = app.webhooks.hooks.read().unwrap().iter().filter(|h| h.events.iter().any(|e| e == event)).cloned().collect();
        if hooks.is_empty() {
            return;
        }
        let body = envelope(&app.config.server.name, event, data);
        for hook in hooks {
            let (app, body, event) = (app.clone(), body.clone(), event.to_string());
            tokio::spawn(async move {
                app.webhooks.deliver(&hook, &event, &body, true).await;
            });
        }
    }

    /// Sends a test event to one hook and waits for the answer.
    pub async fn test(&self, server: &str, hook: &Hook) -> Delivery {
        let body = envelope(server, "test", json!({"message": "Test from FreeX Media Server"}));
        self.deliver(hook, "test", &body, false).await
    }

    async fn deliver(&self, hook: &Hook, event: &str, body: &str, retry: bool) -> Delivery {
        let mut d = self.post(hook, event, body).await;
        if !d.ok && retry {
            // One more try a little later: the receiver may be restarting.
            tokio::time::sleep(Duration::from_secs(5)).await;
            d = self.post(hook, event, body).await;
        }
        if !d.ok {
            tracing::warn!("Webhook {} ({event}) failed: {}", hook.url, d.detail);
        }
        self.last.lock().unwrap().insert(hook.id.clone(), d.clone());
        d
    }

    async fn post(&self, hook: &Hook, event: &str, body: &str) -> Delivery {
        let r = self
            .http
            .post(&hook.url)
            .header("Content-Type", "application/json")
            .header("X-FreeX-Event", event)
            .header("X-FreeX-Signature", format!("sha256={}", sign(&hook.secret, body)))
            .body(body.to_string())
            .send()
            .await;
        let (ok, detail) = match r {
            Ok(res) => (res.status().is_success(), res.status().as_u16().to_string()),
            Err(e) => (false, short_error(&e)),
        };
        Delivery { at: now_ms(), event: event.to_string(), ok, detail }
    }
}

fn short_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timeout".into()
    } else if e.is_connect() {
        "connection failed".into()
    } else {
        e.to_string().chars().take(200).collect()
    }
}

/// Hex HMAC-SHA256 of the body with the hook's secret.
pub fn sign(secret: &str, body: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("any key length");
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn envelope(server: &str, event: &str, data: Value) -> String {
    let mut m = json!({"event": event, "time": now_ms(), "server": server});
    if let (Some(m), Value::Object(d)) = (m.as_object_mut(), data) {
        m.extend(d);
    }
    m.to_string()
}

/// A title as webhooks describe it: no file system paths.
pub fn item_json(idx: &Index, it: &Item) -> Value {
    let mut m = json!({"id": it.key(), "type": it.kind, "title": it.title, "year": it.year});
    if it.kind == Kind::Episode {
        m["episode"] = json!(it.index);
        m["season"] = json!(idx.parent(it).map_or(0, |s| s.index));
        m["show"] = json!(idx.show_of(it).map(|s| s.title.clone()));
    }
    if let Some(f) = &it.file {
        m["durationMs"] = json!(f.duration_ms);
    }
    m
}

pub fn user_json(u: &User) -> Value {
    json!({"id": u.id, "username": u.username})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_like_the_reference() {
        // RFC 4231 test case 2.
        assert_eq!(sign("Jefe", "what do ya want for nothing?"), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        let e: Value = serde_json::from_str(&envelope("FreeX", "play", json!({"user": {"id": "u"}}))).unwrap();
        assert_eq!((e["event"].as_str(), e["server"].as_str(), e["user"]["id"].as_str()), (Some("play"), Some("FreeX"), Some("u")));
    }
}
