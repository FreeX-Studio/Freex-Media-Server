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

//! Watch Together: rooms in which several users watch the same title at the same position.
//! The room keeps one clock (item, playing or paused, position at a server time); every member
//! sends their actions and follows the room's state, which arrives as server-sent events.
//! While a member's player is loading, the clock stops for everyone.

use crate::auth::now_ms;
use crate::db::User;
use rand::Rng;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

const CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const EMPTY_ROOM_TTL_MS: i64 = 10 * 60_000;
const IDLE_ROOM_TTL_MS: i64 = 8 * 3_600_000;
/// A player that reports "loading" for longer than this no longer holds up the others.
const WAIT_LIMIT_MS: i64 = 20_000;
const QUEUE: usize = 200;

/// One open event stream of a member.
struct Subscriber {
    id: u64,
    user_id: String,
    tx: mpsc::Sender<(String, Value)>,
}

struct Room {
    code: String,
    host_id: String,
    item_id: String,
    playing: bool,
    position_ms: i64,
    changed_at: i64,
    changed_by: String,
    /// Counts every change, so clients can drop states older than one they know.
    version: i64,
    last_activity: i64,
    /// Everyone who joined: user id, display name and profile picture version (as of joining).
    members: Vec<(String, String, i64)>,
    /// Members whose player is loading: user id → since when.
    waiting: Vec<(String, i64)>,
    subscribers: Vec<Subscriber>,
}

impl Room {
    /// Whether the position advances: playing, and nobody is loading.
    fn running(&self) -> bool {
        self.playing && self.waiting.is_empty()
    }

    /// Position at `now`, counting the time since the last change while running.
    fn position_at(&self, now: i64) -> i64 {
        if self.running() { self.position_ms + (now - self.changed_at).max(0) } else { self.position_ms }
    }

    /// Fixes the position at now; called before anything that starts or stops the clock.
    fn freeze(&mut self, now: i64) {
        self.position_ms = self.position_at(now);
        self.changed_at = now;
    }

    fn is_member(&self, user_id: &str) -> bool {
        self.members.iter().any(|(id, _, _)| id == user_id)
    }

    fn add_member(&mut self, user: &User) -> bool {
        if self.is_member(&user.id) {
            return false;
        }
        self.members.push((user.id.clone(), user.username.clone(), user.avatar_version));
        true
    }

    fn state(&self) -> Value {
        let now = now_ms();
        let connected: HashSet<&str> = self.subscribers.iter().map(|s| s.user_id.as_str()).collect();
        let name = |id: &str| self.members.iter().find(|(m, _, _)| m == id).map(|(_, n, _)| n.clone()).unwrap_or_default();
        json!({
            "code": self.code,
            "hostId": self.host_id,
            "itemId": self.item_id,
            // playing is what the members chose; running says whether the position moves right now
            "playing": self.playing,
            "running": self.running(),
            "positionMs": self.position_at(now),
            "changedBy": self.changed_by,
            "version": self.version,
            "serverTime": now,
            "waiting": self.waiting.iter().map(|(id, _)| json!({"id": id, "name": name(id)})).collect::<Vec<_>>(),
            "members": self.members.iter().map(|(id, n, avatar)| json!({"id": id, "name": n, "online": connected.contains(id.as_str()),
                                                                         "avatar": avatar})).collect::<Vec<_>>(),
        })
    }

    fn broadcast(&mut self, kind: &str) {
        let state = self.state();
        // A client that stopped reading loses its stream.
        self.subscribers.retain(|s| s.tx.try_send((kind.to_string(), state.clone())).is_ok());
    }

    /// Marks a member as loading or ready; true if that changed the state.
    fn set_waiting(&mut self, user_id: &str, on: bool) -> bool {
        let is_waiting = self.waiting.iter().any(|(id, _)| id == user_id);
        if on == is_waiting {
            return false;
        }
        let now = now_ms();
        self.freeze(now);
        if on {
            self.waiting.push((user_id.to_string(), now));
        } else {
            self.waiting.retain(|(id, _)| id != user_id);
        }
        self.last_activity = now;
        self.version += 1;
        true
    }
}

/// What a member asks the room to do.
pub enum Action {
    Play,
    Pause,
    Seek,
    /// Switch to another title (e.g. the next episode).
    Item(String),
    Buffering,
    Ready,
}

impl Action {
    pub fn parse(action: &str, item_id: Option<&str>) -> Result<Action, &'static str> {
        Ok(match action {
            "play" => Action::Play,
            "pause" => Action::Pause,
            "seek" => Action::Seek,
            "buffering" => Action::Buffering,
            "ready" => Action::Ready,
            "item" => Action::Item(item_id.filter(|s| !s.trim().is_empty()).ok_or("error.itemIdMissing")?.to_string()),
            _ => return Err("error.unknownAction"),
        })
    }
}

#[derive(Debug)]
pub enum UpdateError {
    NotMember,
}

pub struct Together {
    rooms: Mutex<HashMap<String, Room>>,
    next_subscriber: Mutex<u64>,
}

/// An open event stream; dropping it leaves the stream.
pub struct Subscription {
    pub code: String,
    pub rx: mpsc::Receiver<(String, Value)>,
    id: u64,
    user_id: String,
    together: Arc<Together>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.together.unsubscribe(&self.code, self.id, &self.user_id);
    }
}

impl Together {
    pub fn new() -> Arc<Together> {
        let t = Arc::new(Together { rooms: Mutex::default(), next_subscriber: Mutex::new(0) });
        let weak = Arc::downgrade(&t);
        tokio::spawn(async move {
            let mut n = 0u32;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                let Some(t) = weak.upgrade() else { break };
                t.expire_waiting();
                n += 1;
                if n.is_multiple_of(30) {
                    t.cleanup();
                }
            }
        });
        t
    }

    fn with<R>(&self, code: &str, f: impl FnOnce(&mut Room) -> R) -> Option<R> {
        let mut rooms = self.rooms.lock().unwrap();
        let room = rooms.get_mut(&code.trim().to_uppercase())?;
        Some(f(room))
    }

    /// The current state of a room, or None if it does not exist.
    pub fn state(&self, code: &str) -> Option<Value> {
        self.with(code, |r| r.state())
    }

    /// The title a room plays.
    pub fn item_of(&self, code: &str) -> Option<String> {
        self.with(code, |r| r.item_id.clone())
    }

    pub fn create(&self, host: &User, item_id: &str, position_ms: i64) -> Value {
        let mut rooms = self.rooms.lock().unwrap();
        let mut rng = rand::thread_rng();
        let code = loop {
            let c: String = (0..6).map(|_| CODE_CHARS[rng.gen_range(0..CODE_CHARS.len())] as char).collect();
            if !rooms.contains_key(&c) {
                break c;
            }
        };
        let now = now_ms();
        let room = Room {
            code: code.clone(),
            host_id: host.id.clone(),
            item_id: item_id.to_string(),
            playing: true, // the host pressed "watch together": start right away
            position_ms: position_ms.max(0),
            changed_at: now,
            changed_by: String::new(),
            version: 0,
            last_activity: now,
            members: vec![(host.id.clone(), host.username.clone(), host.avatar_version)],
            waiting: Vec::new(),
            subscribers: Vec::new(),
        };
        let state = room.state();
        rooms.insert(code.clone(), room);
        tracing::info!("Watch Together room {code} created by '{}'", host.username);
        state
    }

    /// Adds a user to the room; None if the room does not exist.
    pub fn join(&self, code: &str, user: &User) -> Option<Value> {
        self.with(code, |r| {
            if r.add_member(user) {
                r.broadcast("members");
            }
            r.last_activity = now_ms();
            r.state()
        })
    }

    /// Applies a change from a member. None if the room does not exist.
    pub fn update(&self, code: &str, user: &User, action: Action, position_ms: i64) -> Option<Result<Value, UpdateError>> {
        self.with(code, |r| {
            if !r.is_member(&user.id) {
                return Err(UpdateError::NotMember);
            }
            match action {
                Action::Buffering | Action::Ready => {
                    if r.set_waiting(&user.id, matches!(action, Action::Buffering)) {
                        r.broadcast("state");
                    }
                    return Ok(r.state());
                }
                Action::Play => r.playing = true,
                Action::Pause => r.playing = false,
                Action::Seek => {} // keeps play/pause
                Action::Item(id) => {
                    r.item_id = id;
                    r.playing = true;
                    r.waiting.clear(); // everyone loads the new title and reports again
                }
            }
            r.position_ms = position_ms.max(0);
            r.changed_at = now_ms();
            r.changed_by = user.id.clone();
            r.last_activity = r.changed_at;
            r.version += 1;
            r.broadcast("state");
            Ok(r.state())
        })
    }

    /// Opens an event stream for a member; the first event is the current state.
    pub fn subscribe(self: &Arc<Self>, code: &str, user: &User) -> Option<Subscription> {
        let id = {
            let mut n = self.next_subscriber.lock().unwrap();
            *n += 1;
            *n
        };
        let (tx, rx) = mpsc::channel(QUEUE);
        let code = code.trim().to_uppercase();
        self.with(&code, |r| {
            r.add_member(user);
            r.last_activity = now_ms();
            let _ = tx.try_send(("state".to_string(), r.state()));
            r.subscribers.push(Subscriber { id, user_id: user.id.clone(), tx });
            r.broadcast("members");
        })?;
        Some(Subscription { code, rx, id, user_id: user.id.clone(), together: self.clone() })
    }

    fn unsubscribe(&self, code: &str, id: u64, user_id: &str) {
        self.with(code, |r| {
            r.subscribers.retain(|s| s.id != id);
            r.last_activity = now_ms();
            // Whoever left no longer holds up the others.
            let still_there = r.subscribers.iter().any(|s| s.user_id == user_id);
            let kind = if !still_there && r.set_waiting(user_id, false) { "state" } else { "members" };
            r.broadcast(kind);
        });
    }

    /// Removes a user from a room; the room ends with its last member.
    pub fn leave(&self, code: &str, user: &User) {
        let mut rooms = self.rooms.lock().unwrap();
        let key = code.trim().to_uppercase();
        let Some(r) = rooms.get_mut(&key) else { return };
        r.set_waiting(&user.id, false);
        r.members.retain(|(id, _, _)| *id != user.id);
        r.subscribers.retain(|s| s.user_id != user.id); // dropping the senders ends their streams
        r.broadcast("members");
        if r.members.is_empty() {
            rooms.remove(&key);
        }
    }

    fn expire_waiting(&self) {
        let now = now_ms();
        let mut rooms = self.rooms.lock().unwrap();
        for r in rooms.values_mut() {
            let expired: Vec<String> =
                r.waiting.iter().filter(|(_, since)| now - since > WAIT_LIMIT_MS).map(|(id, _)| id.clone()).collect();
            let mut changed = false;
            for id in expired {
                changed |= r.set_waiting(&id, false);
            }
            if changed {
                r.broadcast("state");
            }
        }
    }

    fn cleanup(&self) {
        let now = now_ms();
        self.rooms.lock().unwrap().retain(|_, r| {
            !((r.subscribers.is_empty() && now - r.last_activity > EMPTY_ROOM_TTL_MS) || now - r.last_activity > IDLE_ROOM_TTL_MS)
        });
    }

    pub fn room_count(&self) -> usize {
        self.rooms.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> User {
        User { id: id.into(), username: id.to_uppercase(), ..User::default() }
    }

    #[tokio::test]
    async fn keeps_one_clock() {
        let t = Together::new();
        let host = user("h");
        let state = t.create(&host, "item1", 5000);
        let code = state["code"].as_str().unwrap().to_string();
        assert_eq!(code.len(), 6);
        let guest = user("g");
        assert!(t.update(&code, &guest, Action::Pause, 0).unwrap().is_err()); // not a member yet
        t.join(&code, &guest).unwrap();
        let mut sub = t.subscribe(&code, &guest).unwrap();
        let (kind, first) = sub.rx.recv().await.unwrap();
        assert_eq!(kind, "state");
        assert!(first["positionMs"].as_i64().unwrap() >= 5000);
        // The guest's player loads: the clock stops for everyone.
        let s = t.update(&code, &guest, Action::Buffering, 0).unwrap().unwrap();
        assert!(s["playing"].as_bool().unwrap() && !s["running"].as_bool().unwrap());
        let s = t.update(&code, &guest, Action::Ready, 0).unwrap().unwrap();
        assert!(s["running"].as_bool().unwrap());
        let s = t.update(&code, &host, Action::Seek, 60_000).unwrap().unwrap();
        assert_eq!(s["changedBy"], "h");
        assert!(s["positionMs"].as_i64().unwrap() >= 60_000);
        // The events arrive at the subscriber (members on subscribe, then the states).
        let mut kinds = Vec::new();
        while let Ok((k, _)) = sub.rx.try_recv() {
            kinds.push(k);
        }
        assert!(kinds.contains(&"state".to_string()));
        t.leave(&code, &guest);
        t.leave(&code, &host);
        assert_eq!(t.room_count(), 0);
        assert!(t.state(&code).is_none());
    }
}
