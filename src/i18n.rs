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

//! Texts the server sends to users, in English and German.
//!
//! The catalogs are `i18n/messages_<language>.properties`, compiled into the program. A [`Text`]
//! is a catalog key with arguments; it is turned into a string in the language of the request
//! only when the response is written. English is the fallback for missing keys.

use std::collections::HashMap;
use std::sync::OnceLock;

/// Display languages of the server and of FreeX Web.
pub const LANGUAGES: [&str; 2] = ["en", "de"];

const CATALOG_EN: &str = include_str!("../i18n/messages_en.properties");
const CATALOG_DE: &str = include_str!("../i18n/messages_de.properties");

fn catalogs() -> &'static HashMap<&'static str, HashMap<String, String>> {
    static CATALOGS: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();
    CATALOGS.get_or_init(|| HashMap::from([("en", parse(CATALOG_EN)), ("de", parse(CATALOG_DE))]))
}

/// `key=value` lines; `#` starts a comment.
fn parse(source: &str) -> HashMap<String, String> {
    source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

pub fn supported(lang: &str) -> bool {
    LANGUAGES.contains(&lang)
}

/// The best supported language for an `Accept-Language` header such as `de-DE,de;q=0.9,en;q=0.8`.
pub fn negotiate(accept_language: Option<&str>) -> &'static str {
    let mut best = ("en", -1.0f32);
    for part in accept_language.unwrap_or("").split(',') {
        let mut fields = part.split(';');
        let tag = fields.next().unwrap_or("").trim().to_ascii_lowercase();
        let lang = tag.split('-').next().unwrap_or("");
        let q = fields.find_map(|f| f.trim().strip_prefix("q=").and_then(|q| q.parse::<f32>().ok())).unwrap_or(1.0);
        if let Some(&supported) = LANGUAGES.iter().find(|l| **l == lang)
            && q > best.1
        {
            best = (supported, q);
        }
    }
    best.0
}

/// An argument of a [`Text`]: plain text or another translatable text.
#[derive(Clone, Debug)]
pub enum Arg {
    Plain(String),
    Text(Text),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Plain(s.to_string())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Self {
        Arg::Plain(s)
    }
}

impl From<i64> for Arg {
    fn from(n: i64) -> Self {
        Arg::Plain(n.to_string())
    }
}

impl From<usize> for Arg {
    fn from(n: usize) -> Self {
        Arg::Plain(n.to_string())
    }
}

impl From<Text> for Arg {
    fn from(t: Text) -> Self {
        Arg::Text(t)
    }
}

/// A message by catalog key, translated when it is sent.
#[derive(Clone, Debug)]
pub struct Text {
    pub key: String,
    pub args: Vec<Arg>,
}

impl Text {
    pub fn new(key: &str) -> Self {
        Text { key: key.to_string(), args: Vec::new() }
    }

    pub fn arg(mut self, a: impl Into<Arg>) -> Self {
        self.args.push(a.into());
        self
    }

    /// The text in a language; placeholders `{0}`, `{1}`, ... are replaced by the arguments.
    pub fn render(&self, lang: &str) -> String {
        let all = catalogs();
        let pattern =
            all.get(lang).and_then(|c| c.get(&self.key)).or_else(|| all["en"].get(&self.key)).cloned().unwrap_or_else(|| self.key.clone());
        let mut out = pattern;
        for (i, a) in self.args.iter().enumerate() {
            let value = match a {
                Arg::Plain(s) => s.clone(),
                Arg::Text(t) => t.render(lang),
            };
            out = out.replace(&format!("{{{i}}}"), &value);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiates_by_quality() {
        assert_eq!(negotiate(Some("de-DE,de;q=0.9,en;q=0.8")), "de");
        assert_eq!(negotiate(Some("fr-FR,en;q=0.5")), "en");
        assert_eq!(negotiate(None), "en");
    }

    #[test]
    fn renders_with_nested_arguments() {
        let t = Text::new("error.tooManyAttempts").arg(Text::new("time.seconds").arg(30i64));
        assert!(t.render("en").contains("30"));
        assert!(!t.render("de").contains("{0}"));
    }

    #[test]
    fn every_key_is_translated() {
        let all = catalogs();
        let missing: Vec<_> = all["en"].keys().filter(|k| !all["de"].contains_key(*k)).collect();
        assert!(missing.is_empty(), "missing in German: {missing:?}");
    }
}
