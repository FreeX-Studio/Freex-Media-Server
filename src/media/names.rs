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

//! Titles, years and episode numbers from file and folder names, for clean names
//! (`The Matrix (1999)`, `Season 2/03 - Title.mkv`) as well as scene release names
//! (`The.Matrix.1999.1080p.BluRay.x264`, `Show.S01E02.German.720p`).

use fancy_regex::{Captures, Regex};
use std::sync::LazyLock;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("valid pattern")
}

static YEAR: LazyLock<Regex> = LazyLock::new(|| re(r"(?:^|[\s(\[._-])((?:19|20)\d{2})(?=$|[\s)\]._-])"));
/// Tokens that start the "technical" part of a scene release name.
static RELEASE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"(?i)(?:^|[\s._\[(-])(2160p|1080[pi]|720p|576p|480p|4k|uhd|hdr10?\+?|dv|bluray|blu-ray|bdrip|brrip|",
        r"web-?dl|web-?rip|webrip|hdtv|dvdrip|dvd|remux|x264|x265|h\.?264|h\.?265|hevc|avc|xvid|divx|",
        r"aac|ac3|dts(?:-hd)?|truehd|atmos|ddp?5\.1|10bit|proper|repack|extended|unrated|",
        r"german|english|multi|dl|ml|subbed|dubbed)(?=$|[\s._\])-])"
    ))
});
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| re(r"\[[^\]]*\]|\{[^}]*\}"));
static BRACKET_YEAR: LazyLock<Regex> = LazyLock::new(|| re(r"^\[(?:19|20)\d{2}\]$"));
static SXXEYY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)s(\d{1,2})[\s._-]*e(\d{1,3})(?:[\s._-]*-?[\s._-]*e(\d{1,3}))?"));
static NXNN: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(?:^|[\s._\[(-])(\d{1,2})x(\d{2,3})(?=$|[\s._\])-])"));
static SEASON_FOLDER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(?:season|staffel|series|serie|saison|s)[\s._-]*(\d{1,3})$"));
static LEADING_NUMBER: LazyLock<Regex> = LazyLock::new(|| re(r"^(\d{1,3})(?=[\s._-]|$)"));
static TMDB_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)[\[{(]\s*tmdb(?:id)?\s*[-=:]\s*(\d+)\s*[\]})]"));
/// Id tags of other databases (`{imdb-tt0133093}`, `[tvdbid=121361]`), removed from titles.
static OTHER_ID_TAG: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)[\[{(]\s*(?:imdb|tvdb)(?:id)?\s*[-=:]\s*\w+\s*[\]})]"));
static EMPTY_PARENS: LazyLock<Regex> = LazyLock::new(|| re(r"\(\s*\)"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
static EDGE_START: LazyLock<Regex> = LazyLock::new(|| re(r"^[\s\-–._:(\[]+"));
static EDGE_END: LazyLock<Regex> = LazyLock::new(|| re(r"[\s\-–._:(\[]+$"));

/// Title and year (0 if none).
#[derive(Debug, Clone, PartialEq)]
pub struct TitleYear {
    pub title: String,
    pub year: i32,
}

/// Season and episode numbers (-1 if unknown), the show part before `SxxEyy` and the episode title.
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeInfo {
    pub season: i32,
    pub episode: i32,
    pub episode_end: i32,
    pub show_part: String,
    pub title: String,
}

fn captures<'t>(r: &Regex, s: &'t str) -> Option<Captures<'t>> {
    r.captures(s).ok().flatten()
}

fn num(c: &Captures, i: usize) -> i32 {
    c.get(i).and_then(|m| m.as_str().parse().ok()).unwrap_or(-1)
}

/// TMDB id written into a name, as other media servers support it: `Movie (2010) {tmdb-27205}`.
pub fn tmdb_tag(name: &str) -> i64 {
    captures(&TMDB_TAG, name).and_then(|c| c.get(1)?.as_str().parse().ok()).unwrap_or(0)
}

pub fn strip_extension(file_name: &str) -> &str {
    match file_name.rfind('.') {
        Some(dot) if dot > 0 => &file_name[..dot],
        _ => file_name,
    }
}

pub fn extension(file_name: &str) -> String {
    match file_name.rfind('.') {
        Some(dot) if dot > 0 => file_name[dot + 1..].to_lowercase(),
        _ => String::new(),
    }
}

/// Parses a movie or show name (`The Matrix (1999)`, `The.Matrix.1999.1080p`).
pub fn title_year(name: &str) -> TitleYear {
    // Drop bracketed tags, but keep a year written as "[2010]".
    let s = TMDB_TAG.replace_all(name, " ");
    let s = OTHER_ID_TAG.replace_all(&s, " ");
    let s = BRACKETS.replace_all(&s, |c: &Captures| {
        let g = &c[0];
        if BRACKET_YEAR.is_match(g).unwrap_or(false) { format!(" {} ", &g[1..5]) } else { " ".to_string() }
    });
    let s = normalize_separators(&s);
    let mut year = 0;
    let mut cut: Option<usize> = None;
    for c in YEAR.captures_iter(&s).flatten() {
        let m = c.get(1).expect("group");
        // A year at the very start is part of the title ("2001 A Space Odyssey"), keep looking.
        if m.start() == 0 {
            continue;
        }
        year = m.as_str().parse().unwrap_or(0);
        cut = Some(m.start());
    }
    if let Some(t) = RELEASE_TAG.find(&s).ok().flatten()
        && t.start() > 0
        && cut.is_none_or(|c| t.start() < c)
    {
        cut = Some(t.start());
    }
    let mut title = cleanup(match cut {
        Some(c) if c > 0 => &s[..c],
        _ => &s,
    });
    if title.is_empty() {
        title = cleanup(&s);
    }
    TitleYear { title, year }
}

static EDITION: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\{\s*edition\s*[-=:]\s*([^}]+)\}"));

/// What sets one version of a movie apart, from its file name: the edition tag
/// (`{edition-Director's Cut}`) or what follows the year (`Inception (2010) - 4K` → `4K`).
/// Empty if the name says nothing.
pub fn version_label(base: &str) -> String {
    if let Some(c) = EDITION.captures(base).ok().flatten() {
        return cleanup(&c[1]);
    }
    let s = TMDB_TAG.replace_all(base, " ");
    let s = OTHER_ID_TAG.replace_all(&s, " ");
    let s = normalize_separators(&s);
    let Some(m) = YEAR.captures_iter(&s).flatten().filter_map(|c| c.get(1)).filter(|m| m.start() > 0).last() else {
        return String::new();
    };
    let rest = s[m.end()..].trim_start_matches([')', ']']);
    let rest = cleanup(&BRACKETS.replace_all(rest, " ").replace(['(', ')', '[', ']'], " "));
    let rest = rest.trim_matches(|c: char| c == '-' || c == '–' || c == '.' || c.is_whitespace());
    rest.chars().take(40).collect::<String>().trim().to_string()
}

/// Season number from a folder name, or -1 when the folder is not a season folder.
pub fn season_from_folder(folder: &str) -> i32 {
    let f = folder.trim();
    if ["specials", "special", "extras"].iter().any(|s| f.eq_ignore_ascii_case(s)) {
        return 0;
    }
    captures(&SEASON_FOLDER, f).map_or(-1, |c| num(&c, 1))
}

/// Season and episode from an episode file name. `folder_season` is used when the name only has
/// an episode number (`03 - Title.mkv` inside `Season 2`).
pub fn episode(file_name: &str, folder_season: i32) -> EpisodeInfo {
    let base = strip_extension(file_name);
    if let Some(c) = captures(&SXXEYY, base) {
        let m = c.get(0).expect("match");
        return EpisodeInfo {
            season: num(&c, 1),
            episode: num(&c, 2),
            episode_end: num(&c, 3),
            show_part: cleanup(&normalize_separators(&base[..m.start()])),
            title: episode_title(&base[m.end()..]),
        };
    }
    if let Some(c) = captures(&NXNN, base) {
        let m = c.get(0).expect("match");
        return EpisodeInfo {
            season: num(&c, 1),
            episode: num(&c, 2),
            episode_end: -1,
            show_part: cleanup(&normalize_separators(&base[..m.start()])),
            title: episode_title(&base[m.end()..]),
        };
    }
    if folder_season >= 0 {
        let norm = base.trim();
        if let Some(c) = captures(&LEADING_NUMBER, norm) {
            let end = c.get(0).expect("match").end();
            return EpisodeInfo {
                season: folder_season,
                episode: num(&c, 1),
                episode_end: -1,
                show_part: String::new(),
                title: episode_title(&norm[end..]),
            };
        }
    }
    EpisodeInfo { season: folder_season, episode: -1, episode_end: -1, show_part: String::new(), title: episode_title(base) }
}

fn episode_title(rest: &str) -> String {
    let s = normalize_separators(&BRACKETS.replace_all(rest, " "));
    match RELEASE_TAG.find(&s).ok().flatten() {
        Some(t) => cleanup(&s[..t.start()]),
        None => cleanup(&s),
    }
}

/// Scene names use dots or underscores instead of spaces; turn them into spaces.
fn normalize_separators(s: &str) -> String {
    let r = s.replace('_', " ");
    // Only treat dots as separators if the name has (almost) no real spaces, so "Mr. Robot" stays intact.
    let spaces = r.matches(' ').count();
    let dots = r.matches('.').count();
    if dots >= 2 && dots > spaces { SPACES.replace_all(&r.replace('.', " "), " ").into_owned() } else { r }
}

fn cleanup(s: &str) -> String {
    let r = EMPTY_PARENS.replace_all(s, " ");
    let r = SPACES.replace_all(&r, " ");
    let r = r.trim();
    // Trim separators and stray brackets left over at the edges.
    let r = EDGE_START.replace_all(r, "");
    let r = EDGE_END.replace_all(&r, "");
    r.trim().to_string()
}

/// Sort key: lower case without a leading article (English and German).
pub fn sort_title(title: &str) -> String {
    let t = title.trim();
    let lower = t.to_lowercase();
    for article in ["the ", "a ", "an ", "der ", "die ", "das "] {
        if lower.starts_with(article) && lower.len() > article.len() {
            return lower[article.len()..].trim().to_string();
        }
    }
    lower
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ty(s: &str) -> (String, i32) {
        let t = title_year(s);
        (t.title, t.year)
    }

    #[test]
    fn version_labels() {
        assert_eq!(version_label("Inception (2010) - 4K"), "4K");
        assert_eq!(version_label("Blade Runner (1982) {edition-Final Cut}"), "Final Cut");
        assert_eq!(version_label("Inception (2010)"), "");
        assert_eq!(version_label("Inception.2010.1080p.BluRay"), "1080p BluRay");
        assert_eq!(version_label("Inception"), "");
    }

    #[test]
    fn movie_names() {
        assert_eq!(ty("The Matrix (1999)"), ("The Matrix".into(), 1999));
        assert_eq!(ty("The.Matrix.1999.1080p.BluRay.x264-GROUP"), ("The Matrix".into(), 1999));
        assert_eq!(ty("2001 A Space Odyssey (1968)"), ("2001 A Space Odyssey".into(), 1968));
        assert_eq!(ty("Inception [2010]"), ("Inception".into(), 2010));
        assert_eq!(ty("Inception (2010) {tmdb-27205}"), ("Inception".into(), 2010));
        assert_eq!(ty("The Matrix (1999) (imdb-tt0133093)"), ("The Matrix".into(), 1999));
        assert_eq!(ty("Mr. Robot"), ("Mr. Robot".into(), 0));
        assert_eq!(ty("Blade.Runner.2049.2017.German.DL.2160p"), ("Blade Runner 2049".into(), 2017));
        assert_eq!(tmdb_tag("Inception (2010) {tmdb-27205}"), 27205);
        assert_eq!(tmdb_tag("Inception [tmdbid=27205]"), 27205);
    }

    #[test]
    fn episode_names() {
        let e = episode("Breaking.Bad.S01E02.Cat's.in.the.Bag.720p.mkv", -1);
        assert_eq!((e.season, e.episode, e.show_part.as_str(), e.title.as_str()), (1, 2, "Breaking Bad", "Cat's in the Bag"));
        let e = episode("Show - s02e05-e06 - Double.mkv", -1);
        assert_eq!((e.season, e.episode, e.episode_end), (2, 5, 6));
        let e = episode("Show 3x07 Title.mkv", -1);
        assert_eq!((e.season, e.episode, e.title.as_str()), (3, 7, "Title"));
        let e = episode("03 - Der Anfang.mkv", 2);
        assert_eq!((e.season, e.episode, e.title.as_str()), (2, 3, "Der Anfang"));
        let e = episode("Pilot.mkv", -1);
        assert_eq!((e.season, e.episode, e.title.as_str()), (-1, -1, "Pilot"));
    }

    #[test]
    fn folders_and_sorting() {
        assert_eq!(season_from_folder("Season 01"), 1);
        assert_eq!(season_from_folder("Staffel 3"), 3);
        assert_eq!(season_from_folder("S2"), 2);
        assert_eq!(season_from_folder("Specials"), 0);
        assert_eq!(season_from_folder("Bonus"), -1);
        assert_eq!(sort_title("The Matrix"), "matrix");
        assert_eq!(sort_title("Die Hard"), "hard");
        assert_eq!(sort_title("A"), "a");
        assert_eq!(strip_extension("a.b.mkv"), "a.b");
        assert_eq!(extension("Movie.MKV"), "mkv");
    }
}
