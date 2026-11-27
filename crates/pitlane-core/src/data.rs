//! Bundled dataset access: zero-copy index over the embedded blob, lazy per-make decoding,
//! and merging of user overrides.

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
        let o: Overrides = ron::from_str(text).map_err(|e| format!("Could not parse overrides: {e}"))?;
        for c in &o.cars {
            validate_car(c)?;
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
    name: String,
    group: String,
    mask: u64,
    payload: &'static [u8],
}

pub struct Db {
    entries: Vec<MakeEntry>,
    cache: Vec<OnceLock<Vec<Car>>>,
    overrides: Overrides,
}

fn rd_u16(b: &[u8], at: &mut usize) -> Option<u16> {
    let v = u16::from_le_bytes(b.get(*at..*at + 2)?.try_into().ok()?);
    *at += 2;
    Some(v)
}
fn rd_u32(b: &[u8], at: &mut usize) -> Option<u32> {
    let v = u32::from_le_bytes(b.get(*at..*at + 4)?.try_into().ok()?);
    *at += 4;
    Some(v)
}
fn rd_u64(b: &[u8], at: &mut usize) -> Option<u64> {
    let v = u64::from_le_bytes(b.get(*at..*at + 8)?.try_into().ok()?);
    *at += 8;
    Some(v)
}
fn rd_str(b: &[u8], at: &mut usize) -> Option<String> {
    let n = rd_u16(b, at)? as usize;
    let s = std::str::from_utf8(b.get(*at..*at + n)?).ok()?.to_string();
    *at += n;
    Some(s)
}

fn parse_index(blob: &'static [u8]) -> Option<Vec<MakeEntry>> {
    let mut at = 0;
    let header_len = rd_u32(blob, &mut at)? as usize;
    let payload = blob.get(4 + header_len..)?;
    let count = rd_u32(blob, &mut at)?;
    let mut out = Vec::new();
    for _ in 0..count {
        let name = rd_str(blob, &mut at)?;
        let group = rd_str(blob, &mut at)?;
        let mask = rd_u64(blob, &mut at)?;
        let off = rd_u32(blob, &mut at)? as usize;
        let len = rd_u32(blob, &mut at)? as usize;
        out.push(MakeEntry { name, group, mask, payload: payload.get(off..off + len)? });
    }
    Some(out)
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
        let mut entries = parse_index(CARS_BIN).unwrap_or_default();
        for c in &overrides.cars {
            let mask: u64 = (c.year_start..=c.year_end).fold(0, |m, y| m | year_bit(y));
            match entries.iter_mut().find(|e| e.name == c.make) {
                Some(e) => e.mask |= mask,
                None => entries.push(MakeEntry { name: c.make.clone(), group: c.parent_group.clone(), mask, payload: &[] }),
            }
        }
        entries.sort_by_key(|a| a.name.to_lowercase());
        let cache = entries.iter().map(|_| OnceLock::new()).collect();
        Db { entries, cache, overrides }
    }

    pub fn overrides(&self) -> &Overrides {
        &self.overrides
    }

    pub fn make_count(&self) -> usize {
        self.entries.len()
    }

    /// (make, parent group) pairs.
    pub fn makes(&self) -> Vec<(&str, &str)> {
        self.entries.iter().map(|e| (e.name.as_str(), e.group.as_str())).collect()
    }

    fn load(&self, idx: usize) -> &[Car] {
        self.cache[idx].get_or_init(|| {
            let e = &self.entries[idx];
            let mut cars: Vec<Car> = if e.payload.is_empty() { Vec::new() } else { bincode::deserialize(e.payload).unwrap_or_default() };
            for ov in self.overrides.cars.iter().filter(|c| c.make == e.name) {
                match cars.iter_mut().find(|c| c.id == ov.id) {
                    Some(slot) => *slot = ov.clone(),
                    None => cars.push(ov.clone()),
                }
            }
            cars
        })
    }

    pub fn cars_of_make(&self, make: &str) -> &[Car] {
        match self.entries.iter().position(|e| e.name == make) {
            Some(i) => self.load(i),
            None => &[],
        }
    }

    pub fn makes_for_year(&self, year: u16) -> Vec<&str> {
        let bit = year_bit(year);
        self.entries.iter().filter(|e| e.mask & bit != 0).map(|e| e.name.as_str()).collect()
    }

    pub fn models_for(&self, year: u16, make: &str) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for c in self.cars_of_make(make).iter().filter(|c| c.covers_year(year)) {
            if !v.contains(&c.model.as_str()) {
                v.push(&c.model);
            }
        }
        v.sort_by_key(|m| m.to_lowercase());
        v
    }

    pub fn trims_for(&self, year: u16, make: &str, model: &str) -> Vec<&Car> {
        self.cars_of_make(make).iter().filter(|c| c.covers_year(year) && c.model == model).collect()
    }

    /// Looks a car up by id (decodes only the make the id belongs to, falling back to a scan).
    pub fn find(&self, id: &str) -> Option<&Car> {
        let hit = self.entries.iter().position(|e| id.starts_with(&format!("{}-", slug(&e.name))));
        if let Some(i) = hit {
            if let Some(c) = self.load(i).iter().find(|c| c.id == id) {
                return Some(c);
            }
        }
        (0..self.entries.len()).find_map(|i| self.load(i).iter().find(|c| c.id == id))
    }

    /// Every car (decodes all makes; used by search and tests).
    pub fn all_cars(&self) -> impl Iterator<Item = &Car> {
        (0..self.entries.len()).flat_map(move |i| self.load(i).iter())
    }
}
