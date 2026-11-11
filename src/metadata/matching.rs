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

//! Choosing the right search result: query variants for a title, title similarity and a score
//! for each candidate. Kept free of network access, so it can be tested in isolation.

use unicode_normalization::UnicodeNormalization;

/// A search result as far as matching is concerned.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub id: i64,
    pub title: String,
    pub original_title: String,
    pub year: i32,
    pub popularity: f64,
    pub vote_count: i64,
}

/// Lower case ASCII words: accents removed, `ß` → `ss`, `&` → `and`, punctuation to spaces,
/// without a leading article.
pub fn normalize(s: &str) -> String {
    let lower = s.to_lowercase().replace('ß', "ss").replace('&', " and ");
    let plain: String = lower.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).collect();
    let words: Vec<&str> = plain.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let skip = usize::from(words.len() > 1 && ["the", "a", "an", "der", "die", "das"].contains(&words[0]));
    words[skip..].join(" ")
}

/// Similarity of two normalized titles between 0 and 1 (edit distance on characters, with a
/// bonus when all words of the shorter title appear in the longer one).
pub fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let dist = levenshtein(&ca, &cb);
    let by_chars = 1.0 - dist as f64 / ca.len().max(cb.len()) as f64;
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let long_words: Vec<&str> = long.split(' ').collect();
    let contained = short.split(' ').all(|w| long_words.contains(&w));
    if contained {
        // "matrix" in "matrix reloaded" is related, but less than an exact match.
        let ratio = short.len() as f64 / long.len() as f64;
        by_chars.max(0.55 + 0.4 * ratio)
    } else {
        by_chars
    }
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Search texts to try for a title, best first, without duplicates.
pub fn query_variants(title: &str, original_title: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        let s = s.trim().to_string();
        if s.chars().count() >= 1 && !out.iter().any(|o| normalize(o) == normalize(&s)) {
            out.push(s);
        }
    };
    push(title.to_string());
    if !original_title.is_empty() {
        push(original_title.to_string());
    }
    // Names cleaned of what is not part of the title, searched early.
    let cleaned = cleaned(title);
    if cleaned != title {
        push(cleaned.clone());
    }
    // Without a subtitle: "Mission: Impossible – Dead Reckoning" → "Mission: Impossible".
    for sep in [" - ", " – ", ": "] {
        if let Some((head, _)) = title.split_once(sep)
            && head.chars().count() >= 3
        {
            push(head.to_string());
        }
    }
    // File names often spell umlauts out: "Die Schoene und das Biest".
    if let Some(u) = with_umlauts(title) {
        push(u);
    }
    if let Some(u) = with_umlauts(&cleaned).filter(|_| cleaned != title) {
        push(u);
    }
    // "&" and "and"/"und" are written either way.
    if title.contains('&') {
        push(title.replace('&', "and"));
    }
    out
}

/// A title without the parts that are rarely part of the real title: a numbering in front
/// ("2 - Pippi geht von Bord"), a scene group in front of a lower-case release name
/// ("7sins-jackass 35") and a studio in front ("Marvels What If", "Disneys Aladdin").
pub fn cleaned(title: &str) -> String {
    // Dots and underscores as word separators ("Ex.Machina-1080"), parts in brackets
    // ("Krieg der Sterne (Despecialized Edition 2.7)").
    let mut spaced = String::new();
    let mut depth = 0usize;
    for c in without_numbering(title.trim()).chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            '.' | '_' if depth == 0 => spaced.push(' '),
            _ if depth == 0 => spaced.push(c),
            _ => {}
        }
    }
    let spaced = without_extras(&spaced.split_whitespace().collect::<Vec<_>>().join(" "));
    let mut t = spaced.trim();
    // "group-title …" in all lower case, as some scene releases name their files.
    if t.chars().all(|c| !c.is_uppercase())
        && let Some((group, rest)) = t.split_once('-')
        && !group.is_empty()
        && !group.contains(' ')
        && group.len() <= 12
        && rest.chars().next().is_some_and(char::is_alphanumeric)
        && rest.len() >= 3
    {
        t = rest;
    }
    // A studio in front of the title.
    let lower = t.to_lowercase();
    for studio in ["marvel's ", "marvels ", "marvel ", "dc's ", "dcs ", "disney's ", "disneys ", "disney ", "pixar's ", "pixars "] {
        if lower.starts_with(studio) && t.len() > studio.len() + 1 {
            t = t[studio.len()..].trim_start();
            break;
        }
    }
    t.to_string()
}

/// Without a numbering in front: "2 - Title", "02. Title"; not "300" or "12 Monkeys".
fn without_numbering(t: &str) -> &str {
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        let rest = t[digits..].trim_start();
        if let Some(r) = rest.strip_prefix(['-', '–', '.', ')']) {
            let r = r.trim_start();
            if r.chars().any(char::is_alphabetic) && r.len() >= 3 {
                return r;
            }
        }
    }
    t
}

/// Words at the end that name a version or the quality, not the title: "Saw I DC",
/// "Hitler … - part 1", "Ex Machina-1080", "bfg webhd".
fn without_extras(s: &str) -> String {
    const EXTRAS: [&str; 22] = [
        "dc",
        "directors cut",
        "director's cut",
        "uncut",
        "extended",
        "extended cut",
        "extended edition",
        "remastered",
        "unrated",
        "special edition",
        "kinofassung",
        "despecialized",
        "edition",
        "webhd",
        "web",
        "hd",
        "sd",
        "2160",
        "1080",
        "720",
        "576",
        "480",
    ];
    let mut t = s.trim().trim_end_matches(['-', '–', ' ']).to_string();
    loop {
        let lower = t.to_lowercase();
        let before = t.len();
        // "part 1", "teil 2", "cd1"
        let words: Vec<&str> = t.split([' ', '-']).filter(|w| !w.is_empty()).collect();
        if words.len() >= 3 {
            let (a, b) = (words[words.len() - 2].to_lowercase(), words[words.len() - 1]);
            if ["part", "teil", "pt", "cd", "disc"].contains(&a.as_str()) && b.chars().all(|c| c.is_ascii_digit()) {
                let cut = lower.rfind(&a).unwrap_or(t.len());
                t.truncate(cut);
            }
        }
        for e in EXTRAS {
            for p in [e.to_string(), format!("{e}p")] {
                if lower.len() > p.len() + 2 && lower.ends_with(&p) && lower[..lower.len() - p.len()].ends_with([' ', '-']) {
                    t.truncate(t.len() - p.len());
                    break;
                }
            }
            if t.len() != before {
                break;
            }
        }
        t = t.trim_end_matches(['-', '–', ' ']).to_string();
        let n = normalize(&t);
        if n.is_empty() || ["the", "a", "an", "der", "die", "das"].contains(&n.as_str()) {
            return s.trim().to_string(); // "The Edition" is a title
        }
        if t.len() == before {
            return t;
        }
    }
}

/// The title with spelled-out umlauts written as umlauts, if it has any.
pub fn with_umlauts(title: &str) -> Option<String> {
    if title.chars().any(|c| "äöüÄÖÜ".contains(c)) {
        return None;
    }
    let u = title.replace("ae", "ä").replace("oe", "ö").replace("ue", "ü").replace("Ae", "Ä").replace("Oe", "Ö").replace("Ue", "Ü");
    (u != title).then_some(u)
}

/// Shorter search texts for a title nothing was found for: its first three, two and one words
/// (a subtitle written without a separator: "The Continental Aus der Welt von John Wick" →
/// "The Continental"), then the title without its first one or two words (a series in front:
/// "Edgar Wallace Das Rätsel des silbernen Halbmonds"), each also with umlauts. Results of these searches have to pass [`best_strict`].
pub fn shorter_queries(title: &str) -> Vec<String> {
    let words: Vec<&str> = title.split_whitespace().collect();
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        let n = normalize(&s);
        let article = ["the", "a", "an", "der", "die", "das"].contains(&n.as_str());
        if n.len() >= 3 && !article && normalize(title) != n && !out.iter().any(|o| normalize(o) == n) {
            out.push(s);
        }
    };
    for n in (1..words.len().min(4)).rev() {
        push(words[..n].join(" "));
    }
    if words.len() >= 3 {
        push(words[1..].join(" "));
    }
    if words.len() >= 4 {
        push(words[2..].join(" "));
    }
    let mut all = Vec::new();
    for q in out {
        let u = with_umlauts(&q);
        all.push(q);
        all.extend(u);
    }
    all.truncate(7);
    all
}

/// Score of a candidate for the wanted titles and year; higher is better.
pub fn score(c: &Candidate, wanted: &[String], year: i32, rank: usize) -> f64 {
    let titles = [normalize(&c.title), normalize(&c.original_title)];
    let sim = wanted.iter().flat_map(|w| titles.iter().map(move |t| similarity(w, t))).fold(0.0, f64::max);
    let mut s = sim * 6.0;
    if year > 0 && c.year > 0 {
        s += match (c.year - year).abs() {
            0 => 3.0,
            1 => 1.5,
            2 => 0.0,
            _ => -3.0,
        };
    }
    // The search engine's order and how well known an entry is only break ties.
    s -= rank as f64 * 0.15;
    s += (c.popularity.max(0.0) + 1.0).ln() * 0.1;
    if c.vote_count == 0 {
        s -= 0.5; // stubs without votes are rarely what a library holds
    }
    s
}

/// The best candidate if it is a believable match: a close title, or a fair title with the year.
pub fn best(candidates: &[Candidate], wanted: &[String], year: i32) -> Option<(i64, f64)> {
    pick(candidates, wanted, year, false)
}

/// Like [`best`], for results of shortened searches: a fair title needs the right year.
pub fn best_strict(candidates: &[Candidate], wanted: &[String], year: i32) -> Option<(i64, f64)> {
    pick(candidates, wanted, year, true)
}

fn pick(candidates: &[Candidate], wanted: &[String], year: i32, strict: bool) -> Option<(i64, f64)> {
    let wanted: Vec<String> = wanted.iter().map(|w| normalize(w)).filter(|w| !w.is_empty()).collect();
    candidates
        .iter()
        .enumerate()
        .map(|(rank, c)| {
            let titles = [normalize(&c.title), normalize(&c.original_title)];
            let sim = wanted.iter().flat_map(|w| titles.iter().map(move |t| similarity(w, t))).fold(0.0, f64::max);
            let year_ok = year > 0 && c.year > 0 && (c.year - year).abs() <= 1;
            let believable = sim >= 0.8 || (sim >= 0.55 && (year_ok || (year == 0 && !strict)));
            (c, score(c, &wanted, year, rank), believable)
        })
        .filter(|(_, _, ok)| *ok)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, s, _)| (c.id, s))
}

/// IMDb id (`tt` and 7–8 digits) written into a name: `Movie (1999) {imdb-tt0133093}`.
pub fn imdb_tag(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let at = lower.find("imdb")?;
    let rest = &lower[at + 4..];
    let start = rest.find("tt")?;
    if start > 3 {
        return None;
    }
    let digits: String = rest[start + 2..].chars().take_while(char::is_ascii_digit).collect();
    (7..=9).contains(&digits.len()).then(|| format!("tt{digits}"))
}

/// TVDB id written into a name: `Show {tvdb-121361}`.
pub fn tvdb_tag(name: &str) -> Option<i64> {
    let lower = name.to_lowercase();
    let at = lower.find("tvdb")?;
    let rest = lower[at + 4..].trim_start_matches(|c: char| c == 'i' || c == 'd' || "-=: ".contains(c));
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(id: i64, title: &str, original: &str, year: i32) -> Candidate {
        Candidate { id, title: title.into(), original_title: original.into(), year, popularity: 10.0, vote_count: 100 }
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("The Matrix"), "matrix");
        assert_eq!(normalize("Amélie"), "amelie");
        assert_eq!(normalize("Die Straße"), "strasse");
        assert_eq!(normalize("Fast & Furious"), "fast and furious");
        assert_eq!(normalize("Die"), "die");
    }

    #[test]
    fn similar_titles() {
        assert_eq!(similarity("matrix", "matrix"), 1.0);
        assert!(similarity("matrix", "matrix reloaded") > 0.6);
        assert!(similarity("matrix", "matrix reloaded") < 0.9);
        assert!(similarity("herr der ringe die gefahrten", "herr der ringe die gefaehrten") > 0.9);
        assert!(similarity("inception", "interstellar") < 0.5);
    }

    #[test]
    fn variants() {
        let v = query_variants("Die Schoene und das Biest", "");
        assert!(v.contains(&"Die Schöne und das Biest".to_string()));
        let v = query_variants("Mission: Impossible - Dead Reckoning", "");
        assert!(v.contains(&"Mission: Impossible".to_string()));
        assert!(v.contains(&"Mission".to_string()) || v.len() >= 2);
    }

    #[test]
    fn picks_the_right_one() {
        let list = vec![
            c(1, "Matrix Reloaded", "The Matrix Reloaded", 2003),
            c(2, "Matrix", "The Matrix", 1999),
            c(3, "Matrix Resurrections", "The Matrix Resurrections", 2021),
        ];
        assert_eq!(best(&list, &["The Matrix".into()], 1999).map(|b| b.0), Some(2));
        // The English file name matches the original title of the German result.
        let list = vec![c(7, "Die Verurteilten", "The Shawshank Redemption", 1994)];
        assert_eq!(best(&list, &["The Shawshank Redemption".into()], 1994).map(|b| b.0), Some(7));
        // Remakes: the year decides.
        let list = vec![c(10, "Dune", "Dune", 1984), c(11, "Dune", "Dune", 2021)];
        assert_eq!(best(&list, &["Dune".into()], 2021).map(|b| b.0), Some(11));
        // Nothing believable: better unmatched than wrong.
        let list = vec![c(20, "Something Else", "Something Else", 2010)];
        assert_eq!(best(&list, &["Inception".into()], 2010), None);
    }

    #[test]
    fn cleans_titles() {
        assert_eq!(cleaned("2 - Pippi geht von Bord"), "Pippi geht von Bord");
        assert_eq!(cleaned("02. Der Hobbit"), "Der Hobbit");
        assert_eq!(cleaned("7sins-jackass 35"), "jackass 35");
        assert_eq!(cleaned("Marvels What if"), "What if");
        assert_eq!(cleaned("Disney's Aladdin"), "Aladdin");
        assert_eq!(cleaned("300"), "300");
        assert_eq!(cleaned("2012"), "2012");
        assert_eq!(cleaned("X-Men"), "X-Men");
        assert_eq!(cleaned("Spider-Man"), "Spider-Man");
        assert_eq!(cleaned("12 Monkeys"), "12 Monkeys");
        assert_eq!(cleaned("Marvel"), "Marvel");
        assert_eq!(cleaned("Krieg der Sterne (Despecialized Edition 2.7)"), "Krieg der Sterne");
        assert_eq!(cleaned("Saw I DC"), "Saw I");
        assert_eq!(cleaned("Ex.Machina-1080"), "Ex Machina");
        assert_eq!(cleaned("pso-bfg webhd"), "bfg");
        assert_eq!(cleaned("Hitler.Aufstieg.des.Boesen - part 1"), "Hitler Aufstieg des Boesen");
        assert_eq!(cleaned("dfd-underworld.lycaner"), "underworld lycaner");
        assert_eq!(cleaned("Mr. Robot"), "Mr Robot");
        assert_eq!(cleaned("Das Boot"), "Das Boot");
        assert_eq!(cleaned("Web"), "Web");
        assert_eq!(cleaned("The Edition"), "The Edition");
        let v = query_variants("2 - Pippi geht von Bord", "");
        assert_eq!(v[1], "Pippi geht von Bord");
    }

    #[test]
    fn shorter() {
        assert_eq!(
            shorter_queries("The Continental Aus der Welt von John Wick"),
            vec!["The Continental Aus", "The Continental", "Aus der Welt von John Wick"]
        );
        assert_eq!(shorter_queries("Alladin Bud Spencer"), vec!["Alladin Bud", "Alladin", "Bud Spencer"]);
        assert_eq!(shorter_queries("Saw I"), vec!["Saw"]);
        let edgar = shorter_queries("Edgar Wallace Das Raetsel des silbernen Halbmonds");
        assert!(edgar.contains(&"Das Rätsel des silbernen Halbmonds".to_string()), "{edgar:?}");
        assert_eq!(shorter_queries("jackass 35"), vec!["jackass"]);
        assert!(shorter_queries("Up").is_empty());
        // A shortened search alone, without the year, is not enough.
        let list = vec![c(30, "Bud Spencer", "Bud Spencer", 2010)];
        assert_eq!(best_strict(&list, &["Alladin Bud Spencer".into()], 0), None);
        let list = vec![c(31, "Jackass 3.5", "Jackass 3.5", 2011)];
        assert_eq!(best_strict(&list, &["jackass 35".into()], 2011).map(|b| b.0), Some(31));
    }

    #[test]
    fn id_tags() {
        assert_eq!(imdb_tag("The Matrix (1999) {imdb-tt0133093}").as_deref(), Some("tt0133093"));
        assert_eq!(imdb_tag("[imdbid=tt12345678]").as_deref(), Some("tt12345678"));
        assert_eq!(imdb_tag("The Matrix"), None);
        assert_eq!(tvdb_tag("Dark {tvdb-334824}"), Some(334_824));
        assert_eq!(tvdb_tag("Dark [tvdbid=334824]"), Some(334_824));
        assert_eq!(tvdb_tag("Dark"), None);
    }
}
