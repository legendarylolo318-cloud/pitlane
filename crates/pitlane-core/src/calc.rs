//! Mod math: stacking, aspiration rules, environment, calibrated 0-60 estimate.
//! All numbers come from `ModDef` data (data/mods.ron); nothing is hardcoded per mod.

use crate::model::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModSel {
    pub id: String,
    /// Slider value (ignored for mods without a slider).
    pub value: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Range {
    pub min: f32,
    pub typ: f32,
    pub max: f32,
}

impl Range {
    fn of(v: [f32; 3]) -> Self {
        Range { min: v[0].min(v[2]), typ: v[1], max: v[0].max(v[2]) }
    }
}

/// A warning attached to one mod (so the UI can show it next to that mod).
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub mod_id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Eval {
    pub hp: Range,
    pub torque: Range,
    pub weight_lbs: f32,
    /// 0-60 mph, seconds. `min` is the quickest (from max hp).
    pub zero_sixty: Range,
    pub aspiration: Aspiration,
    pub modded: bool,
    pub warnings: Vec<Warning>,
}

/// Exponent of the (weight/power) -> time scaling. Empirically ~0.55-0.6 for 0-60.
pub const TIME_EXPONENT: f32 = 0.58;

/// Approximate hp-per-lb above which launch becomes traction limited, per drivetrain.
fn traction_limit(dt: Drivetrain) -> f32 {
    match dt {
        Drivetrain::FWD => 0.12,
        Drivetrain::RWD => 0.20,
        Drivetrain::AWD => 0.30,
        Drivetrain::FourWD => 0.22,
    }
}

fn soft_min(p: f32, lim: f32) -> f32 {
    (p.powi(-3) + lim.powi(-3)).powf(-1.0 / 3.0)
}

/// 0-60 scaled from the car's *stock* time (so the stock figure is reproduced exactly),
/// with diminishing returns once power exceeds what the drivetrain can put down.
pub fn estimate_zero_sixty(car: &Car, hp: f32, weight_lbs: f32, traction_mult: f32) -> f32 {
    let w0 = car.weight_lbs as f32;
    let lim = traction_limit(car.drivetrain);
    let eff0 = soft_min(car.hp as f32 / w0, lim);
    let eff1 = soft_min((hp.max(1.0)) / weight_lbs.max(500.0), lim * traction_mult.max(0.1));
    car.zero_sixty * (eff0 / eff1).powf(TIME_EXPONENT)
}

fn find_def<'a>(defs: &'a [ModDef], id: &str) -> Option<&'a ModDef> {
    defs.iter().find(|d| d.id == id)
}

pub fn evaluate(car: &Car, sels: &[ModSel], defs: &[ModDef]) -> Eval {
    let selected: HashSet<&str> = sels.iter().map(|s| s.id.as_str()).collect();
    let mut asp = car.aspiration;
    let mut hp = [car.hp as f32; 3];
    let mut tq = [car.torque as f32; 3];
    let mut weight = car.weight_lbs as f32;
    let mut zs_mult = [1.0f32; 2];
    let mut traction = 1.0f32;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut warn = |id: &str, text: String| warnings.push(Warning { mod_id: id.to_string(), text });
    let mut modded = false;

    for def in defs {
        let Some(sel) = sels.iter().find(|s| s.id == def.id) else { continue };

        if !def.fuels.is_empty() && !def.fuels.contains(&car.fuel) {
            warn(&def.id, format!("{}: not applicable to {} engines (ignored).", def.name, car.fuel.label()));
            continue;
        }
        if let Some(c) = def.conflicts.iter().find(|c| selected.contains(c.as_str()) && defs.iter().position(|d| &d.id == *c) < defs.iter().position(|d| d.id == def.id)) {
            let other = find_def(defs, c).map(|d| d.name.as_str()).unwrap_or(c);
            warn(&def.id, format!("{} conflicts with {other}; only {other} is counted.", def.name));
            continue;
        }

        let applicable = |e: &&ModEffect| e.applies_to.is_empty() || e.applies_to.contains(&asp);
        let eff = def.effects.iter().filter(applicable).find(|e| e.requires.iter().all(|r| selected.contains(r.as_str())));
        let Some(eff) = eff else {
            if def.effects.iter().any(|e| applicable(&e)) {
                let needs: Vec<&str> = def
                    .effects
                    .iter()
                    .filter(|e| applicable(e))
                    .flat_map(|e| e.requires.iter())
                    .filter_map(|r| find_def(defs, r).map(|d| d.name.as_str()))
                    .collect();
                warn(&def.id, format!("{}: requires {} (no effect counted).", def.name, needs.join(" or ")));
            } else if def.incompatible_note.is_empty() {
                warn(&def.id, format!("{}: has no effect on {} engines (ignored).", def.name, asp.label()));
            } else {
                warn(&def.id, format!("{} (ignored for this {} car).", def.incompatible_note.trim_end_matches('.'), asp.label()));
            }
            continue;
        };

        let value = match &def.slider {
            Some(s) if sel.value.is_finite() => sel.value.clamp(s.min, s.max),
            Some(s) => s.default,
            None => 0.0,
        };
        let u = if eff.unit_div.abs() > f32::EPSILON { value / eff.unit_div } else { 0.0 };
        let pct = [eff.min_pct + eff.unit_pct_min * u, 0.0, eff.max_pct + eff.unit_pct_max * u];
        let pct = [pct[0], (pct[0] + pct[2]) / 2.0, pct[2]];
        let flat = [eff.flat_hp + eff.unit_hp_min * u, 0.0, eff.flat_hp + eff.unit_hp_max * u];
        let flat = [flat[0], (flat[0] + flat[2]) / 2.0, flat[2]];

        for i in 0..3 {
            let before = hp[i];
            let after = (before * (1.0 + pct[i] / 100.0) + flat[i]).max(1.0);
            hp[i] = after;
            tq[i] = (tq[i] + (after - before) * (tq[i] / before.max(1.0)) * eff.torque_factor).max(1.0);
        }
        weight += eff.weight_lbs + eff.unit_lbs * u;
        zs_mult[0] *= 1.0 + eff.zs_pct_min / 100.0;
        zs_mult[1] *= 1.0 + eff.zs_pct_max / 100.0;
        traction *= 1.0 + eff.traction_pct / 100.0;
        if let Some(a) = eff.makes_aspiration {
            asp = a;
        }
        if !eff.note.is_empty() {
            warn(&def.id, format!("{}: {}", def.name, eff.note));
        }
        modded = true;
    }

    let weight = weight.max(500.0);
    let zero_sixty = if modded {
        let t = |h: f32, m: f32| estimate_zero_sixty(car, h, weight, traction) * m;
        let mid = (zs_mult[0] + zs_mult[1]) / 2.0;
        // quickest = most power and the most favourable direct modifier
        let fast = t(hp[2].max(hp[0]), zs_mult[0].min(zs_mult[1]));
        let slow = t(hp[0].min(hp[2]), zs_mult[0].max(zs_mult[1]));
        Range { min: fast.min(slow), typ: t(hp[1], mid), max: fast.max(slow) }
    } else {
        Range { min: car.zero_sixty, typ: car.zero_sixty, max: car.zero_sixty }
    };

    Eval {
        hp: Range::of(hp),
        torque: Range::of(tq),
        weight_lbs: weight,
        zero_sixty,
        aspiration: asp,
        modded,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::mod_defs;

    fn car(asp: Aspiration, hp: u16, dt: Drivetrain) -> Car {
        Car {
            id: "t".into(),
            make: "T".into(),
            parent_group: "G".into(),
            model: "M".into(),
            generation: "g".into(),
            year_start: 2010,
            year_end: 2012,
            trim: "x".into(),
            engine_code: "e".into(),
            displacement_l: 2.0,
            cylinders: 4,
            aspiration: asp,
            hp,
            hp_rpm: 6000,
            torque: hp,
            torque_rpm: 4000,
            zero_sixty: 5.0,
            zero_sixty_source: SourceKind::Est,
            weight_lbs: 3000,
            drivetrain: dt,
            transmissions: vec!["6MT".into()],
            redline_rpm: 7000,
            fuel: Fuel::Gas,
            confidence: Confidence::Estimated,
        }
    }
    fn sel(id: &str, v: f32) -> ModSel {
        ModSel { id: id.into(), value: v }
    }

    #[test]
    fn no_mods_reproduces_stock() {
        let c = car(Aspiration::NA, 300, Drivetrain::RWD);
        let e = evaluate(&c, &[], mod_defs());
        assert_eq!(e.hp.typ, 300.0);
        assert_eq!(e.zero_sixty.typ, 5.0);
        assert!(!e.modded);
    }

    #[test]
    fn intake_depends_on_aspiration() {
        let na = evaluate(&car(Aspiration::NA, 300, Drivetrain::RWD), &[sel("intake", 0.0)], mod_defs());
        let gain_na = na.hp.typ - 300.0;
        assert!((4.0..=10.0).contains(&gain_na), "{gain_na}");
        let tb = car(Aspiration::Turbo, 300, Drivetrain::RWD);
        let plain = evaluate(&tb, &[sel("intake", 0.0)], mod_defs()).hp.typ - 300.0;
        let tuned = evaluate(&tb, &[sel("intake", 0.0), sel("tune1", 0.0)], mod_defs());
        let tune_only = evaluate(&tb, &[sel("tune1", 0.0)], mod_defs());
        assert!(tuned.hp.typ - tune_only.hp.typ > plain, "intake should gain more once tuned");
    }

    #[test]
    fn tune_is_large_on_turbo_small_on_na() {
        let t = evaluate(&car(Aspiration::Turbo, 300, Drivetrain::AWD), &[sel("tune1", 0.0)], mod_defs());
        let n = evaluate(&car(Aspiration::NA, 300, Drivetrain::AWD), &[sel("tune1", 0.0)], mod_defs());
        assert!(t.hp.typ - 300.0 > 40.0);
        assert!(n.hp.typ - 300.0 < 10.0);
    }

    #[test]
    fn incompatible_mods_warn_and_do_nothing() {
        let e = evaluate(&car(Aspiration::NA, 300, Drivetrain::RWD), &[sel("downpipe", 0.0), sel("pulley", 0.0)], mod_defs());
        assert_eq!(e.hp.typ, 300.0);
        assert_eq!(e.warnings.len(), 2);
        assert!(e.warnings.iter().any(|w| w.text.contains("turbo")));
        let t = evaluate(&car(Aspiration::Turbo, 300, Drivetrain::RWD), &[sel("pulley", 0.0)], mod_defs());
        assert_eq!(t.hp.typ, 300.0);
        assert!(!t.warnings.is_empty());
    }

    #[test]
    fn stacking_is_diminishing_and_ranged() {
        let c = car(Aspiration::NA, 300, Drivetrain::RWD);
        let e = evaluate(&c, &[sel("intake", 0.0), sel("headers", 0.0), sel("catback", 0.0), sel("tune1", 0.0)], mod_defs());
        assert!(e.hp.min < e.hp.typ && e.hp.typ < e.hp.max);
        let sum: f32 = [("intake", 3.0), ("headers", 6.0), ("catback", 2.0), ("tune1", 3.0)].iter().map(|x| x.1).sum();
        assert!(e.hp.max <= 300.0 * (1.0 + sum / 100.0) * 1.01);
    }

    #[test]
    fn altitude_na_vs_turbo() {
        let na = evaluate(&car(Aspiration::NA, 300, Drivetrain::RWD), &[sel("altitude", 5000.0)], mod_defs());
        assert!((na.hp.typ - 255.0).abs() < 1.5, "{}", na.hp.typ); // ~15 % loss
        let tb = evaluate(&car(Aspiration::Turbo, 300, Drivetrain::RWD), &[sel("altitude", 5000.0)], mod_defs());
        assert!(tb.hp.typ > na.hp.typ + 20.0);
        assert!(na.zero_sixty.typ > 5.0);
    }

    #[test]
    fn altitude_slider_is_clamped() {
        let a = evaluate(&car(Aspiration::NA, 300, Drivetrain::RWD), &[sel("altitude", 1.0e9)], mod_defs());
        assert!(a.hp.typ > 100.0);
        let n = evaluate(&car(Aspiration::NA, 300, Drivetrain::RWD), &[sel("altitude", f32::NAN)], mod_defs());
        assert!(n.hp.typ.is_finite());
    }

    #[test]
    fn nitrous_scales_with_shot() {
        let c = car(Aspiration::NA, 300, Drivetrain::RWD);
        let s50 = evaluate(&c, &[sel("nitrous", 50.0)], mod_defs()).hp.typ;
        let s150 = evaluate(&c, &[sel("nitrous", 150.0)], mod_defs()).hp.typ;
        assert!((s50 - 347.5).abs() < 1.0, "{s50}");
        assert!((s150 - 442.5).abs() < 1.0, "{s150}");
    }

    #[test]
    fn weight_mods_affect_time_and_weight() {
        let c = car(Aspiration::NA, 300, Drivetrain::RWD);
        let light = evaluate(&c, &[sel("weight_red", 200.0)], mod_defs());
        assert_eq!(light.weight_lbs, 2800.0);
        assert!(light.zero_sixty.typ < 5.0);
        let heavy = evaluate(&c, &[sel("passengers", 340.0), sel("roof_rack", 0.0)], mod_defs());
        assert_eq!(heavy.weight_lbs, 3000.0 + 340.0 + 45.0);
        assert!(heavy.zero_sixty.typ > 5.0);
        assert_eq!(heavy.hp.typ, 300.0);
    }

    #[test]
    fn zero_sixty_scales_from_stock() {
        // Doubling power on a non-traction-limited AWD car should cut time by roughly 2^-0.58.
        let mut c = car(Aspiration::NA, 100, Drivetrain::AWD);
        c.weight_lbs = 4000;
        let t = estimate_zero_sixty(&c, 200.0, 4000.0, 1.0);
        let ideal = 5.0 * 0.5f32.powf(TIME_EXPONENT);
        assert!((t - ideal).abs() < 0.35, "{t} vs {ideal}");
        // Traction-limited FWD gains much less from the same power.
        let mut f = car(Aspiration::NA, 250, Drivetrain::FWD);
        f.weight_lbs = 3000;
        let tf = estimate_zero_sixty(&f, 500.0, 3000.0, 1.0);
        assert!(tf > 5.0 * 0.5f32.powf(TIME_EXPONENT) + 0.5);
        // Stock reproduces exactly.
        assert!((estimate_zero_sixty(&f, 250.0, 3000.0, 1.0) - 5.0).abs() < 1e-4);
    }

    #[test]
    fn tires_only_change_time() {
        let c = car(Aspiration::NA, 500, Drivetrain::RWD);
        let e = evaluate(&c, &[sel("tires", 0.0)], mod_defs());
        assert_eq!(e.hp.typ, 500.0);
        assert!(e.zero_sixty.typ < 5.0);
    }

    #[test]
    fn requirements_and_conflicts() {
        let c = car(Aspiration::Turbo, 300, Drivetrain::AWD);
        let with = evaluate(&c, &[sel("tune2", 0.0), sel("downpipe", 0.0)], mod_defs());
        let without = evaluate(&c, &[sel("tune2", 0.0)], mod_defs());
        assert!(with.hp.typ > without.hp.typ);
        assert!(without.warnings.iter().any(|w| w.text.contains("downpipe")));
        let both = evaluate(&c, &[sel("tune1", 0.0), sel("tune2", 0.0)], mod_defs());
        assert!(both.warnings.iter().any(|w| w.text.contains("conflicts")));
    }

    #[test]
    fn turbo_kit_changes_aspiration_for_later_mods() {
        let c = car(Aspiration::NA, 200, Drivetrain::RWD);
        let e = evaluate(&c, &[sel("turbo_kit", 0.0), sel("downpipe", 0.0)], mod_defs());
        assert_eq!(e.aspiration, Aspiration::Turbo);
        assert!(e.hp.typ > 300.0);
    }

    #[test]
    fn custom_hp_can_be_negative() {
        let c = car(Aspiration::NA, 300, Drivetrain::RWD);
        let e = evaluate(&c, &[sel("custom", -40.0)], mod_defs());
        assert!((e.hp.typ - 260.0).abs() < 0.01);
        assert!(e.zero_sixty.typ > 5.0);
    }

    #[test]
    fn e85_not_for_diesel() {
        let mut c = car(Aspiration::Turbo, 300, Drivetrain::RWD);
        c.fuel = Fuel::Diesel;
        let e = evaluate(&c, &[sel("e85", 0.0)], mod_defs());
        assert_eq!(e.hp.typ, 300.0);
        assert!(!e.warnings.is_empty());
    }
}
