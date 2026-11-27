//! Bundled dataset access: zero-copy index over the embedded blob (strings are borrowed from
//! it, never copied), lazy per-make decoding, and merging of user overrides.

use crate::model::*;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

static CARS_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cars.bin"));
static MODS_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mods.bin"));

/// Mod definitions (decoded once).
pub fn mod_defs() -> &'static [ModDef] {
    static MODS: OnceLock<Vec<ModDef>> = OnceLock::new();
    MODS.get_or_init(|| bincode::deserialize(MODS_BIN).unwrap_or_default())
}

/// User edits: a car whose `id` matches a bundled car replaces it; any other car is added.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overrides {
    pub cars: Vec<Car>,
}

impl Overrides {
    pub fn to_text(&self) -> Result<String, String> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())
    }
    /// Parses and validates; nothing is accepted if any car is invalid.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let parsed: Overrides = ron::from_str(text).map_err(|e| format!("Could not parse overrides: {e}"))?;
        // A repeated id keeps its last definition (same as editing it twice).
        let mut o = Overrides::default();
        for c in parsed.cars {
            validate_car(&c)?;
            o.upsert(c);
        }
        Ok(o)
    }
    pub fn upsert(&mut self, car: Car) {
        match self.cars.iter_mut().find(|c| c.id == car.id) {
            Some(slot) => *slot = car,
            None => self.cars.push(car),
        }
    }
    pub fn remove(&mut self, id: &str) {
        self.cars.retain(|c| c.id != id);
    }
}

struct MakeEntry {
    name: Text,
    group: Text,
    /// `slug(name) + "-"`: bundled ids start with it, so `find` decodes only one make.
    id_prefix: String,
    mask: u64,
    /// Range of bundled records (`first..first + count`) belonging to this make.
    first: usize,
    count: usize,
}

/// Index over the embedded blob. Strings are borrowed from it, never copied.
struct Bundled {
    strings: Vec<&'static str>,
    records: &'static [u8],
}

pub struct Db {
    bundled: Bundled,
    entries: Vec<MakeEntry>,
    cache: Vec<OnceLock<Vec<Car>>>,
    search: OnceLock<Vec<SearchKey>>,
    overrides: Overrides,
}

/// Normalized search text for one car (built on first search, then reused for every keystroke).
pub struct SearchKey {
    pub make: u16,
    pub index: u16,
    /// All text glued together ("hondacivicsi..."), for substring/subsequence matches.
    pub hay: Box<str>,
    /// The same words, space separated, for whole-word bonuses.
    pub words: Box<str>,
    /// Make + model only ("fordranger"): a query that only names the car, not an engine.
    pub car: Box<str>,
}

struct Reader {
    b: &'static [u8],
    at: usize,
}

impl Reader {
    fn take(&mut self, n: usize) -> Option<&'static [u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn parse_blob(blob: &'static [u8]) -> Option<(Bundled, Vec<MakeEntry>)> {
    let mut r = Reader { b: blob, at: 0 };
    if r.take(4)? != CARS_MAGIC {
        return None;
    }
    let n = r.u32()? as usize;
    let mut spans = Vec::with_capacity(n);
    for _ in 0..n {
        spans.push((r.u32()? as usize, r.u16()? as usize));
    }
    let bytes_len = r.u32()? as usize;
    let text = std::str::from_utf8(r.take(bytes_len)?).ok()?;
    let mut strings = Vec::with_capacity(n);
    for (off, len) in spans {
        strings.push(text.get(off..off.checked_add(len)?)?);
    }
    let str_at = |i: u16| strings.get(i as usize).copied();
    let makes = r.u32()? as usize;
    let mut entries = Vec::with_capacity(makes);
    for _ in 0..makes {
        let name = str_at(r.u16()?)?;
        let group = str_at(r.u16()?)?;
        let mask = r.u64()?;
        let first = r.u32()? as usize;
        let count = r.u32()? as usize;
        entries.push(MakeEntry { name: Text::Borrowed(name), group: Text::Borrowed(group), id_prefix: format!("{}-", slug(name)), mask, first, count });
    }
    let cars = r.u32()? as usize;
    let records = r.take(cars.checked_mul(CAR_RECORD_LEN)?)?;
    // Validate every reference once, so decoding can index without checks failing later.
    for rec in records.as_chunks::<CAR_RECORD_LEN>().0 {
        for k in 0..8 {
            str_at(le16(rec, k * 2))?;
        }
        let e = &rec[40..46];
        Aspiration::from_u8(e[1])?;
        Drivetrain::from_u8(e[2])?;
        Fuel::from_u8(e[3])?;
        SourceKind::from_u8(e[4])?;
        Confidence::from_u8(e[5])?;
    }
    if entries.iter().any(|e| e.first + e.count > cars) {
        return None;
    }
    Some((Bundled { strings, records }, entries))
}

impl Bundled {
    fn car(&self, i: usize) -> Car {
        let rec = &self.records[i * CAR_RECORD_LEN..(i + 1) * CAR_RECORD_LEN];
        let s = |k: usize| Text::Borrowed(self.strings[le16(rec, k * 2) as usize]);
        let n = |k: usize| le16(rec, 16 + k * 2);
        let f = |at: usize| f32::from_le_bytes([rec[at], rec[at + 1], rec[at + 2], rec[at + 3]]);
        let e = &rec[40..46];
        Car {
            id: s(0),
            make: s(1),
            parent_group: s(2),
            model: s(3),
            generation: s(4),
            trim: s(5),
            engine_code: s(6),
            transmission: s(7),
            year_start: n(0),
            year_end: n(1),
            hp: n(2),
            hp_rpm: n(3),
            torque: n(4),
            torque_rpm: n(5),
            weight_lbs: n(6),
            redline_rpm: n(7),
            displacement_l: f(32),
            zero_sixty: f(36),
            cylinders: e[0],
            // parse_blob validated these; the fallbacks are unreachable
            aspiration: Aspiration::from_u8(e[1]).unwrap_or(Aspiration::NA),
            drivetrain: Drivetrain::from_u8(e[2]).unwrap_or(Drivetrain::RWD),
            fuel: Fuel::from_u8(e[3]).unwrap_or_default(),
            zero_sixty_source: SourceKind::from_u8(e[4]).unwrap_or_default(),
            confidence: Confidence::from_u8(e[5]).unwrap_or_default(),
        }
    }
}

fn year_bit(y: u16) -> u64 {
    if (YEAR_MIN..=YEAR_MAX).contains(&y) {
        1u64 << (y - YEAR_MIN)
    } else {
        0
    }
}

impl Db {
    pub fn bundled() -> Self {
        Self::with_overrides(Overrides::default())
    }

    /// Builds the index from the embedded blob (no car is decoded yet).
    pub fn with_overrides(overrides: Overrides) -> Self {
        let (bundled, mut entries) = parse_blob(CARS_BIN).unwrap_or((Bundled { strings: Vec::new(), records: &[] }, Vec::new()));
        for c in &overrides.cars {
            let mask: u64 = (c.year_start..=c.year_end).fold(0, |m, y| m | year_bit(y));
            match entries.iter_mut().find(|e| e.name == c.make) {
                Some(e) => e.mask |= mask,
                None => entries.push(MakeEntry {
                    name: c.make.clone(),
                    group: c.parent_group.clone(),
                    id_prefix: format!("{}-", slug(&c.make)),
                    mask,
                    first: 0,
                    count: 0,
                }),
            }
        }
        entries.sort_by_cached_key(|a| a.name.to_lowercase());
        let cache = entries.iter().map(|_| OnceLock::new()).collect();
        Db { bundled, entries, cache, search: OnceLock::new(), overrides }
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    pub fn make_count(&self) -> usize {
        self.entries.len()
    }

    /// (make, parent group) pairs.
    pub fn makes(&self) -> Vec<(&str, &str)> {
        self.entries.iter().map(|e| (&*e.name, &*e.group)).collect()
    }

    fn load(&self, idx: usize) -> &[Car] {
        self.cache[idx].get_or_init(|| {
            let e = &self.entries[idx];
            let mut cars: Vec<Car> = (e.first..e.first + e.count).map(|i| self.bundled.car(i)).collect();
            for ov in self.overrides.cars.iter().filter(|c| c.make == e.name) {
                match cars.iter_mut().find(|c| c.id == ov.id) {
                    Some(slot) => *slot = ov.clone(),
                    None => cars.push(ov.clone()),
                }
            }
            cars
        })
    }

    fn make_index(&self, make: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == make)
    }

    pub fn cars_of_make(&self, make: &str) -> &[Car] {
        match self.make_index(make) {
            Some(i) => self.load(i),
            None => &[],
        }
    }

    pub fn makes_for_year(&self, year: u16) -> Vec<&str> {
        let bit = year_bit(year);
        self.entries.iter().filter(|e| e.mask & bit != 0).map(|e| &*e.name).collect()
    }

    pub fn models_for(&self, year: u16, make: &str) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for c in self.cars_of_make(make).iter().filter(|c| c.covers_year(year)) {
            if !v.contains(&&*c.model) {
                v.push(&c.model);
            }
        }
        v.sort_by_cached_key(|m| m.to_lowercase());
        v
    }

    pub fn trims_for(&self, year: u16, make: &str, model: &str) -> Vec<&Car> {
        self.cars_of_make(make).iter().filter(|c| c.covers_year(year) && c.model == model).collect()
    }

    /// Looks a car up by id, decoding only the make it belongs to.
    pub fn find(&self, id: &str) -> Option<&Car> {
        let make = match self.overrides.cars.iter().find(|c| c.id == id) {
            Some(ov) => self.make_index(&ov.make),
            None => self.entries.iter().position(|e| e.count > 0 && id.starts_with(&e.id_prefix)),
        };
        if let Some(i) = make {
            if let Some(c) = self.load(i).iter().find(|c| c.id == id) {
                return Some(c);
            }
        }
        // make slugs can be prefixes of each other ("ram-" vs a make called "Ram Trucks"): scan as a fallback
        (0..self.entries.len()).find_map(|i| self.load(i).iter().find(|c| c.id == id))
    }

    /// Car by its position in the search index.
    pub fn car_at(&self, make: u16, index: u16) -> Option<&Car> {
        if make as usize >= self.entries.len() {
            return None;
        }
        self.load(make as usize).get(index as usize)
    }

    /// Normalized search keys for every car (built once, on first use).
    pub fn search_keys(&self) -> &[SearchKey] {
        self.search.get_or_init(|| {
            let mut out = Vec::new();
            for mi in 0..self.entries.len() {
                for (ci, c) in self.load(mi).iter().enumerate() {
                    let text = format!("{} {} {} {} {}", c.make, c.model, c.trim, c.generation, c.engine_code);
                    let words: Vec<String> = text.split(|ch: char| !ch.is_alphanumeric()).map(crate::search::normalize).filter(|w| !w.is_empty()).collect();
                    let car = crate::search::normalize(&format!("{}{}", c.make, c.model));
                    out.push(SearchKey {
                        make: mi as u16,
                        index: ci as u16,
                        hay: words.concat().into_boxed_str(),
                        words: words.join(" ").into_boxed_str(),
                        car: car.into_boxed_str(),
                    });
                }
            }
            out
        })
    }

    /// Every car (decodes all makes; used by search and tests).
    pub fn all_cars(&self) -> impl Iterator<Item = &Car> {
        (0..self.entries.len()).flat_map(move |i| self.load(i).iter())
    }
}
