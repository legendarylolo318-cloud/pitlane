//! Converts data/*.ron into compact bincode blobs included with `include_bytes!`.
//!
//! cars.bin layout (little endian):
//!   u32 make_count
//!   per make: u16 name_len, name, u16 group_len, group, u64 year_mask, u32 offset, u32 len
//!   payload: per make, bincode(Vec<Car>) at `offset` (relative to the start of the payload)
#![allow(dead_code)]

#[path = "src/model.rs"]
mod model;

use model::*;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;

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
        let text = fs::read_to_string(f).expect("read ron");
        let src: FileSrc = ron::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        for car in expand(&src) {
            if let Err(e) = validate_car(&car) {
                panic!("{}: {e}", f.display());
            }
            if !ids.insert(car.id.clone()) {
                panic!("duplicate id {}", car.id);
            }
            by_make.entry(car.make.clone()).or_insert_with(|| (car.parent_group.clone(), Vec::new())).1.push(car);
        }
    }

    let mut header = Vec::new();
    let mut payload = Vec::new();
    header.extend((by_make.len() as u32).to_le_bytes());
    for (make, (group, cars)) in &by_make {
        let mut mask: u64 = 0;
        for c in cars {
            for y in c.year_start..=c.year_end {
                mask |= 1 << (y - YEAR_MIN);
            }
        }
        let bytes = bincode::serialize(cars).expect("serialize cars");
        header.extend((make.len() as u16).to_le_bytes());
        header.extend(make.as_bytes());
        header.extend((group.len() as u16).to_le_bytes());
        header.extend(group.as_bytes());
        header.extend(mask.to_le_bytes());
        header.extend((payload.len() as u32).to_le_bytes());
        header.extend((bytes.len() as u32).to_le_bytes());
        payload.extend(bytes);
    }
    let mut blob = Vec::new();
    blob.extend((header.len() as u32).to_le_bytes());
    blob.extend(header);
    blob.extend(payload);
    fs::write(out_dir.join("cars.bin"), blob).expect("write cars.bin");

    let mods_text = fs::read_to_string(data_dir.join("mods.ron")).expect("mods.ron");
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
