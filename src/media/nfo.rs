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

//! Kodi-style `.nfo` files (`movie.nfo`, `tvshow.nfo`, `<video>.nfo`). Only the common elements
//! are read, with simple patterns instead of a full XML parser: the files in the wild are often
//! not well-formed.

use super::Item;
use super::names::sort_title;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

const MAX_SIZE: u64 = 2 * 1024 * 1024;

const ELEMENTS: [&str; 15] = [
    "tmdbid",
    "imdbid",
    "tvdbid",
    "id",
    "title",
    "sorttitle",
    "originaltitle",
    "plot",
    "outline",
    "tagline",
    "year",
    "premiered",
    "genre",
    "value",
    "rating",
];

static PATTERNS: LazyLock<HashMap<&'static str, Regex>> = LazyLock::new(|| {
    ELEMENTS.iter().map(|n| (*n, Regex::new(&format!(r"(?is)<{n}(?:\s[^>]*)?>(.*?)</{n}>")).expect("valid pattern"))).collect()
});
static UNIQUE_TMDB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<uniqueid[^>]*type="tmdb"[^>]*>(\d+)</uniqueid>"#).expect("valid pattern"));
static UNIQUE_IMDB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<uniqueid[^>]*type="imdb"[^>]*>(tt\d{7,9})</uniqueid>"#).expect("valid pattern"));
static UNIQUE_TVDB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<uniqueid[^>]*type="tvdb"[^>]*>(\d+)</uniqueid>"#).expect("valid pattern"));
static IMDB_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^tt\d{7,9}$").expect("valid pattern"));
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").expect("valid pattern"));

/// Applies the values found in the file to the item; missing elements leave fields unchanged.
pub fn apply(nfo: &Path, item: &mut Item) {
    if std::fs::metadata(nfo).map(|m| m.len() > MAX_SIZE).unwrap_or(true) {
        return;
    }
    let Ok(bytes) = std::fs::read(nfo) else { return };
    // UTF-8, or Latin-1 for old files.
    let xml = String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
    apply_text(&xml, item);
}

pub fn apply_text(xml: &str, item: &mut Item) {
    if !xml.contains('<') {
        return;
    }
    item.has_nfo = true;
    let mut tmdb = element(xml, "tmdbid");
    if tmdb.is_empty() {
        tmdb = UNIQUE_TMDB.captures(xml).map(|c| c[1].to_string()).unwrap_or_default();
    }
    if let Ok(id) = tmdb.trim().parse() {
        item.tmdb_hint = id;
    }
    let mut imdb = element(xml, "imdbid");
    if imdb.is_empty() {
        imdb = UNIQUE_IMDB.captures(xml).map(|c| c[1].to_string()).unwrap_or_default();
    }
    if imdb.is_empty() {
        imdb = element(xml, "id"); // old Kodi files keep the IMDb id in <id>
    }
    if IMDB_ID.is_match(imdb.trim()) {
        item.imdb_hint = imdb.trim().to_string();
    }
    let mut tvdb = element(xml, "tvdbid");
    if tvdb.is_empty() {
        tvdb = UNIQUE_TVDB.captures(xml).map(|c| c[1].to_string()).unwrap_or_default();
    }
    if let Ok(id) = tvdb.trim().parse::<i64>() {
        item.tvdb_hint = id;
    }
    let title = element(xml, "title");
    if !title.is_empty() {
        item.sort_title = sort_title(&title);
        item.title = title;
    }
    let sort = element(xml, "sorttitle");
    if !sort.is_empty() {
        item.sort_title = sort.to_lowercase();
    }
    let original = element(xml, "originaltitle");
    if !original.is_empty() && original != item.title {
        item.original_title = original;
    }
    let mut plot = element(xml, "plot");
    if plot.is_empty() {
        plot = element(xml, "outline");
    }
    if !plot.is_empty() {
        item.summary = plot;
    }
    let tagline = element(xml, "tagline");
    if !tagline.is_empty() {
        item.tagline = tagline;
    }
    let mut year = element(xml, "year");
    if year.is_empty() {
        year = element(xml, "premiered").chars().take(4).collect();
    }
    if let Ok(y) = year.trim().parse() {
        item.year = y;
    }
    let genres = elements(xml, "genre");
    if !genres.is_empty() {
        item.genres = genres.join(", ");
    }
    // New Kodi format: <ratings><rating name=".."><value>7.8</value>..., old format: <rating>7.8</rating>
    let mut rating = element(xml, "value");
    if rating.is_empty() {
        rating = element(xml, "rating");
    }
    if let Ok(r) = rating.replace(',', ".").trim().parse::<f64>() {
        item.rating = (r * 10.0).round() / 10.0;
    }
}

fn element(xml: &str, name: &str) -> String {
    PATTERNS[name].captures(xml).map(|c| decode(&c[1])).unwrap_or_default()
}

fn elements(xml: &str, name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in PATTERNS[name].captures_iter(xml) {
        let v = decode(&c[1]);
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

fn decode(s: &str) -> String {
    let mut r = s.trim();
    if let Some(inner) = r.strip_prefix("<![CDATA[").and_then(|x| x.strip_suffix("]]>")) {
        r = inner;
    }
    TAGS.replace_all(r, "")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Kind;

    #[test]
    fn reads_common_elements() {
        let xml = r#"<?xml version="1.0"?><movie><title>Inception</title><originaltitle>Inception</originaltitle>
            <plot><![CDATA[A thief &amp; dreams.]]></plot><year>2010</year><genre>Action</genre><genre>Sci-Fi</genre>
            <ratings><rating name="imdb"><value>8,84</value></rating></ratings>
            <uniqueid type="tmdb">27205</uniqueid><uniqueid type="imdb">tt1375666</uniqueid></movie>"#;
        let mut it = Item::new("x".into(), "l", Kind::Movie);
        apply_text(xml, &mut it);
        assert!(it.has_nfo);
        assert_eq!(it.title, "Inception");
        assert_eq!(it.original_title, "");
        assert_eq!(it.summary, "A thief & dreams.");
        assert_eq!(it.year, 2010);
        assert_eq!(it.genres, "Action, Sci-Fi");
        assert_eq!(it.rating, 8.8);
        assert_eq!(it.tmdb_hint, 27205);
        assert_eq!(it.imdb_hint, "tt1375666");
    }
}
