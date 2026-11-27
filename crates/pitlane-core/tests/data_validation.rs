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
        ("Ford", "Mustang"), ("Ford", "F-150"), ("Chevrolet", "Camaro"), ("Chevrolet", "Corvette"), ("Chevrolet", "Silverado"),
        ("Dodge", "Challenger"), ("Dodge", "Charger"), ("Dodge", "Viper"), ("Honda", "Civic"), ("Honda", "Accord"),
        ("Acura", "Integra"), ("Acura", "RSX"), ("Acura", "NSX"), ("Toyota", "Supra"), ("Toyota", "Camry"),
        ("Toyota", "Tacoma"), ("Toyota", "4Runner"), ("Lexus", "IS"), ("Lexus", "GS"), ("Lexus", "LS"), ("Lexus", "LC"),
        ("Subaru", "WRX"), ("Subaru", "WRX STI"), ("Subaru", "BRZ"), ("Toyota", "86"), ("Jeep", "Wrangler"),
        ("Jeep", "Grand Cherokee"),
    ] {
        assert!(has(mk, md), "missing {mk} {md}");
    }
    assert!(db.cars_of_make("Jeep").iter().any(|c| c.trim.contains("Trackhawk")));
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
