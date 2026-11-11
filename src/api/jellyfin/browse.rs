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

//! Browsing for the apps: libraries, the `/Items` query with its filters, sorting and paging,
//! the rows of the home screen (latest, resume, next up), seasons and episodes, genres,
//! artwork and the intro and credits markers as media segments.

use super::dto::{Dto, image_tag, kind_of, ticks};
use super::{Cx, Id, Out, genre_guid, item_guid, json_out, not_found, parse_guid, query_result};
use crate::api::media::{IMAGE_CACHE_SECONDS, image_path, may_see, visible_item};
use crate::media::{Item, Kind};
use axum::http::StatusCode;
use rand::seq::SliceRandom;
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// The libraries of the user as the top-level folders of the apps.
pub fn views(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let libs: Vec<Value> = cx.app.db.libraries().iter().filter(|l| u.may_access(&l.id)).map(|l| d.library(l)).collect();
    let n = libs.len();
    json_out(query_result(libs, n, 0))
}

/// One entry or library.
pub fn item(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    match parse_guid(raw) {
        Some(Id::Item(id)) => {
            let it = visible_item(&idx, u, &id).map_err(|_| not_found())?;
            json_out(d.item(&it, true))
        }
        Some(Id::Library(id)) => {
            let l = cx.app.db.library(&id).filter(|l| u.may_access(&l.id)).ok_or_else(not_found)?;
            json_out(d.library(&l))
        }
        _ => Err(not_found()),
    }
}

/// An entry the user may see, by GUID.
fn visible(cx: &Cx, idx: &crate::media::catalog::Index, raw: &str) -> Result<Arc<Item>, super::Fail> {
    let u = cx.user()?;
    match parse_guid(raw) {
        Some(Id::Item(id)) => visible_item(idx, u, &id).map_err(|_| not_found()),
        _ => Err(not_found()),
    }
}

fn descendants(idx: &crate::media::catalog::Index, it: &Arc<Item>, out: &mut Vec<Arc<Item>>) {
    for c in idx.children(&it.id) {
        out.push(c.clone());
        descendants(idx, c, out);
    }
}

/// `GET /Items`: the general query of the apps.
pub fn items(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let r = &cx.req;
    let ids = r.list("ids");
    let recursive = r.bool("recursive").unwrap_or(false);
    let types_given = r.list_lower("includeItemTypes");
    let include: Vec<Kind> = types_given.iter().filter_map(|t| kind_of(t)).collect();
    let exclude: Vec<Kind> = r.list_lower("excludeItemTypes").iter().filter_map(|t| kind_of(t)).collect();
    let search = r.param("searchTerm").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let filters = r.list_lower("filters");
    let full = r.list_lower("fields").iter().any(|f| f == "mediasources" || f == "mediastreams");

    // Libraries asked for by id come first, as folders.
    let mut folders: Vec<Value> = Vec::new();
    let mut list: Vec<Arc<Item>> = if !ids.is_empty() {
        let mut out = Vec::new();
        for g in &ids {
            match parse_guid(g) {
                Some(Id::Item(i)) => out.extend(idx.item(&i).cloned()),
                Some(Id::Library(l)) => {
                    if let Some(lib) = cx.app.db.library(&l).filter(|l| u.may_access(&l.id)) {
                        folders.push(d.library(&lib));
                    }
                }
                _ => {}
            }
        }
        out
    } else {
        match r.param("parentId").as_deref().and_then(parse_guid) {
            Some(Id::Library(l)) if u.may_access(&l) => {
                if recursive {
                    idx.of_library(&l).filter(|it| it.kind != Kind::Extra).cloned().collect()
                } else {
                    idx.top_level(&l).to_vec()
                }
            }
            Some(Id::Library(_)) | Some(Id::Genre(_)) => Vec::new(),
            Some(Id::Item(i)) => {
                let p = visible_item(&idx, u, &i).map_err(|_| not_found())?;
                if recursive {
                    let mut out = Vec::new();
                    descendants(&idx, &p, &mut out);
                    out
                } else {
                    idx.children(&p.id).to_vec()
                }
            }
            Some(Id::User(_)) | None => {
                if recursive || search.is_some() || !ids.is_empty() || !types_given.is_empty() {
                    idx.all().cloned().collect()
                } else {
                    // The root folder: the libraries.
                    return views(cx);
                }
            }
        }
    };

    list.retain(|it| d.view.sees(it));
    if !types_given.is_empty() {
        list.retain(|it| include.contains(&it.kind));
    } else if ids.is_empty() {
        list.retain(|it| it.kind != Kind::Extra);
    }
    list.retain(|it| !exclude.contains(&it.kind));
    if r.list_lower("mediaTypes").iter().any(|t| t == "video") {
        list.retain(|it| it.kind.playable());
    }
    if let Some(folder) = r.bool("isFolder") {
        list.retain(|it| it.kind.playable() != folder);
    }
    let played = |it: &Item| d.view.fully_watched(it);
    let favorite = |it: &Item| matches!(it.kind, Kind::Movie | Kind::Show) && d.view.on_watchlist(&it.id);
    let resumable = |it: &Item| {
        it.kind.playable() && {
            let p = d.view.progress(it.key());
            !p.watched && p.offset_ms > 0
        }
    };
    for f in &filters {
        match f.as_str() {
            "isfavorite" | "isfavoriteorlikes" | "likes" => list.retain(|it| favorite(it)),
            "isplayed" => list.retain(|it| played(it)),
            "isunplayed" => list.retain(|it| !played(it)),
            "isresumable" => list.retain(|it| resumable(it)),
            "isfolder" => list.retain(|it| !it.kind.playable()),
            "isnotfolder" => list.retain(|it| it.kind.playable()),
            _ => {}
        }
    }
    if let Some(p) = r.bool("isPlayed") {
        list.retain(|it| played(it) == p);
    }
    if let Some(f) = r.bool("isFavorite") {
        list.retain(|it| favorite(it) == f);
    }
    if let Some(q) = &search {
        list.retain(|it| it.title.to_lowercase().contains(q) || it.original_title.to_lowercase().contains(q) || it.sort_title.contains(q));
    }
    let mut genres: Vec<String> = r.list("genres").iter().flat_map(|g| g.split('|')).map(|g| g.trim().to_lowercase()).collect();
    let genre_ids = r.list("genreIds");
    if !genre_ids.is_empty() {
        // Genre GUIDs are hashes of the name: find the names among the genres there are.
        let names: BTreeSet<String> = idx.all().flat_map(|it| it.genre_list().map(str::to_lowercase).collect::<Vec<_>>()).collect();
        let wanted: Vec<String> = genre_ids.iter().map(|g| g.replace('-', "").to_lowercase()).collect();
        genres.extend(names.into_iter().filter(|n| wanted.contains(&genre_guid(n))));
    }
    if !genres.is_empty() {
        list.retain(|it| it.genre_list().any(|g| genres.contains(&g.to_lowercase())));
    }
    let years: Vec<i32> = r.list("years").iter().filter_map(|y| y.parse().ok()).collect();
    if !years.is_empty() {
        list.retain(|it| years.contains(&it.year));
    }
    let ratings = r.list("officialRatings");
    if !ratings.is_empty() {
        list.retain(|it| ratings.iter().any(|x| x.eq_ignore_ascii_case(&it.content_rating)));
    }
    if let Some(x) = r.param("nameStartsWith").map(|s| s.to_lowercase()).filter(|s| !s.is_empty()) {
        list.retain(|it| it.sort_title.starts_with(&x));
    }
    if let Some(x) = r.param("nameStartsWithOrGreater").map(|s| s.to_lowercase()).filter(|s| !s.is_empty()) {
        list.retain(|it| it.sort_title.as_str() >= x.as_str());
    }
    if let Some(x) = r.param("nameLessThan").map(|s| s.to_lowercase()).filter(|s| !s.is_empty()) {
        list.retain(|it| it.sort_title.as_str() < x.as_str());
    }
    let excluded = r.list("excludeItemIds");
    if !excluded.is_empty() {
        let out: Vec<String> = excluded
            .iter()
            .filter_map(|g| match parse_guid(g) {
                Some(Id::Item(i)) => Some(i),
                _ => None,
            })
            .collect();
        list.retain(|it| !out.contains(&it.id));
    }

    sort(&d, &mut list, &r.list_lower("sortBy"), &r.list_lower("sortOrder"));
    let total = folders.len() + list.len();
    let start = r.int("startIndex").unwrap_or(0).max(0) as usize;
    let limit = r.int("limit").filter(|l| *l > 0).map_or(usize::MAX, |l| l as usize);
    let all: Vec<Value> = folders.into_iter().chain(list.iter().map(|it| d.item(it, full))).skip(start).take(limit).collect();
    json_out(query_result(all, total, start))
}

/// Sorts by the apps' sort keys (`SortName`, `DateCreated`, `ProductionYear`, ...), each with
/// its own order; without keys the order stays as it is (library or episode order).
fn sort(d: &Dto, list: &mut [Arc<Item>], keys: &[String], orders: &[String]) {
    if keys.iter().any(|k| k == "random") {
        list.shuffle(&mut rand::thread_rng());
        return;
    }
    if keys.is_empty() {
        return;
    }
    let last_played = |it: &Item| -> i64 {
        if it.kind.playable() {
            d.view.progress(it.key()).updated_at
        } else {
            d.idx.children(&it.id).iter().map(|c| d.view.progress(&c.id).updated_at).max().unwrap_or(0)
        }
    };
    let index = |it: &Item| -> (i32, i32) {
        if it.kind == Kind::Episode { (d.idx.parent(it).map_or(0, |s| s.index), it.index) } else { (0, it.index) }
    };
    let cmp = |a: &Arc<Item>, b: &Arc<Item>, key: &str| -> Ordering {
        match key {
            "sortname" | "name" | "seriessortname" => a.sort_title.cmp(&b.sort_title),
            "datecreated" | "datelastcontentadded" => a.added_at.cmp(&b.added_at),
            "premieredate" | "productionyear" => a.year.cmp(&b.year),
            "communityrating" | "criticrating" => a.rating.partial_cmp(&b.rating).unwrap_or(Ordering::Equal),
            "runtime" => a.duration_ms().cmp(&b.duration_ms()),
            "dateplayed" | "datelastplayed" => last_played(a).cmp(&last_played(b)),
            "playcount" => d.view.progress(a.key()).view_count.cmp(&d.view.progress(b.key()).view_count),
            "indexnumber" | "parentindexnumber" | "airedepisodeorder" => index(a).cmp(&index(b)),
            "officialrating" => a.content_rating.cmp(&b.content_rating),
            _ => Ordering::Equal,
        }
    };
    list.sort_by(|a, b| {
        for (i, k) in keys.iter().enumerate() {
            let desc = orders.get(i).or(orders.first()).is_some_and(|o| o == "descending");
            let o = cmp(a, b, k);
            let o = if desc { o.reverse() } else { o };
            if o != Ordering::Equal {
                return o;
            }
        }
        Ordering::Equal
    });
}

fn page<T>(cx: &Cx, list: Vec<T>, default_limit: usize) -> (Vec<T>, usize, usize) {
    let total = list.len();
    let start = cx.req.int("startIndex").unwrap_or(0).max(0) as usize;
    let limit = cx.req.int("limit").filter(|l| *l > 0).map_or(default_limit, |l| l as usize);
    (list.into_iter().skip(start).take(limit).collect(), total, start)
}

/// `GET /Items/Latest`: newest movies and shows (a show counts with its newest episode), as a
/// plain array.
pub fn latest(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let r = &cx.req;
    let libraries: Vec<String> = match r.param("parentId").as_deref().and_then(parse_guid) {
        Some(Id::Library(l)) => vec![l],
        Some(_) => return json_out(json!([])),
        None => cx.app.db.libraries().into_iter().map(|l| l.id).collect(),
    };
    let types = r.list_lower("includeItemTypes");
    let group = r.bool("groupItems").unwrap_or(true);
    let mut list: Vec<(i64, Arc<Item>)> = Vec::new();
    for l in libraries.iter().filter(|l| u.may_access(l)) {
        for top in idx.top_level(l).iter().filter(|it| d.view.sees(it)) {
            match top.kind {
                Kind::Show if group && !types.iter().any(|t| t == "episode") || types.iter().any(|t| t == "series") => {
                    let newest = idx.episodes_of(top).iter().map(|e| e.added_at).max().unwrap_or(top.added_at);
                    list.push((newest, top.clone()));
                }
                Kind::Show => list.extend(idx.episodes_of(top).into_iter().map(|e| (e.added_at, e))),
                _ => list.push((top.added_at, top.clone())),
            }
        }
    }
    let kinds: Vec<Kind> = types.iter().filter_map(|t| kind_of(t)).collect();
    if !kinds.is_empty() {
        list.retain(|(_, it)| kinds.contains(&it.kind));
    }
    if let Some(p) = r.bool("isPlayed") {
        list.retain(|(_, it)| d.view.fully_watched(it) == p);
    }
    list.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    let limit = r.int("limit").filter(|l| *l > 0).unwrap_or(16) as usize;
    let out: Vec<Value> = list.iter().take(limit).map(|(_, it)| d.item(it, false)).collect();
    json_out(json!(out))
}

/// `GET /UserItems/Resume`: started movies and episodes, last played first.
pub fn resume(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let library = match cx.req.param("parentId").as_deref().and_then(parse_guid) {
        Some(Id::Library(l)) => Some(l),
        _ => None,
    };
    let mut list = d.view.started(library.as_deref());
    let kinds: Vec<Kind> = cx.req.list_lower("includeItemTypes").iter().filter_map(|t| kind_of(t)).collect();
    if !kinds.is_empty() {
        list.retain(|it| kinds.contains(&it.kind));
    }
    let (list, total, start) = page(cx, list, 50);
    json_out(query_result(list.iter().map(|it| d.item(it, false)).collect(), total, start))
}

/// `GET /Shows/NextUp`: the next episode of every show the user is watching, the show watched
/// most recently first; with `seriesId` only that show (its first episode if none is watched).
pub fn next_up(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let r = &cx.req;
    let only = match r.param("seriesId").as_deref().and_then(parse_guid) {
        Some(Id::Item(i)) => Some(i),
        Some(_) => return json_out(query_result(vec![], 0, 0)),
        None => None,
    };
    let library = match r.param("parentId").as_deref().and_then(parse_guid) {
        Some(Id::Library(l)) => Some(l),
        _ => None,
    };
    let mut list: Vec<(i64, Arc<Item>)> = Vec::new();
    for show in idx.all().filter(|it| it.kind == Kind::Show && d.view.sees(it)) {
        if only.as_ref().is_some_and(|o| *o != show.id) || library.as_ref().is_some_and(|l| *l != show.library_id) {
            continue;
        }
        let eps = idx.episodes_of(show);
        let last = eps.iter().map(|e| d.view.progress(&e.id).updated_at).max().unwrap_or(0);
        if last == 0 && only.is_none() {
            continue;
        }
        if let Some(next) = d.view.next_episode(show) {
            list.push((last, next));
        }
    }
    list.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    let (list, total, start) = page(cx, list, 50);
    let full = r.list_lower("fields").iter().any(|f| f == "mediasources");
    json_out(query_result(list.iter().map(|(_, it)| d.item(it, full)).collect(), total, start))
}

/// `GET /Shows/{id}/Seasons`.
pub fn seasons(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let show = visible(cx, &idx, raw)?;
    if show.kind != Kind::Show {
        return Err(not_found());
    }
    let list: Vec<Value> = idx.children(&show.id).iter().filter(|s| s.kind == Kind::Season).map(|s| d.item(s, false)).collect();
    let n = list.len();
    json_out(query_result(list, n, 0))
}

/// `GET /Shows/{id}/Episodes`: all episodes, or those of `seasonId` / `season`, from `startItemId` on.
pub fn episodes(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let r = &cx.req;
    let show = visible(cx, &idx, raw)?;
    if show.kind != Kind::Show {
        return Err(not_found());
    }
    let mut list: Vec<Arc<Item>> = match (r.param("seasonId").as_deref().and_then(parse_guid), r.int("season")) {
        (Some(Id::Item(s)), _) => idx.children(&s).to_vec(),
        (_, Some(n)) => {
            idx.children(&show.id).iter().find(|s| i64::from(s.index) == n).map(|s| idx.children(&s.id).to_vec()).unwrap_or_default()
        }
        _ => idx.episodes_of(&show),
    };
    if let Some(Id::Item(start)) = r.param("startItemId").as_deref().and_then(parse_guid)
        && let Some(i) = list.iter().position(|e| e.id == start)
    {
        list.drain(..i);
    }
    if let Some(p) = r.bool("isPlayed") {
        list.retain(|e| d.view.fully_watched(e) == p);
    }
    let full = r.list_lower("fields").iter().any(|f| f == "mediasources");
    let (list, total, start) = page(cx, list, usize::MAX);
    json_out(query_result(list.iter().map(|e| d.item(e, full)).collect(), total, start))
}

/// The genres of the movies and shows the user sees (of one library with `parentId`).
fn genre_names(cx: &Cx, d: &Dto) -> Vec<String> {
    let library = match cx.req.param("parentId").as_deref().and_then(parse_guid) {
        Some(Id::Library(l)) => Some(l),
        _ => None,
    };
    let kinds: Vec<Kind> = cx.req.list_lower("includeItemTypes").iter().filter_map(|t| kind_of(t)).collect();
    let mut names: HashMap<String, String> = HashMap::new();
    for it in d.idx.all().filter(|it| matches!(it.kind, Kind::Movie | Kind::Show)) {
        if library.as_ref().is_some_and(|l| *l != it.library_id) || (!kinds.is_empty() && !kinds.contains(&it.kind)) || !d.view.sees(it) {
            continue;
        }
        for g in it.genre_list() {
            names.entry(g.to_lowercase()).or_insert_with(|| g.to_string());
        }
    }
    let mut list: Vec<String> = names.into_values().collect();
    list.sort_by_key(|g| g.to_lowercase());
    list
}

/// `GET /Genres`.
pub fn genres(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let list: Vec<Value> = genre_names(cx, &d)
        .iter()
        .map(|g| {
            let id = genre_guid(g);
            json!({"Name": g, "ServerId": cx.jf().server_id, "Id": id, "Type": "Genre", "IsFolder": true, "SortName": g.to_lowercase(),
                   "ImageTags": {}, "BackdropImageTags": [], "LocationType": "Virtual", "MediaType": "Unknown",
                   "UserData": {"PlaybackPositionTicks": 0, "PlayCount": 0, "IsFavorite": false, "Played": false, "Key": id, "ItemId": id}})
        })
        .collect();
    let (list, total, start) = page(cx, list, usize::MAX);
    json_out(query_result(list, total, start))
}

pub fn filters_legacy(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let years: BTreeSet<i32> =
        idx.all().filter(|it| matches!(it.kind, Kind::Movie | Kind::Show) && it.year > 0 && d.view.sees(it)).map(|it| it.year).collect();
    json_out(json!({"Genres": genre_names(cx, &d), "Tags": [], "OfficialRatings": [], "Years": years}))
}

pub fn filters(cx: &Cx) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let genres: Vec<Value> = genre_names(cx, &d).iter().map(|g| json!({"Name": g, "Id": genre_guid(g)})).collect();
    json_out(json!({"Genres": genres, "Tags": []}))
}

/// `GET /Items/{id}/Similar`: entries of the same kind sharing the most genres, best rated first.
pub fn similar(cx: &Cx, raw: &str) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let it = visible(cx, &idx, raw)?;
    let mine: Vec<String> = it.genre_list().map(str::to_lowercase).collect();
    let mut list: Vec<(usize, &Arc<Item>)> = idx
        .all()
        .filter(|o| o.kind == it.kind && o.id != it.id && matches!(o.kind, Kind::Movie | Kind::Show) && d.view.sees(o))
        .map(|o| (o.genre_list().filter(|g| mine.contains(&g.to_lowercase())).count(), o))
        .filter(|(n, _)| *n > 0)
        .collect();
    list.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.rating.partial_cmp(&a.1.rating).unwrap_or(Ordering::Equal)));
    let limit = cx.req.int("limit").filter(|l| *l > 0).unwrap_or(12) as usize;
    let out: Vec<Value> = list.iter().take(limit).map(|(_, o)| d.item(o, false)).collect();
    let n = out.len();
    json_out(query_result(out, n, 0))
}

/// Trailers (`trailers`) or the other bonus videos of a movie or show, as a plain array.
pub fn extras(cx: &Cx, raw: &str, trailers: bool) -> Out {
    let u = cx.user()?;
    let idx = cx.app.catalog.snapshot();
    let d = Dto::new(cx, &idx, u);
    let it = visible(cx, &idx, raw)?;
    let out: Vec<Value> = idx.extras(&it.id).iter().filter(|e| (e.extra_type == "trailer") == trailers).map(|e| d.item(e, false)).collect();
    json_out(json!(out))
}

pub fn no_theme_media() -> Value {
    let empty = json!({"Items": [], "TotalRecordCount": 0, "StartIndex": 0});
    json!({"ThemeVideosResult": empty, "ThemeSongsResult": empty, "SoundtrackSongsResult": empty})
}

/// `GET /MediaSegments/{id}`: the detected intro and end credits, for "Skip intro" in the apps.
pub fn segments(cx: &Cx, raw: &str) -> Out {
    let idx = cx.app.catalog.snapshot();
    let it = visible(cx, &idx, raw)?;
    let mut list = Vec::new();
    if let Some(m) = cx.app.markers.as_ref().and_then(|s| s.get(&it)) {
        let item_id = item_guid(&it.id);
        for (kind, seg) in [("Intro", m.intro), ("Outro", m.credits)] {
            if let Some(s) = seg {
                let id = format!("f4ee0005{}", &crate::auth::sha256(&format!("{}\0{kind}", it.id))[..24]);
                list.push(json!({"Id": id, "ItemId": item_id, "Type": kind, "StartTicks": ticks(s.start_ms), "EndTicks": ticks(s.end_ms)}));
            }
        }
    }
    let n = list.len();
    json_out(query_result(list, n, 0))
}

/// `GET /Items/{id}/Images/{type}`: artwork. The apps load images without signing in, so the
/// request needs either a sign-in or the signed tag the entry came with.
pub async fn image(cx: &Cx, raw: &str, kind: &str) -> Out {
    let idx = cx.app.catalog.snapshot();
    let Some(Id::Item(id)) = parse_guid(raw) else { return Err(not_found()) };
    let it = idx.item(&id).cloned().ok_or_else(not_found)?;
    let tag_kind = kind.to_ascii_lowercase();
    let source = match tag_kind.as_str() {
        "primary" if matches!(it.kind, Kind::Episode | Kind::Extra) => "thumb",
        "primary" => "poster",
        "backdrop" => "backdrop",
        "thumb" => "thumb",
        _ => return Err(not_found()),
    };
    let allowed = match &cx.ctx.user {
        Some(u) => may_see(&idx, u, &it),
        None => cx.req.param("tag").is_some_and(|t| t == image_tag(cx.jf(), &idx, &it, &tag_kind)),
    };
    if !allowed {
        return Err(StatusCode::UNAUTHORIZED.into());
    }
    drop(idx);
    let path = image_path(&cx.app, &it, source).await.ok_or_else(not_found)?;
    crate::files::send(&path, IMAGE_CACHE_SECONDS, &cx.req.headers).await.map_err(super::err)
}
