//! Fuzzy search: "06 civic si", "lx570", "'99 supra tt".

use crate::data::Db;
use crate::model::*;

#[derive(Clone, Debug)]
pub struct Hit {
    pub car_id: String,
    pub year: u16,
    pub label: String,
    pub score: i32,
}

pub fn normalize(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|n| it.any(|h| h == n))
}

/// Parses a year token: 4 digits in range, or 2 digits (90-99 -> 19xx, 00-26 -> 20xx).
pub fn parse_year(tok: &str) -> Option<u16> {
    let t = tok.trim_start_matches('\'').trim_start_matches('’');
    if !t.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u16 = t.parse().ok()?;
    let y = match t.len() {
        4 => n,
        2 if n >= 90 => 1900 + n,
        2 if n <= 26 => 2000 + n,
        _ => return None,
    };
    (YEAR_MIN..=YEAR_MAX).contains(&y).then_some(y)
}

fn score_text(tokens: &[String], hay: &str) -> Option<i32> {
    let mut total = 0;
    for t in tokens {
        if hay.contains(t.as_str()) {
            total += 100;
        } else if t.len() >= 3 && is_subsequence(t, hay) {
            total += 25;
        } else {
            return None;
        }
    }
    // also reward matching the whole query glued together (e.g. "lx 570" vs "lx570")
    let glued: String = tokens.concat();
    if tokens.len() > 1 && hay.contains(&glued) {
        total += 40;
    }
    Some(total)
}

pub fn search(db: &Db, query: &str, limit: usize) -> Vec<Hit> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }
    let year_idx = words.iter().position(|w| parse_year(w).is_some());
    for use_year in [true, false] {
        let year = if use_year { year_idx.and_then(|i| parse_year(words[i])) } else { None };
        if !use_year && year_idx.is_none() {
            break;
        }
        let tokens: Vec<String> = words
            .iter()
            .enumerate()
            .filter(|(i, _)| !(use_year && Some(*i) == year_idx))
            .map(|(_, w)| normalize(w))
            .filter(|t| !t.is_empty())
            .collect();
        if tokens.is_empty() && year.is_none() {
            continue;
        }
        let mut hits: Vec<Hit> = Vec::new();
        for c in db.all_cars() {
            if let Some(y) = year {
                if !c.covers_year(y) {
                    continue;
                }
            }
            let hay = normalize(&format!("{} {} {} {} {}", c.make, c.model, c.trim, c.generation, c.engine_code));
            let Some(mut score) = score_text(&tokens, &hay) else { continue };
            score -= (hay.len() / 8) as i32;
            let y = year.unwrap_or(c.year_end);
            hits.push(Hit {
                car_id: c.id.clone(),
                year: y,
                label: format!("{} {} {} {}", y, c.make, c.model, c.trim),
                score,
            });
        }
        if !hits.is_empty() {
            hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.label.cmp(&b.label)));
            hits.truncate(limit);
            return hits;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn years() {
        assert_eq!(parse_year("06"), Some(2006));
        assert_eq!(parse_year("'99"), Some(1999));
        assert_eq!(parse_year("2015"), Some(2015));
        assert_eq!(parse_year("86"), None);
        assert_eq!(parse_year("1985"), None);
        assert_eq!(parse_year("570"), None);
    }

    #[test]
    fn finds_cars() {
        let db = Db::bundled();
        let h = search(&db, "06 civic si", 5);
        assert!(!h.is_empty());
        assert!(h[0].label.contains("Civic") && h[0].year == 2006, "{:?}", h[0]);
        let h = search(&db, "lx570", 5);
        assert!(h.iter().any(|x| x.label.contains("LX")), "{h:?}");
        assert!(search(&db, "zzzzqq", 5).is_empty());
        assert!(search(&db, "", 5).is_empty());
    }
}
