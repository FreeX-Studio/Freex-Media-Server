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

//! Metadata edited by hand. Every field set here wins over the file names, `.nfo` files and
//! online metadata, also after scans and refreshes, until it is reset.

use crate::media::Item;
use crate::media::names::sort_title;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Edit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tagline: Option<String>,
    /// Comma separated, like the entries keep them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genres: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<String>,
    /// Uploaded artwork in the data directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backdrop: Option<String>,
}

/// Fields an admin can edit, as the API names them.
pub const FIELDS: [&str; 9] = ["title", "sortTitle", "originalTitle", "year", "summary", "tagline", "genres", "rating", "contentRating"];

impl Edit {
    pub fn is_empty(&self) -> bool {
        *self == Edit::default()
    }

    /// Writes the edited fields over an entry.
    pub fn apply(&self, it: &mut Item) {
        if let Some(t) = &self.title {
            it.title.clone_from(t);
            if self.sort_title.is_none() {
                it.sort_title = match it.kind {
                    crate::media::Kind::Episode => format!("{:04} {}", it.index, sort_title(t)),
                    crate::media::Kind::Season => it.sort_title.clone(),
                    _ => sort_title(t),
                };
            }
        }
        if let Some(s) = &self.sort_title {
            it.sort_title = match it.kind {
                crate::media::Kind::Episode => format!("{:04} {}", it.index, s.to_lowercase()),
                _ => s.to_lowercase(),
            };
        }
        if let Some(v) = &self.original_title {
            it.original_title.clone_from(v);
        }
        if let Some(y) = self.year {
            it.year = y;
        }
        if let Some(v) = &self.summary {
            it.summary.clone_from(v);
        }
        if let Some(v) = &self.tagline {
            it.tagline.clone_from(v);
        }
        if let Some(v) = &self.genres {
            it.genres.clone_from(v);
        }
        if let Some(r) = self.rating {
            it.rating = r;
        }
        if let Some(v) = &self.content_rating {
            it.content_rating.clone_from(v);
        }
        if let Some(p) = self.poster.as_ref().filter(|p| std::path::Path::new(p).is_file()) {
            it.poster_path = Some(p.clone());
        }
        if let Some(p) = self.backdrop.as_ref().filter(|p| std::path::Path::new(p).is_file()) {
            it.backdrop_path = Some(p.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Kind;

    #[test]
    fn edited_fields_win() {
        let mut it = Item::new("a".into(), "l", Kind::Movie);
        it.title = "Matrix".into();
        it.year = 1999;
        let e = Edit { title: Some("The Matrix (Director's Cut)".into()), genres: Some("Action, Sci-Fi".into()), ..Edit::default() };
        e.apply(&mut it);
        assert_eq!((it.title.as_str(), it.year, it.genres.as_str()), ("The Matrix (Director's Cut)", 1999, "Action, Sci-Fi"));
        assert_eq!(it.sort_title, sort_title("The Matrix (Director's Cut)"));
        assert!(Edit::default().is_empty() && !e.is_empty());
    }
}
