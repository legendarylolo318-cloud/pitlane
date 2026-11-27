//! Mod math: stacking, aspiration rules, environment, calibrated 0-60 estimate.
//! All numbers come from `ModDef` data (data/mods.ron); nothing is hardcoded per mod.

use crate::model::*;
use serde::{Deserialize, Serialize};

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

/// Why a selected mod is not counted (None = it applies).
fn skip_reason<'a>(def: &'a ModDef, car: &Car, asp: Aspiration, defs: &'a [ModDef], active: &[&str]) -> Option<Skip<'a>> {
    if !def.fuels.is_empty() && !def.fuels.contains(&car.fuel) {
        return Some(Skip::Fuel);
    }
    // Of two conflicting mods the one defined first wins (both are only ever "active" if selected).
    if let Some(c) = def.conflicts.iter().find(|c| active.contains(&c.as_str())) {
        return Some(Skip::Conflict(find_def(defs, c).map(|d| d.name.as_str()).unwrap_or(c)));
    }
    if !def.effects.iter().any(|e| e.applies_to.is_empty() || e.applies_to.contains(&asp)) {
        return Some(Skip::Aspiration);
    }
    None
}

enum Skip<'a> {
    Fuel,
    Conflict(&'a str),
    Aspiration,
}

/// Applies the selected mods to `car`.
///
/// Rules:
/// * mods are applied in `defs` order; percentage effects compound on the running *engine* power,
///   so stacking has diminishing returns;
/// * flat effects (nitrous shot, custom +/- hp) are added on top at the end and are never scaled by
///   other mods' percentages (altitude does not shrink a nitrous shot, a tune does not grow it);
/// * a mod that is ignored (wrong fuel, wrong aspiration, losing a conflict) cannot satisfy another
///   mod's `requires`;
/// * aspiration changes (`makes_aspiration`, e.g. a turbo kit) affect every later mod.
pub fn evaluate(car: &Car, sels: &[ModSel], defs: &[ModDef]) -> Eval {
    let picked = |id: &str| sels.iter().find(|s| s.id == id);

    // Pass 1: which selected mods actually take part (independent of `requires`).
    let mut active: Vec<&str> = Vec::new();
    let mut asp = car.aspiration;
    for def in defs.iter().filter(|d| picked(&d.id).is_some()) {
        if skip_reason(def, car, asp, defs, &active).is_none() {
            active.push(&def.id);
            let first = def.effects.iter().find(|e| e.applies_to.is_empty() || e.applies_to.contains(&asp));
            if let Some(a) = first.and_then(|e| e.makes_aspiration) {
                asp = a;
            }
        }
    }

    // Pass 2: apply effects.
    let mut asp = car.aspiration;
    let mut hp = [car.hp as f32; 3];
    let mut tq = [car.torque as f32; 3];
    let mut flat_hp = [0.0f32; 3];
    let mut flat_tq = [0.0f32; 3];
    let mut weight = car.weight_lbs as f32;
    let mut zs_mult = [1.0f32; 2];
    let mut traction = 1.0f32;
    let mut warnings: Vec<Warning> = Vec::new();
    let mut warn = |id: &str, text: String| warnings.push(Warning { mod_id: id.to_string(), text });
    let mut modded = false;
    let mut seen: Vec<&str> = Vec::new();

    for def in defs {
        let Some(sel) = picked(&def.id) else { continue };
        match skip_reason(def, car, asp, defs, &seen) {
            Some(Skip::Fuel) => {
                warn(&def.id, format!("{}: not applicable to {} engines (ignored).", def.name, car.fuel.label()));
                continue;
            }
            Some(Skip::Conflict(other)) => {
                warn(&def.id, format!("{} conflicts with {other}; only {other} is counted.", def.name));
                continue;
            }
            Some(Skip::Aspiration) => {
                if def.incompatible_note.is_empty() {
                    warn(&def.id, format!("{}: has no effect on {} engines (ignored).", def.name, asp.label()));
                } else {
                    warn(&def.id, format!("{} (ignored for this {} car).", def.incompatible_note.trim_end_matches('.'), asp.label()));
                }
                continue;
            }
            None => seen.push(&def.id),
        }

        let applicable = |e: &&ModEffect| e.applies_to.is_empty() || e.applies_to.contains(&asp);
        let eff = def.effects.iter().filter(applicable).find(|e| e.requires.iter().all(|r| active.contains(&r.as_str())));
        let Some(eff) = eff else {
            let needs: Vec<&str> = def
                .effects
                .iter()
                .filter(applicable)
                .flat_map(|e| e.requires.iter())
                .filter_map(|r| find_def(defs, r).map(|d| d.name.as_str()))
                .collect();
            warn(&def.id, format!("{}: requires {} (no effect counted).", def.name, needs.join(" or ")));
            continue;
        };

        let value = match &def.slider {
            Some(s) if sel.value.is_finite() => sel.value.clamp(s.min, s.max),
            Some(s) => s.default,
            None => 0.0,
        };
        let u = if eff.unit_div.abs() > f32::EPSILON { value / eff.unit_div } else { 0.0 };
        let lo_pct = eff.min_pct + eff.unit_pct_min * u;
        let hi_pct = eff.max_pct + eff.unit_pct_max * u;
        let pct = [lo_pct, (lo_pct + hi_pct) / 2.0, hi_pct];
        let lo_flat = eff.flat_hp + eff.unit_hp_min * u;
        let hi_flat = eff.flat_hp + eff.unit_hp_max * u;
        let flat = [lo_flat, (lo_flat + hi_flat) / 2.0, hi_flat];

        for i in 0..3 {
            if pct[i] != 0.0 {
                let before = hp[i];
                let after = (before * (1.0 + pct[i] / 100.0)).max(1.0);
                hp[i] = after;
                tq[i] = (tq[i] * (1.0 + (after / before - 1.0) * eff.torque_factor)).max(1.0);
            }
            if flat[i] != 0.0 {
                // torque follows the engine's current torque/power ratio
                flat_hp[i] += flat[i];
                flat_tq[i] += flat[i] * (tq[i] / hp[i].max(1.0)) * eff.torque_factor;
            }
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

    let hp: [f32; 3] = std::array::from_fn(|i| (hp[i] + flat_hp[i]).max(1.0));
    let tq: [f32; 3] = std::array::from_fn(|i| (tq[i] + flat_tq[i]).max(1.0));
    let weight = weight.max(500.0);
    let zero_sixty = if modded {
        let t = |h: f32, m: f32| (estimate_zero_sixty(car, h, weight, traction) * m).clamp(ZERO_SIXTY_MIN, ZERO_SIXTY_MAX);
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

/// Bounds for the estimated 0-60 (a de-tuned car can otherwise produce absurd times).
pub const ZERO_SIXTY_MIN: f32 = 1.5;
pub const ZERO_SIXTY_MAX: f32 = 60.0;

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
            transmission: "6MT".into(),
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

    fn all_ids(kind: Option<ModKind>) -> Vec<ModSel> {
        mod_defs()
            .iter()
            .filter(|d| kind.map(|k| d.kind == k).unwrap_or(true))
            .map(|d| sel(&d.id, d.slider.as_ref().map(|s| s.max).unwrap_or(0.0)))
            .collect()
    }
    fn finite(e: &Eval) -> bool {
        [e.hp.min, e.hp.typ, e.hp.max, e.torque.min, e.torque.typ, e.torque.max, e.zero_sixty.min, e.zero_sixty.typ, e.zero_sixty.max, e.weight_lbs]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
    }

    #[test]
    fn everything_stacked_stays_sane_for_every_aspiration() {
        for asp in Aspiration::ALL {
            for dt in Drivetrain::ALL {
                let mut c = car(asp, 300, dt);
                if asp == Aspiration::EV {
                    c.fuel = Fuel::Electric;
                    c.cylinders = 0;
                    c.displacement_l = 0.0;
                }
                let e = evaluate(&c, &all_ids(Some(ModKind::Gain)), mod_defs());
                assert!(finite(&e), "{asp:?} {dt:?}: {e:?}");
                assert!(e.hp.min <= e.hp.typ && e.hp.typ <= e.hp.max, "{asp:?}: {:?}", e.hp);
                assert!(e.zero_sixty.min <= e.zero_sixty.typ && e.zero_sixty.typ <= e.zero_sixty.max);
                // every gain mod at once, sliders maxed (turbo kit + stage 2 + E85 + 150 shot ...):
                // big but bounded, no runaway compounding
                assert!(e.hp.max < 300.0 * 4.0, "{asp:?}: {:?}", e.hp);
                let all = evaluate(&c, &all_ids(None), mod_defs());
                assert!(finite(&all), "{asp:?} everything: {all:?}");
            }
        }
    }

    #[test]
    fn ev_only_takes_weight_tires_and_conditions() {
        let mut c = car(Aspiration::EV, 480, Drivetrain::AWD);
        c.fuel = Fuel::Electric;
        let e = evaluate(&c, &[sel("intake", 0.0), sel("tune1", 0.0), sel("nitrous", 100.0), sel("downpipe", 0.0), sel("e85", 0.0)], mod_defs());
        assert_eq!(e.hp.typ, 480.0);
        assert!(!e.modded);
        assert_eq!(e.warnings.len(), 5);
        let e = evaluate(&c, &[sel("altitude", 10000.0)], mod_defs());
        assert_eq!(e.hp.typ, 480.0, "EVs do not lose power with altitude");
        let e = evaluate(&c, &[sel("weight_red", 300.0), sel("tires", 0.0)], mod_defs());
        assert_eq!(e.weight_lbs, 2700.0);
        assert!(e.zero_sixty.typ < 5.0);
    }

    #[test]
    fn hybrid_behaves_like_na_for_bolt_ons() {
        let mut h = car(Aspiration::Hybrid, 300, Drivetrain::AWD);
        h.fuel = Fuel::Hybrid;
        let n = car(Aspiration::NA, 300, Drivetrain::AWD);
        let mods = [sel("intake", 0.0), sel("headers", 0.0), sel("altitude", 5000.0)];
        assert_eq!(evaluate(&h, &mods, mod_defs()).hp, evaluate(&n, &mods, mod_defs()).hp);
        // E85 needs a gasoline/flex fuel system
        assert_eq!(evaluate(&h, &[sel("e85", 0.0)], mod_defs()).hp.typ, 300.0);
    }

    #[test]
    fn negative_results_are_clamped_not_nonsense() {
        let mut c = car(Aspiration::NA, 55, Drivetrain::FWD);
        c.weight_lbs = 1650;
        c.zero_sixty = 14.0;
        let e = evaluate(&c, &[sel("custom", -200.0), sel("worn", 0.0), sel("altitude", 14000.0), sel("cargo", 1000.0)], mod_defs());
        assert!(finite(&e), "{e:?}");
        assert_eq!(e.hp.typ, 1.0);
        assert!(e.zero_sixty.typ <= ZERO_SIXTY_MAX);
    }

    #[test]
    fn extreme_altitude() {
        let na = evaluate(&car(Aspiration::NA, 300, Drivetrain::RWD), &[sel("altitude", 14000.0)], mod_defs());
        // 2.7-3.3 % per 1000 ft compounding once -> 38-46 % loss
        assert!(na.hp.min > 300.0 * 0.5 && na.hp.max < 300.0 * 0.65, "{:?}", na.hp);
        let tb = evaluate(&car(Aspiration::Turbo, 300, Drivetrain::RWD), &[sel("altitude", 14000.0)], mod_defs());
        assert!(tb.hp.typ > 300.0 * 0.75 && tb.hp.typ < 300.0, "{:?}", tb.hp);
        let sc = evaluate(&car(Aspiration::Supercharged, 300, Drivetrain::RWD), &[sel("altitude", 14000.0)], mod_defs());
        assert!(sc.hp.typ > na.hp.typ && sc.hp.typ < tb.hp.typ);
    }

    #[test]
    fn flat_gains_are_not_scaled_by_later_percentages() {
        let c = car(Aspiration::NA, 300, Drivetrain::RWD);
        let shot = evaluate(&c, &[sel("nitrous", 100.0)], mod_defs()).hp.typ - 300.0;
        let alt = evaluate(&c, &[sel("altitude", 5000.0)], mod_defs()).hp.typ;
        let both = evaluate(&c, &[sel("nitrous", 100.0), sel("altitude", 5000.0)], mod_defs()).hp.typ;
        assert!((both - (alt + shot)).abs() < 0.01, "{both} vs {alt} + {shot}");
        let custom = evaluate(&c, &[sel("custom", 25.0), sel("worn", 0.0)], mod_defs()).hp.typ;
        let worn = evaluate(&c, &[sel("worn", 0.0)], mod_defs()).hp.typ;
        assert!((custom - (worn + 25.0)).abs() < 0.01);
    }

    #[test]
    fn ignored_mods_do_not_satisfy_requirements() {
        let c = car(Aspiration::Turbo, 300, Drivetrain::AWD);
        // tune2 loses the conflict to tune1, so the downpipe must use its tune1 effect
        let both = evaluate(&c, &[sel("tune1", 0.0), sel("tune2", 0.0), sel("downpipe", 0.0)], mod_defs());
        let t1 = evaluate(&c, &[sel("tune1", 0.0), sel("downpipe", 0.0)], mod_defs());
        assert_eq!(both.hp, t1.hp);
        // an NA car with tune2 + headers: headers count, so tune2 uses its "with headers" effect;
        // a downpipe on that NA car is ignored and must not unlock anything
        let na = car(Aspiration::NA, 300, Drivetrain::RWD);
        let a = evaluate(&na, &[sel("tune2", 0.0), sel("downpipe", 0.0)], mod_defs());
        let b = evaluate(&na, &[sel("tune2", 0.0)], mod_defs());
        assert_eq!(a.hp, b.hp);
    }

    #[test]
    fn order_of_selection_does_not_matter() {
        let c = car(Aspiration::Turbo, 300, Drivetrain::AWD);
        let a = evaluate(&c, &[sel("downpipe", 0.0), sel("tune2", 0.0), sel("intake", 0.0), sel("nitrous", 75.0)], mod_defs());
        let b = evaluate(&c, &[sel("nitrous", 75.0), sel("intake", 0.0), sel("tune2", 0.0), sel("downpipe", 0.0)], mod_defs());
        assert_eq!(a.hp, b.hp);
        assert_eq!(a.zero_sixty, b.zero_sixty);
    }

    #[test]
    fn modded_zero_sixty_is_calibrated_to_stock() {
        // a mod with zero net effect must give back exactly the stock time
        let c = car(Aspiration::Turbo, 300, Drivetrain::AWD);
        let e = evaluate(&c, &[sel("custom", 0.0)], mod_defs());
        assert!(e.modded);
        assert!((e.zero_sixty.typ - c.zero_sixty).abs() < 1e-4);
        // more power -> quicker, more weight -> slower, monotonic
        let mut last = c.zero_sixty;
        for hp in [25.0, 50.0, 100.0, 200.0] {
            let t = evaluate(&c, &[sel("custom", hp)], mod_defs()).zero_sixty.typ;
            assert!(t < last, "{hp}: {t} !< {last}");
            last = t;
        }
    }

    #[test]
    fn turbo_kit_on_na_unlocks_turbo_only_parts() {
        let c = car(Aspiration::NA, 200, Drivetrain::RWD);
        let e = evaluate(&c, &[sel("turbo_kit", 0.0), sel("intercooler", 0.0), sel("pulley", 0.0)], mod_defs());
        assert!(e.warnings.iter().any(|w| w.mod_id == "pulley"), "pulley is still not valid on a turbo car");
        assert!(!e.warnings.iter().any(|w| w.mod_id == "intercooler"), "{:?}", e.warnings);
    }
}
