use pitlane_core::*;
use std::collections::HashSet;

#[test]
fn dataset_is_valid() {
    let db = Db::bundled();
    let mut ids = HashSet::new();
    let mut n = 0;
    for c in db.all_cars() {
        n += 1;
        validate_car(c).unwrap_or_else(|e| panic!("{e}"));
        assert!(ids.insert(c.id.clone()), "duplicate id {}", c.id);
        assert!(c.year_start >= YEAR_MIN && c.year_end <= YEAR_MAX, "{}: years", c.id);
        assert!(c.hp_rpm == 0 || (1000..=11_000).contains(&c.hp_rpm), "{}: hp rpm", c.id);
        assert!(c.torque_rpm == 0 || (500..=9_000).contains(&c.torque_rpm), "{}: tq rpm", c.id);
    }
    assert!(n > 500, "dataset unexpectedly small: {n}");
}

#[test]
fn every_required_brand_is_present() {
    let db = Db::bundled();
    let makes: Vec<&str> = db.makes().into_iter().map(|m| m.0).collect();
    for m in [
        "Ford", "Lincoln", "Mercury", "Chevrolet", "GMC", "Cadillac", "Buick", "Pontiac", "Oldsmobile", "Saturn",
        "Hummer", "Geo", "Dodge", "Chrysler", "Plymouth", "Ram", "Eagle", "SRT", "Jeep", "Honda", "Acura", "Toyota",
        "Lexus", "Scion", "Subaru",
    ] {
        assert!(makes.contains(&m), "missing make {m}");
        assert!(!db.cars_of_make(m).is_empty(), "no cars for {m}");
    }
}

#[test]
fn every_make_has_cars_in_its_years_and_pickers_cascade() {
    let db = Db::bundled();
    for y in [1990u16, 2000, 2010, 2020, 2026] {
        let makes = db.makes_for_year(y);
        assert!(!makes.is_empty(), "no makes in {y}");
        for m in makes {
            let models = db.models_for(y, m);
            assert!(!models.is_empty(), "{m} listed for {y} but has no models");
            for model in models {
                assert!(!db.trims_for(y, m, model).is_empty());
            }
        }
    }
    assert!(db.makes_for_year(1989).is_empty() && db.makes_for_year(2027).is_empty());
}

#[test]
fn priority_models_exist() {
    let db = Db::bundled();
    let has = |make: &str, model: &str| db.cars_of_make(make).iter().any(|c| c.model.contains(model));
    for (mk, md) in [
        ("Ford", "Mustang"), ("Ford", "F-150"), ("Ford", "Ranger"), ("Chevrolet", "Camaro"), ("Chevrolet", "Corvette"), ("Chevrolet", "Silverado"),
        ("Dodge", "Challenger"), ("Dodge", "Charger"), ("Dodge", "Viper"), ("Honda", "Civic"), ("Honda", "Accord"),
        ("Acura", "Integra"), ("Acura", "RSX"), ("Acura", "NSX"), ("Toyota", "Supra"), ("Toyota", "Camry"),
        ("Toyota", "Tacoma"), ("Toyota", "4Runner"), ("Lexus", "IS"), ("Lexus", "GS"), ("Lexus", "LS"), ("Lexus", "LC"),
        ("Subaru", "WRX"), ("Subaru", "WRX STI"), ("Subaru", "BRZ"), ("Toyota", "86"), ("Jeep", "Wrangler"),
        ("Jeep", "Grand Cherokee"),
    ] {
        assert!(has(mk, md), "missing {mk} {md}");
    }
    assert!(db.cars_of_make("Jeep").iter().any(|c| c.trim.contains("Trackhawk")));
    // every Silverado 1500 and 2500 HD year, several engines each, 2WD and 4WD
    for y in 1990..=2026 {
        for model in ["Silverado 1500", "Silverado 2500 HD"] {
            let trims = db.trims_for(y, "Chevrolet", model);
            assert!(trims.len() >= 4, "{y} {model}: only {} cars", trims.len());
            assert!(trims.iter().any(|c| c.drivetrain == Drivetrain::FourWD), "{y} {model}: no 4WD");
            assert!(trims.iter().any(|c| c.drivetrain == Drivetrain::RWD), "{y} {model}: no 2WD");
        }
    }
    // every US-market Ranger year (not sold in the US 2012-2018)
    for y in (1990..=2011).chain(2019..=2026) {
        assert!(!db.trims_for(y, "Ford", "Ranger").is_empty(), "no Ford Ranger in {y}");
    }
}

#[test]
fn overrides_merge_on_top() {
    let base = Db::bundled();
    let mut car = base.all_cars().next().unwrap().clone();
    let original_hp = car.hp;
    car.hp = original_hp + 10;
    let mut custom = car.clone();
    custom.id = "custom-test".into();
    custom.make = "Pitlane Motors".into();
    custom.parent_group = "Custom".into();
    let mut ov = Overrides::default();
    ov.upsert(car.clone());
    ov.upsert(custom);
    let text = ov.to_text().unwrap();
    let ov2 = Overrides::from_text(&text).unwrap();
    let db = Db::with_overrides(ov2);
    assert_eq!(db.find(&car.id).unwrap().hp, original_hp + 10);
    assert!(db.find("custom-test").is_some());
    assert!(db.makes().iter().any(|m| m.0 == "Pitlane Motors"));
    assert_eq!(db.make_count(), base.make_count() + 1);
    // invalid override is rejected
    let mut bad = car;
    bad.year_start = 1985;
    let t = Overrides { cars: vec![bad] }.to_text().unwrap();
    assert!(Overrides::from_text(&t).is_err());
}

#[test]
fn mod_definitions_are_sane() {
    let defs = mod_defs();
    assert!(defs.len() >= 18);
    for d in defs {
        assert!(!d.effects.is_empty(), "{}", d.id);
        for e in &d.effects {
            assert!(e.min_pct <= e.max_pct, "{}: min_pct > max_pct", d.id);
            assert!(e.unit_pct_min <= e.unit_pct_max, "{}", d.id);
            assert!(e.unit_hp_min <= e.unit_hp_max, "{}", d.id);
        }
        if let Some(s) = &d.slider {
            assert!(s.min <= s.default && s.default <= s.max, "{}", d.id);
        }
    }
}

fn car<'a>(db: &'a Db, make: &str, model: &str, year: u16, trim: &str) -> &'a Car {
    db.trims_for(year, make, model)
        .into_iter()
        .find(|c| c.trim == trim)
        .unwrap_or_else(|| panic!("missing {year} {make} {model} {trim}: have {:?}", db.trims_for(year, make, model).iter().map(|c| &c.trim).collect::<Vec<_>>()))
}

/// Spot checks against widely published factory figures (hp, lb-ft, curb weight +/- 3 %).
#[test]
fn reference_cars_match_published_figures() {
    let db = Db::bundled();
    let check = |make: &str, model: &str, year: u16, trim: &str, hp: u16, tq: u16, weight: u16| {
        let c = car(&db, make, model, year, trim);
        assert_eq!((c.hp, c.torque), (hp, tq), "{year} {make} {model} {trim}");
        let dw = (c.weight_lbs as f32 - weight as f32).abs() / weight as f32;
        assert!(dw < 0.03, "{year} {make} {model} {trim}: weight {} vs {weight}", c.weight_lbs);
    };
    check("Ford", "Mustang", 2015, "GT 5.0", 435, 400, 3705);
    check("Honda", "Civic", 2017, "Type R 2.0T (FK8)", 306, 295, 3117);
    check("Toyota", "Supra", 2020, "3.0 B58 (2020)", 335, 365, 3397);
    check("Toyota", "Supra", 2021, "3.0 B58 (2021+)", 382, 368, 3397);
    check("Chevrolet", "Corvette", 2020, "Stingray LT2", 495, 470, 3535);
    check("Dodge", "Challenger", 2015, "SRT Hellcat 6.2 SC", 707, 650, 4448);
    check("Subaru", "WRX STI", 2015, "STI 2.5T (2015-21)", 305, 290, 3391);
    check("Jeep", "Grand Cherokee", 2018, "Trackhawk 6.2 SC", 707, 645, 5363);
    check("Lexus", "LS", 1990, "LS400 4.0 V8 (XF10)", 250, 260, 3759);
    check("Acura", "NSX", 1991, "3.0 V6 VTEC", 270, 210, 3010);
    check("Acura", "NSX", 2017, "3.5 TT Hybrid", 573, 476, 3878);
}

/// The same make/model/year must not offer two trims with the same name, or two identical cars.
#[test]
fn no_duplicate_or_shadowed_trims() {
    let db = Db::bundled();
    let mut bad = Vec::new();
    for (make, _) in db.makes() {
        for y in YEAR_MIN..=YEAR_MAX {
            for model in db.models_for(y, make) {
                let trims = db.trims_for(y, make, model);
                for (i, a) in trims.iter().enumerate() {
                    for b in &trims[i + 1..] {
                        if a.trim == b.trim && a.generation == b.generation && a.drivetrain == b.drivetrain {
                            bad.push(format!("{y} {make} {model}: '{}' {} listed twice", a.trim, a.drivetrain.label()));
                        }
                        let same = a.hp == b.hp && a.torque == b.torque && a.weight_lbs == b.weight_lbs && a.drivetrain == b.drivetrain && a.transmission == b.transmission && a.engine_code == b.engine_code;
                        if same && a.zero_sixty == b.zero_sixty && a.trim != b.trim {
                            bad.push(format!("{y} {make} {model}: '{}' and '{}' are identical", a.trim, b.trim));
                        }
                    }
                }
            }
        }
    }
    bad.dedup();
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Prints per-make coverage (`cargo test -- --nocapture coverage`) so gaps are easy to see.
#[test]
fn coverage_report() {
    let db = Db::bundled();
    let mut total = 0;
    for (make, group) in db.makes() {
        let cars = db.cars_of_make(make);
        total += cars.len();
        let years: Vec<u16> = (YEAR_MIN..=YEAR_MAX).filter(|y| cars.iter().any(|c| c.covers_year(*y))).collect();
        let missing: Vec<u16> = (YEAR_MIN..=YEAR_MAX).filter(|y| !years.contains(y)).collect();
        let verified = cars.iter().filter(|c| c.confidence == Confidence::Verified).count();
        println!("{make:<11} {group:<26} {:>3} trims, {verified:>3} verified, years {}-{}, gaps {:?}", cars.len(), years.first().unwrap_or(&0), years.last().unwrap_or(&0), missing);
    }
    println!("total {total}");
}
