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

//! Running playbacks for the activity menu of admins: who watches what, on which device, how
//! (direct or converted) and where. Players announce a playback when they start it and report
//! the position regularly; a playback without a report for 45 seconds counts as ended.

use crate::auth::now_ms;
use crate::i18n::Text;
use std::collections::HashMap;
use std::sync::Mutex;

const TIMEOUT_MS: i64 = 45_000;

#[derive(Clone, Debug)]
pub struct Playback {
    pub user_id: String,
    pub item_id: String,
    pub client: String,
    pub started_at: i64,
    pub mode: String,
    pub reason: Text,
    pub quality: String,
    pub session: Option<String>,
    pub ip: String,
    pub user_agent: String,
    pub device: String,
    pub offset_ms: i64,
    pub state: String,
    pub last_seen: i64,
    /// Estimated bits per second of the stream.
    pub bitrate: i64,
}

/// What a player announces when it starts.
pub struct Start<'a> {
    pub user_id: &'a str,
    pub item_id: &'a str,
    pub client: Option<&'a str>,
    pub device: &'a str,
    pub mode: &'a str,
    pub reason: Text,
    pub quality: &'a str,
    pub session: Option<String>,
    pub offset_ms: i64,
    pub ip: &'a str,
    pub user_agent: &'a str,
    pub bitrate: i64,
}

#[derive(Default)]
pub struct Activity {
    playbacks: Mutex<HashMap<String, Playback>>,
}

/// Client ids are chosen by the player; anything but a short token is ignored.
pub fn client(raw: Option<&str>) -> Option<&str> {
    raw.filter(|c| (1..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}

fn key(user: &str, item: &str, client: &str) -> String {
    format!("{user}|{item}|{client}")
}

impl Activity {
    /// A player started (or restarted after a jump or a quality change) a playback; true if it
    /// is a new one.
    pub fn started(&self, s: Start) -> bool {
        let Some(client) = client(s.client) else { return false };
        let now = now_ms();
        let mut all = self.playbacks.lock().unwrap();
        let k = key(s.user_id, s.item_id, client);
        let fresh = all.get(&k).is_none_or(|p| now - p.last_seen > TIMEOUT_MS);
        let p = all.entry(k).or_insert_with(|| Playback {
            user_id: s.user_id.to_string(),
            item_id: s.item_id.to_string(),
            client: client.to_string(),
            started_at: now,
            mode: String::new(),
            reason: Text::new(""),
            quality: String::new(),
            session: None,
            ip: String::new(),
            user_agent: String::new(),
            device: String::new(),
            offset_ms: 0,
            state: String::new(),
            last_seen: now,
            bitrate: 0,
        });
        p.mode = s.mode.to_string();
        p.reason = s.reason;
        p.quality = s.quality.to_string();
        p.session = s.session;
        p.ip = s.ip.to_string();
        p.user_agent = s.user_agent.chars().take(300).collect();
        p.device = s.device.chars().take(80).collect();
        p.offset_ms = s.offset_ms;
        p.state = "playing".into();
        p.last_seen = now;
        p.bitrate = s.bitrate;
        fresh
    }

    /// Sets the state of an announced playback, e.g. `starting` while its conversion gets ready.
    pub fn set_state(&self, user_id: &str, item_id: &str, client_id: Option<&str>, state: &str) {
        let Some(client) = client(client_id) else { return };
        if let Some(p) = self.playbacks.lock().unwrap().get_mut(&key(user_id, item_id, client)) {
            p.state = state.to_string();
        }
    }

    /// A position report; `stopped` ends the playback. Returns what changed for webhooks:
    /// `pause`, `resume` or `stop`.
    pub fn progress(
        &self,
        user_id: &str,
        item_id: &str,
        client_id: Option<&str>,
        offset_ms: i64,
        state: &str,
        stopped: bool,
    ) -> Option<&'static str> {
        let client = client(client_id)?;
        let k = key(user_id, item_id, client);
        let mut all = self.playbacks.lock().unwrap();
        if stopped {
            return all.remove(&k).map(|_| "stop");
        }
        // Not announced (e.g. after a restart of the server): the next start announces it.
        let p = all.get_mut(&k)?;
        p.offset_ms = offset_ms;
        let before = std::mem::take(&mut p.state);
        p.state = if matches!(state, "paused" | "playing" | "buffering") { state.to_string() } else { before.clone() };
        p.last_seen = now_ms();
        match (before.as_str(), p.state.as_str()) {
            ("playing" | "buffering", "paused") => Some("pause"),
            ("paused", "playing") => Some("resume"),
            _ => None,
        }
    }

    /// Whether someone is watching right now (playing or buffering, not paused).
    pub fn watching(&self) -> bool {
        let cutoff = now_ms() - TIMEOUT_MS;
        self.playbacks.lock().unwrap().values().any(|p| p.last_seen >= cutoff && p.state != "paused")
    }

    /// Running playbacks, oldest first.
    pub fn list(&self) -> Vec<Playback> {
        let cutoff = now_ms() - TIMEOUT_MS;
        let mut all = self.playbacks.lock().unwrap();
        all.retain(|_, p| p.last_seen >= cutoff);
        let mut v: Vec<Playback> = all.values().cloned().collect();
        v.sort_by_key(|p| p.started_at);
        v
    }

    pub fn count(&self) -> usize {
        self.list().len()
    }
}

/// Background media work (seek previews, intro detection) waits here while someone watches, so
/// that playback gets the processor and the disks; it goes on as soon as nobody does. What is
/// left over runs in the nightly maintenance at the latest.
pub async fn yield_to_playback(app: &crate::AppState) {
    if !app.activity.watching() {
        return;
    }
    tracing::debug!("Background work paused while a title plays");
    while app.activity.watching() {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

/// How many background media jobs may run at once when nobody watches: one per four cores,
/// at most three.
pub fn background_jobs() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    (cores / 4).clamp(1, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start<'a>(item: &'a str, c: &'a str) -> Start<'a> {
        Start {
            user_id: "u",
            item_id: item,
            client: Some(c),
            device: "Chrome",
            mode: "direct",
            reason: Text::new("playback.direct"),
            quality: "Original",
            session: None,
            offset_ms: 0,
            ip: "1.2.3.4",
            user_agent: "UA",
            bitrate: 0,
        }
    }

    #[test]
    fn tracks_playbacks() {
        let a = Activity::default();
        a.started(start("i1", "c1"));
        a.started(start("i1", "c1")); // restart after a jump: still one
        a.started(start("i2", "bad client!"));
        assert_eq!(a.count(), 1);
        a.progress("u", "i1", Some("c1"), 5000, "paused", false);
        assert_eq!(a.list()[0].state, "paused");
        a.progress("u", "i1", Some("c1"), 6000, "playing", true);
        assert_eq!(a.count(), 0);
    }
}
