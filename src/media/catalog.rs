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

//! The library index in memory. It is read on every page, so it lives in RAM as an immutable
//! [`Index`] snapshot: readers take the current snapshot without waiting, a finished scan builds
//! a new one and swaps it in. The database keeps a copy for the next start.

use super::{Item, Kind};
use crate::db::Db;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// One consistent state of all libraries.
#[derive(Default)]
pub struct Index {
    items: HashMap<String, Arc<Item>>,
    /// Children by parent id, ordered by number and title.
    children: HashMap<String, Vec<Arc<Item>>>,
    /// Movies or shows by library id.
    top: HashMap<String, Vec<Arc<Item>>>,
    /// Extras by the id of their movie or show; they are neither children nor top-level entries.
    extras: HashMap<String, Vec<Arc<Item>>>,
}

impl Index {
    fn build(items: HashMap<String, Arc<Item>>) -> Index {
        let mut children: HashMap<String, Vec<Arc<Item>>> = HashMap::new();
        let mut top: HashMap<String, Vec<Arc<Item>>> = HashMap::new();
        let mut extras: HashMap<String, Vec<Arc<Item>>> = HashMap::new();
        for it in items.values() {
            if it.kind == Kind::Extra {
                if let Some(p) = &it.parent_id {
                    extras.entry(p.clone()).or_default().push(it.clone());
                }
                continue;
            }
            match &it.parent_id {
                Some(p) => children.entry(p.clone()).or_default().push(it.clone()),
                None => top.entry(it.library_id.clone()).or_default().push(it.clone()),
            }
        }
        for list in children.values_mut() {
            list.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.sort_title.cmp(&b.sort_title)));
        }
        for list in top.values_mut() {
            list.sort_by(|a, b| a.sort_title.cmp(&b.sort_title));
        }
        for list in extras.values_mut() {
            list.sort_by(|a, b| extra_order(&a.extra_type).cmp(&extra_order(&b.extra_type)).then_with(|| a.sort_title.cmp(&b.sort_title)));
        }
        Index { items, children, top, extras }
    }

    /// Trailers, featurettes and other bonus videos of a movie or show.
    pub fn extras(&self, id: &str) -> &[Arc<Item>] {
        self.extras.get(id).map_or(&[], Vec::as_slice)
    }

    pub fn item(&self, id: &str) -> Option<&Arc<Item>> {
        self.items.get(id)
    }

    pub fn parent(&self, it: &Item) -> Option<&Arc<Item>> {
        it.parent_id.as_deref().and_then(|p| self.items.get(p))
    }

    pub fn children(&self, id: &str) -> &[Arc<Item>] {
        self.children.get(id).map_or(&[], Vec::as_slice)
    }

    /// Movies or shows of a library, by title.
    pub fn top_level(&self, library_id: &str) -> &[Arc<Item>] {
        self.top.get(library_id).map_or(&[], Vec::as_slice)
    }

    /// Movies, shows, seasons and episodes of all libraries, without extras.
    pub fn all(&self) -> impl Iterator<Item = &Arc<Item>> {
        self.items.values().filter(|it| it.kind != Kind::Extra)
    }

    /// Every entry of a library, extras included.
    pub fn of_library<'a>(&'a self, library_id: &'a str) -> impl Iterator<Item = &'a Arc<Item>> {
        self.items.values().filter(move |it| it.library_id == library_id)
    }

    /// All episodes of a show in viewing order: regular seasons in order, specials last.
    pub fn episodes_of(&self, show: &Item) -> Vec<Arc<Item>> {
        let mut seasons: Vec<&Arc<Item>> = self.children(&show.id).iter().collect();
        seasons.sort_by_key(|s| if s.index == 0 { i32::MAX } else { s.index });
        seasons.iter().flat_map(|s| self.children(&s.id).iter().cloned()).collect()
    }

    /// The show an episode or season belongs to.
    pub fn show_of(&self, it: &Item) -> Option<&Arc<Item>> {
        match it.kind {
            Kind::Season => self.parent(it),
            Kind::Episode => self.parent(it).and_then(|s| self.parent(s)),
            _ => None,
        }
    }
}

/// Trailers first, then the other kinds in a fixed order.
fn extra_order(t: &str) -> usize {
    ["trailer", "featurette", "behindTheScenes", "deletedScene", "interview", "scene", "short", "other"]
        .iter()
        .position(|x| *x == t)
        .unwrap_or(9)
}

pub struct Catalog {
    current: RwLock<Arc<Index>>,
}

impl Catalog {
    /// The index as stored in the database.
    pub fn load(db: &Db) -> Catalog {
        let mut items = HashMap::new();
        let mut broken = 0;
        for data in db.items() {
            match serde_json::from_str::<Item>(&data) {
                Ok(it) => {
                    items.insert(it.id.clone(), Arc::new(it));
                }
                Err(_) => broken += 1,
            }
        }
        if broken > 0 {
            tracing::warn!("{broken} entries of the library index could not be read; the next scan restores them");
        }
        Catalog { current: RwLock::new(Arc::new(Index::build(items))) }
    }

    /// The current state; stays valid and unchanged while it is used.
    pub fn snapshot(&self) -> Arc<Index> {
        self.current.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Replaces the entries of one library, in memory and in the database.
    pub fn replace_library(&self, db: &Db, library_id: &str, scanned: Vec<Item>) {
        let rows: Vec<(String, String)> =
            scanned.iter().map(|it| (it.id.clone(), serde_json::to_string(it).expect("serializable"))).collect();
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        // A library deleted during its scan stays deleted.
        if !db.replace_items(library_id, &rows) {
            return;
        }
        let mut items: HashMap<String, Arc<Item>> =
            current.items.iter().filter(|(_, it)| it.library_id != library_id).map(|(k, v)| (k.clone(), v.clone())).collect();
        for it in scanned {
            items.insert(it.id.clone(), Arc::new(it));
        }
        *current = Arc::new(Index::build(items));
    }

    /// Replaces one entry (same id, same parent), in memory and in the database.
    pub fn update_item(&self, db: &Db, item: Item) {
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        if !current.items.contains_key(&item.id) {
            return;
        }
        db.save_item(&item.id, &serde_json::to_string(&item).expect("serializable"));
        let mut items = current.items.clone();
        items.insert(item.id.clone(), Arc::new(item));
        *current = Arc::new(Index::build(items));
    }

    /// Changes the entries of one library in place (in memory and in the database), atomically
    /// with respect to scans.
    pub fn update_library(&self, db: &Db, library_id: &str, change: impl FnOnce(&mut Vec<Item>)) {
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        let mut items: Vec<Item> = current.of_library(library_id).map(|it| Item::clone(it)).collect();
        if items.is_empty() {
            return;
        }
        change(&mut items);
        let rows: Vec<(String, String)> =
            items.iter().map(|it| (it.id.clone(), serde_json::to_string(it).expect("serializable"))).collect();
        if !db.replace_items(library_id, &rows) {
            return;
        }
        let mut all: HashMap<String, Arc<Item>> =
            current.items.iter().filter(|(_, it)| it.library_id != library_id).map(|(k, v)| (k.clone(), v.clone())).collect();
        for it in items {
            all.insert(it.id.clone(), Arc::new(it));
        }
        *current = Arc::new(Index::build(all));
    }

    /// Forgets a deleted library (the database removes its rows itself).
    pub fn remove_library(&self, library_id: &str) {
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        let items = current.items.iter().filter(|(_, it)| it.library_id != library_id).map(|(k, v)| (k.clone(), v.clone())).collect();
        *current = Arc::new(Index::build(items));
    }
}
