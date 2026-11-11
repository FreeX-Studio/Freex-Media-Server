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

//! Library filters: which movies or shows of a library to list. The same filter, written as a
//! query string, is what a smart collection stores, so it is evaluated anew for every viewer
//! (with their own watch state) and every change of the library.

use super::media::View;
use crate::AppState;
use crate::media::catalog::Index;
use crate::media::{Item, Kind, MediaFile};
use crate::metadata::matching::normalize;
use std::collections::HashMap;

/// Query parameters a filter understands, in the order they are written back.
const KEYS: [&str; 10] = ["genre", "watched", "yearFrom", "yearTo", "minRating", "resolution", "person", "contentRating", "addedDays", "q"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub genre: String,
    /// `unwatched`, `watched` or `inProgress`.
    pub watched: String,
    pub year_from: i32,
    pub year_to: i32,
    pub min_rating: f64,
    /// `4K`, `1080p`, `720p`, `576p` or `SD` (as the file's resolution label).
    pub resolution: String,
    /// Part of the name of an actor or director.
    pub person: String,
    /// Age rating from the metadata (`FSK 12`, `PG-13`), exactly.
    pub content_rating: String,
    /// Added within this many days.
    pub added_days: i64,
    /// Part of the title.
    pub text: String,
}

impl Filter {
    pub fn from_params(q: &HashMap<String, String>) -> Filter {
        let s = |k: &str| q.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
        let mut f = Filter {
            genre: s("genre"),
            watched: s("watched"),
            year_from: s("yearFrom").parse().unwrap_or(0),
            year_to: s("yearTo").parse().unwrap_or(0),
            min_rating: s("minRating").parse::<f64>().ok().filter(|r| r.is_finite()).unwrap_or(0.0).clamp(0.0, 10.0),
            resolution: s("resolution"),
            person: s("person"),
            content_rating: s("contentRating"),
            added_days: s("addedDays").parse::<i64>().unwrap_or(0).max(0),
            text: s("q"),
        };
        // The older switch of FreeX Web.
        if f.watched.is_empty() && s("unwatched") == "1" {
            f.watched = "unwatched".into();
        }
        if !["unwatched", "watched", "inProgress"].contains(&f.watched.as_str()) {
            f.watched.clear();
        }
        f
    }

    /// Parses a stored query string (`genre=Drama&watched=unwatched`).
    pub fn parse(query: &str) -> Filter {
        let q: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
        Filter::from_params(&q)
    }

    /// The filter as a query string with only the set keys, in a fixed order.
    pub fn to_query(&self) -> String {
        let mut out = url::form_urlencoded::Serializer::new(String::new());
        for k in KEYS {
            let v = match k {
                "genre" => self.genre.clone(),
                "watched" => self.watched.clone(),
                "yearFrom" if self.year_from > 0 => self.year_from.to_string(),
                "yearTo" if self.year_to > 0 => self.year_to.to_string(),
                "minRating" if self.min_rating > 0.0 => self.min_rating.to_string(),
                "resolution" => self.resolution.clone(),
                "person" => self.person.clone(),
                "contentRating" => self.content_rating.clone(),
                "addedDays" if self.added_days > 0 => self.added_days.to_string(),
                "q" => self.text.clone(),
                _ => String::new(),
            };
            if !v.is_empty() {
                out.append_pair(k, &v);
            }
        }
        out.finish()
    }

    /// Whether a movie or show passes the filter for the viewer of `view`.
    pub fn matches(&self, app: &AppState, idx: &Index, view: &View, it: &Item, now_ms: i64) -> bool {
        if !self.genre.is_empty() && !it.has_genre(&self.genre) {
            return false;
        }
        if self.year_from > 0 && it.year < self.year_from || self.year_to > 0 && (it.year == 0 || it.year > self.year_to) {
            return false;
        }
        if self.min_rating > 0.0 && it.rating < self.min_rating {
            return false;
        }
        if self.added_days > 0 && it.added_at < now_ms - self.added_days * 86_400_000 {
            return false;
        }
        if !self.text.is_empty() {
            let t = normalize(&self.text);
            if !normalize(&it.title).contains(&t) && !normalize(&it.original_title).contains(&t) {
                return false;
            }
        }
        match self.watched.as_str() {
            "unwatched" if view.fully_watched(it) => return false,
            "watched" if !view.fully_watched(it) => return false,
            "inProgress" if view.fully_watched(it) || !in_progress(view, it) => return false,
            _ => {}
        }
        if !self.resolution.is_empty() && !files(idx, it).iter().any(|f| f.resolution_label() == self.resolution) {
            return false;
        }
        if !self.person.is_empty() || !self.content_rating.is_empty() {
            let Some(md) = app.metadata.get(&it.id) else { return false };
            if !self.content_rating.is_empty() && md.content_rating != self.content_rating {
                return false;
            }
            if !self.person.is_empty() {
                let p = normalize(&self.person);
                let hit = md
                    .cast
                    .iter()
                    .map(|c| c.name.as_str())
                    .chain(md.directors.iter().map(String::as_str))
                    .any(|n| normalize(n).contains(&p));
                if !hit {
                    return false;
                }
            }
        }
        true
    }
}

/// Started but not finished: a movie with a resume position, a show with some episodes watched
/// or one begun.
fn in_progress(view: &View, it: &Item) -> bool {
    if it.kind.playable() { view.progress(it.key()).offset_ms > 0 } else { view.any_watched(it) }
}

/// The files of a movie (all versions) or of the episodes of a show.
pub fn files<'a>(idx: &'a Index, it: &'a Item) -> Vec<&'a MediaFile> {
    match it.kind {
        Kind::Show => idx.children(&it.id).iter().flat_map(|s| idx.children(&s.id)).filter_map(|e| e.file.as_ref()).collect(),
        _ if !it.versions.is_empty() => it.versions.iter().map(|v| &v.file).collect(),
        _ => it.file.iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_round_trip() {
        let f = Filter::parse("watched=unwatched&genre=Science+Fiction&yearFrom=1990&yearTo=1999&minRating=7.5&person=Keanu&bogus=1");
        assert_eq!(
            (f.genre.as_str(), f.watched.as_str(), f.year_from, f.year_to, f.min_rating),
            ("Science Fiction", "unwatched", 1990, 1999, 7.5)
        );
        assert_eq!(f.to_query(), "genre=Science+Fiction&watched=unwatched&yearFrom=1990&yearTo=1999&minRating=7.5&person=Keanu");
        assert_eq!(Filter::parse(&f.to_query()), f);
        assert_eq!(Filter::parse("unwatched=1").watched, "unwatched");
        assert_eq!(Filter::parse("watched=maybe").watched, "");
        assert_eq!(Filter::parse(""), Filter::default());
    }
}
