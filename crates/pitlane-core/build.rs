//! Converts data/*.ron into compact blobs included with `include_bytes!`.
//! Cars use the fixed-record, string-interned layout documented on `model::CARS_MAGIC`,
//! so the app can read them in place without allocating a String per field.
#![allow(dead_code)]

#[path = "src/model.rs"]
mod model;

use model::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::PathBuf;

#[derive(Default)]
struct Interner {
    ids: HashMap<String, u16>,
    list: Vec<String>,
}

impl Interner {
    fn id(&mut self, s: &str) -> u16 {
        if let Some(&i) = self.ids.get(s) {
            return i;
        }
        let i = u16::try_from(self.list.len()).expect("more than 65535 distinct strings");
        self.ids.insert(s.to_string(), i);
        self.list.push(s.to_string());
        i
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let data_dir = manifest.join("../../data");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rerun-if-changed={}", data_dir.display());
    println!("cargo:rerun-if-changed=src/model.rs");

    let mut files: Vec<PathBuf> = fs::read_dir(&data_dir)
        .expect("data dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x == "ron").unwrap_or(false))
        .filter(|p| p.file_name().map(|n| n != "mods.ron").unwrap_or(false))
        .collect();
    files.sort();

    let mut by_make: BTreeMap<String, (String, Vec<Car>)> = BTreeMap::new();
    let mut ids = HashSet::new();
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let text = fs::read_to_string(f).expect("read ron");
        let src: FileSrc = ron::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        for car in expand(&src) {
            if let Err(e) = validate_car(&car) {
                panic!("{}: {e}", f.display());
            }
            if !ids.insert(car.id.to_string()) {
                panic!("duplicate id {}", car.id);
            }
            by_make.entry(car.make.to_string()).or_insert_with(|| (car.parent_group.to_string(), Vec::new())).1.push(car);
        }
    }

    let mut strings = Interner::default();
    let mut makes = Vec::new();
    let mut records = Vec::new();
    let mut car_count: u32 = 0;
    for (make, (group, cars)) in &by_make {
        let mut mask: u64 = 0;
        for c in cars {
            for y in c.year_start..=c.year_end {
                mask |= 1 << (y - YEAR_MIN);
            }
        }
        makes.extend(strings.id(make).to_le_bytes());
        makes.extend(strings.id(group).to_le_bytes());
        makes.extend(mask.to_le_bytes());
        makes.extend(car_count.to_le_bytes());
        makes.extend((cars.len() as u32).to_le_bytes());
        for c in cars {
            let start = records.len();
            for s in [&c.id, &c.make, &c.parent_group, &c.model, &c.generation, &c.trim, &c.engine_code, &c.transmission] {
                records.extend(strings.id(s).to_le_bytes());
            }
            for v in [c.year_start, c.year_end, c.hp, c.hp_rpm, c.torque, c.torque_rpm, c.weight_lbs, c.redline_rpm] {
                records.extend(v.to_le_bytes());
            }
            records.extend(c.displacement_l.to_le_bytes());
            records.extend(c.zero_sixty.to_le_bytes());
            records.extend([
                c.cylinders,
                c.aspiration as u8,
                c.drivetrain as u8,
                c.fuel as u8,
                c.zero_sixty_source as u8,
                c.confidence as u8,
            ]);
            assert_eq!(records.len() - start, CAR_RECORD_LEN);
            car_count += 1;
        }
    }

    let mut blob = Vec::new();
    blob.extend(CARS_MAGIC);
    blob.extend((strings.list.len() as u32).to_le_bytes());
    let mut bytes: Vec<u8> = Vec::new();
    for s in &strings.list {
        blob.extend((bytes.len() as u32).to_le_bytes());
        blob.extend((s.len() as u16).to_le_bytes());
        bytes.extend(s.as_bytes());
    }
    blob.extend((bytes.len() as u32).to_le_bytes());
    blob.extend(bytes);
    blob.extend((by_make.len() as u32).to_le_bytes());
    blob.extend(makes);
    blob.extend(car_count.to_le_bytes());
    blob.extend(records);
    fs::write(out_dir.join("cars.bin"), blob).expect("write cars.bin");

    let mods_text = fs::read_to_string(data_dir.join("mods.ron")).expect("mods.ron");
    println!("cargo:rerun-if-changed={}", data_dir.join("mods.ron").display());
    let mods: ModsFile = ron::from_str(&mods_text).unwrap_or_else(|e| panic!("mods.ron: {e}"));
    let mut mids = HashSet::new();
    for m in &mods.mods {
        assert!(mids.insert(m.id.clone()), "duplicate mod id {}", m.id);
        for r in m.effects.iter().flat_map(|e| e.requires.iter()).chain(m.conflicts.iter()) {
            assert!(mods.mods.iter().any(|x| &x.id == r), "mod {} references unknown mod {r}", m.id);
        }
    }
    fs::write(out_dir.join("mods.bin"), bincode::serialize(&mods.mods).expect("serialize mods")).expect("write mods.bin");
}
