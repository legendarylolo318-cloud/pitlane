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

fn score_text(tokens: &[String], hay: &str, words: &str) -> Option<i32> {
    let mut total = 0;
    for t in tokens {
        if hay.contains(t.as_str()) {
            total += 100;
            // prefer "sti" -> "WRX STI" over "sti" inside "Stingray"
            if words.split(' ').any(|w| w == t) {
                total += 60;
            } else if words.split(' ').any(|w| w.starts_with(t.as_str())) {
                total += 20;
            }
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
        // A query that only names the car ("ford ranger", "2005 ranger") gives one hit per car,
        // loaded with its first engine. Any word that only matches the engine/trim ("4.0",
        // "ecoboost", "type r") gives one hit per engine instead, so that engine gets picked.
        let car_only = !tokens.is_empty() && db.search_keys().iter().any(|k| tokens.iter().all(|t| k.car.contains(t.as_str())));
        let mut scored: Vec<(i32, u16, &Car)> = Vec::new();
        for k in db.search_keys() {
            let Some(c) = db.car_at(k.make, k.index) else { continue };
            if let Some(y) = year {
                if !c.covers_year(y) {
                    continue;
                }
            }
            let Some(score) = score_text(&tokens, &k.hay, &k.words) else { continue };
            if car_only {
                if !tokens.iter().all(|t| k.car.contains(t.as_str())) {
                    continue;
                }
                scored.push((score, k.index, c));
            } else {
                scored.push((score - (k.hay.len() / 8) as i32, k.index, c));
            }
        }
        if scored.is_empty() {
            continue;
        }
        let hit = |score: i32, y: u16, c: &Car| Hit { car_id: c.id.to_string(), year: y, label: format!("{} {} {} {}", y, c.make, c.model, c.trim), score };
        if !car_only {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.model.cmp(&b.2.model)).then_with(|| a.1.cmp(&b.1)));
            return scored.into_iter().take(limit).map(|(s, _, c)| hit(s, year.unwrap_or(c.year_end), c)).collect();
        }
        // group: with a year, one hit per make/model; without, one per generation (newest year)
        let mut groups: Vec<(i32, u16, &Car)> = Vec::new();
        let mut seen: Vec<(&str, &str, &str)> = Vec::new();
        for &(score, _, c) in &scored {
            let key = (&*c.make, &*c.model, if year.is_some() { "" } else { &*c.generation });
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            let members = scored.iter().filter(|(_, _, o)| o.make == key.0 && o.model == key.1 && (year.is_some() || o.generation == key.2));
            let y = year.unwrap_or_else(|| members.clone().map(|m| m.2.year_end).max().unwrap_or(c.year_end));
            // the first engine listed for that year
            let first = members.filter(|m| m.2.covers_year(y)).min_by_key(|m| m.1).map(|m| m.2).unwrap_or(c);
            groups.push((score, y, first));
        }
        groups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.model.cmp(&b.2.model)));
        return groups.into_iter().take(limit).map(|(s, y, c)| hit(s, y, c)).collect();
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
        let h = search(&db, "2020 supra", 5);
        assert!(h[0].label.contains("Supra") && h[0].year == 2020, "{h:?}");
        let h = search(&db, "hellcat", 20);
        assert!(h.iter().any(|x| x.label.contains("Challenger")) && h.iter().any(|x| x.label.contains("Charger")), "{h:?}");
        let h = search(&db, "'15 sti", 5);
        assert!(h[0].label.contains("WRX STI") && h[0].year == 2015, "{h:?}");
        // naming only the car: one hit per generation, newest first, each with its first engine
        let h = search(&db, "ford ranger", 10);
        assert_eq!(h.len(), 5, "{h:?}");
        assert!(h.iter().all(|x| x.label.contains("Ford Ranger")));
        assert_eq!(h[0].label, "2026 Ford Ranger 2.3 EcoBoost");
        assert_eq!(h[2].label, "2011 Ford Ranger 2.3 Duratec I4");
        // with a year: exactly that year's car, first engine of that year
        let h = search(&db, "2005 ford ranger", 10);
        assert_eq!(h.len(), 1, "{h:?}");
        assert_eq!((h[0].year, h[0].label.as_str()), (2005, "2005 Ford Ranger 2.3 Duratec I4"));
        let h = search(&db, "1999 ranger", 10);
        assert_eq!(h[0].label, "1999 Ford Ranger 2.5 I4");
        // engine words pick the engine
        let h = search(&db, "2005 ranger 4.0", 10);
        assert_eq!(h[0].label, "2005 Ford Ranger 4.0 SOHC V6", "{h:?}");
        let h = search(&db, "ranger ecoboost", 10);
        assert!(h.iter().all(|x| x.label.contains("EcoBoost")) && h.len() >= 4, "{h:?}");
        let h = search(&db, "ranger raptor", 10);
        assert_eq!(h[0].label, "2026 Ford Ranger Raptor 3.0 EcoBoost");
        let h = search(&db, "c8 z06", 5);
        assert!(h[0].label.contains("Z06"), "{h:?}");
        // subsequence fallback for typos with letters skipped
        assert!(!search(&db, "mstang", 5).is_empty());
        // year outside the dataset is treated as text, so it simply finds nothing
        assert!(search(&db, "1985 mustang", 5).iter().all(|x| x.year >= YEAR_MIN));
        assert!(search(&db, "zzzzqq", 5).is_empty());
        assert!(search(&db, "", 5).is_empty());
    }
}
