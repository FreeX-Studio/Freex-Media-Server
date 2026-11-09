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

//! Age ratings as a minimum age, for the parental controls. Ratings come as TMDB and `.nfo`
//! files write them: German FSK ("FSK 12", "12", "DE:12"), US film (MPAA) and TV ratings, and the
//! plain ages most other countries use.

/// The minimum age a rating stands for; None if the rating is unknown or empty.
pub fn age_of(rating: &str) -> Option<i32> {
    let mut r = rating.trim().to_uppercase();
    // "DE:12", "US:PG-13"
    if let Some((country, rest)) = r.split_once(':')
        && country.len() <= 3
    {
        r = rest.trim().to_string();
    }
    let r = r.strip_prefix("RATED ").unwrap_or(&r).trim().to_string();
    let r = r.strip_prefix("FSK").or_else(|| r.strip_prefix("FSK-")).unwrap_or(&r).trim_matches(|c: char| c == ' ' || c == '-');
    let named = match r {
        "G" | "TV-Y" | "TV-G" | "U" | "ALL" | "AL" | "0" | "O.AL." | "OHNE ALTERSBESCHRÄNKUNG" => Some(0),
        "TV-Y7" | "TV-Y7-FV" => Some(7),
        "PG" | "TV-PG" => Some(10),
        "PG-13" => Some(13),
        "TV-14" => Some(14),
        "R" | "TV-MA" => Some(17),
        "NC-17" | "X" => Some(18),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    // "12", "16+", "12A", "-16"
    let digits: String = r.trim_start_matches('-').chars().take_while(char::is_ascii_digit).collect();
    digits.parse::<i32>().ok().filter(|a| (0..=21).contains(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages() {
        assert_eq!(age_of("FSK 12"), Some(12));
        assert_eq!(age_of("FSK 0"), Some(0));
        assert_eq!(age_of("DE:16"), Some(16));
        assert_eq!(age_of("16"), Some(16));
        assert_eq!(age_of("PG-13"), Some(13));
        assert_eq!(age_of("Rated R"), Some(17));
        assert_eq!(age_of("TV-MA"), Some(17));
        assert_eq!(age_of("TV-Y7"), Some(7));
        assert_eq!(age_of("12A"), Some(12));
        assert_eq!(age_of(""), None);
        assert_eq!(age_of("NR"), None);
        assert_eq!(age_of("Unrated"), None);
    }
}
