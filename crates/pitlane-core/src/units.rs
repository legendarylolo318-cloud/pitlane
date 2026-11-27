//! Unit conversion and drivetrain-loss handling.

use crate::model::{Aspiration, Car, Drivetrain};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UnitSystem {
    #[default]
    Imperial,
    Metric,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PowerUnit {
    #[default]
    Hp,
    Kw,
    Ps,
}

/// Drivetrain losses in percent. Configurable in the UI settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LossCfg {
    pub two_wd_manual: f32,
    pub two_wd_auto: f32,
    pub awd_manual: f32,
    pub awd_auto: f32,
    pub ev: f32,
}

impl Default for LossCfg {
    fn default() -> Self {
        Self { two_wd_manual: 12.0, two_wd_auto: 15.0, awd_manual: 18.0, awd_auto: 20.0, ev: 8.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub system: UnitSystem,
    pub power: PowerUnit,
    /// Show estimated wheel power/torque instead of crank figures.
    pub wheel: bool,
    pub loss: LossCfg,
}

pub const KW_PER_HP: f32 = 0.745_699_9;
pub const PS_PER_HP: f32 = 1.013_869_7;
pub const NM_PER_LBFT: f32 = 1.355_818;
pub const KG_PER_LB: f32 = 0.453_592_37;

pub fn power_from_hp(p: PowerUnit, hp: f32) -> f32 {
    match p {
        PowerUnit::Hp => hp,
        PowerUnit::Kw => hp * KW_PER_HP,
        PowerUnit::Ps => hp * PS_PER_HP,
    }
}

/// Inverse of [`power_from_hp`].
pub fn hp_from_power(p: PowerUnit, v: f32) -> f32 {
    match p {
        PowerUnit::Hp => v,
        PowerUnit::Kw => v / KW_PER_HP,
        PowerUnit::Ps => v / PS_PER_HP,
    }
}

pub fn power_label(p: PowerUnit) -> &'static str {
    match p {
        PowerUnit::Hp => "hp",
        PowerUnit::Kw => "kW",
        PowerUnit::Ps => "PS",
    }
}

pub fn torque_from_lbft(s: UnitSystem, v: f32) -> f32 {
    match s {
        UnitSystem::Imperial => v,
        UnitSystem::Metric => v * NM_PER_LBFT,
    }
}

/// Inverse of [`torque_from_lbft`].
pub fn lbft_from_torque(s: UnitSystem, v: f32) -> f32 {
    match s {
        UnitSystem::Imperial => v,
        UnitSystem::Metric => v / NM_PER_LBFT,
    }
}

pub fn torque_label(s: UnitSystem) -> &'static str {
    match s {
        UnitSystem::Imperial => "lb-ft",
        UnitSystem::Metric => "Nm",
    }
}

pub fn weight_from_lbs(s: UnitSystem, v: f32) -> f32 {
    match s {
        UnitSystem::Imperial => v,
        UnitSystem::Metric => v * KG_PER_LB,
    }
}

/// Inverse of [`weight_from_lbs`].
pub fn lbs_from_weight(s: UnitSystem, v: f32) -> f32 {
    match s {
        UnitSystem::Imperial => v,
        UnitSystem::Metric => v / KG_PER_LB,
    }
}

pub fn weight_label(s: UnitSystem) -> &'static str {
    match s {
        UnitSystem::Imperial => "lb",
        UnitSystem::Metric => "kg",
    }
}

/// 0-100 km/h is about 2.1 mph more than 0-60 mph, which costs a little more time on slower cars.
/// This is an approximation (documented in README), not a measurement.
pub fn zero_sixty_to_100kmh(t60: f32) -> f32 {
    t60 * 1.04 + 0.05
}

/// Inverse of [`zero_sixty_to_100kmh`].
pub fn zero_sixty_from_100kmh(t100: f32) -> f32 {
    (t100 - 0.05) / 1.04
}

pub fn accel_from_60(s: UnitSystem, t60: f32) -> f32 {
    match s {
        UnitSystem::Imperial => t60,
        UnitSystem::Metric => zero_sixty_to_100kmh(t60),
    }
}

pub fn accel_label(s: UnitSystem) -> &'static str {
    match s {
        UnitSystem::Imperial => "0-60 mph",
        UnitSystem::Metric => "0-100 km/h",
    }
}

/// Power per ton in the chosen power unit (imperial: per 2000 lb short ton; metric: per tonne).
pub fn power_per_ton(s: Settings, hp: f32, weight_lbs: f32) -> f32 {
    if weight_lbs <= 0.0 {
        return 0.0;
    }
    match s.system {
        UnitSystem::Imperial => power_from_hp(s.power, hp) / weight_lbs * 2000.0,
        UnitSystem::Metric => power_from_hp(s.power, hp) / (weight_lbs * KG_PER_LB) * 1000.0,
    }
}

/// Weight per unit of power (lb/hp, or kg per chosen power unit).
pub fn weight_per_power(s: Settings, hp: f32, weight_lbs: f32) -> f32 {
    let p = power_from_hp(s.power, hp);
    if p <= 0.0 {
        return 0.0;
    }
    weight_from_lbs(s.system, weight_lbs) / p
}

/// Multiplier (<1) turning crank power/torque into estimated wheel power/torque.
pub fn wheel_factor(car: &Car, asp: Aspiration, loss: &LossCfg) -> f32 {
    let pct = if asp == Aspiration::EV {
        loss.ev
    } else {
        let awd = matches!(car.drivetrain, Drivetrain::AWD | Drivetrain::FourWD);
        match (awd, car.is_auto()) {
            (false, false) => loss.two_wd_manual,
            (false, true) => loss.two_wd_auto,
            (true, false) => loss.awd_manual,
            (true, true) => loss.awd_auto,
        }
    };
    (1.0 - pct.clamp(0.0, 60.0) / 100.0).max(0.4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert!((power_from_hp(PowerUnit::Kw, 100.0) - 74.57).abs() < 0.01);
        assert!((power_from_hp(PowerUnit::Ps, 100.0) - 101.39).abs() < 0.01);
        assert!((torque_from_lbft(UnitSystem::Metric, 100.0) - 135.58).abs() < 0.01);
        assert!((weight_from_lbs(UnitSystem::Metric, 3000.0) - 1360.8).abs() < 0.1);
        assert!(zero_sixty_to_100kmh(5.0) > 5.0 && zero_sixty_to_100kmh(5.0) < 5.5);
    }

    #[test]
    fn power_to_weight() {
        let s = Settings::default();
        assert!((power_per_ton(s, 300.0, 3000.0) - 200.0).abs() < 1e-3);
        assert!((weight_per_power(s, 300.0, 3000.0) - 10.0).abs() < 1e-3);
        let m = Settings { system: UnitSystem::Metric, power: PowerUnit::Kw, ..Settings::default() };
        let v = power_per_ton(m, 300.0, 3000.0);
        assert!((v - 164.4).abs() < 0.5, "{v}");
    }

    #[test]
    fn conversions_round_trip() {
        for v in [1.0f32, 55.0, 300.0, 1064.0, 1500.0] {
            for p in [PowerUnit::Hp, PowerUnit::Kw, PowerUnit::Ps] {
                let back = hp_from_power(p, power_from_hp(p, v));
                assert!((back - v).abs() < v * 1e-5, "{p:?} {v} -> {back}");
            }
            for s in [UnitSystem::Imperial, UnitSystem::Metric] {
                assert!((lbft_from_torque(s, torque_from_lbft(s, v)) - v).abs() < v * 1e-5);
                assert!((lbs_from_weight(s, weight_from_lbs(s, v)) - v).abs() < v * 1e-5);
            }
        }
        for t in [2.3f32, 4.0, 9.5, 17.0] {
            assert!((zero_sixty_from_100kmh(zero_sixty_to_100kmh(t)) - t).abs() < 1e-4);
        }
        // published pairs: 1 kW = 1.341 hp, 1 PS = 0.9863 hp, 100 lb-ft = 135.6 Nm
        assert!((hp_from_power(PowerUnit::Kw, 1.0) - 1.341).abs() < 0.001);
        assert!((hp_from_power(PowerUnit::Ps, 1.0) - 0.9863).abs() < 0.0001);
        // 0-100 km/h (62.1 mph) is always a little slower than 0-60
        assert!(zero_sixty_to_100kmh(2.5) > 2.5 && zero_sixty_to_100kmh(2.5) < 2.8);
    }

    fn test_car(dt: Drivetrain, tx: &'static str) -> Car {
        Car {
            id: "t".into(),
            make: "T".into(),
            parent_group: "G".into(),
            model: "M".into(),
            generation: "g".into(),
            year_start: 2010,
            year_end: 2010,
            trim: "x".into(),
            engine_code: "e".into(),
            displacement_l: 2.0,
            cylinders: 4,
            aspiration: Aspiration::NA,
            hp: 300,
            hp_rpm: 0,
            torque: 300,
            torque_rpm: 0,
            zero_sixty: 5.0,
            zero_sixty_source: crate::model::SourceKind::Est,
            weight_lbs: 3000,
            drivetrain: dt,
            transmission: tx.into(),
            redline_rpm: 0,
            fuel: crate::model::Fuel::Gas,
            confidence: crate::model::Confidence::Estimated,
        }
    }

    #[test]
    fn wheel_power_loss_by_layout() {
        let l = LossCfg::default();
        let f = |dt, tx| wheel_factor(&test_car(dt, tx), Aspiration::NA, &l);
        assert!((f(Drivetrain::RWD, "6MT/8AT") - 0.88).abs() < 1e-6, "first listed transmission decides");
        assert!((f(Drivetrain::FWD, "8AT/6MT") - 0.85).abs() < 1e-6);
        assert!((f(Drivetrain::AWD, "6MT") - 0.82).abs() < 1e-6);
        assert!((f(Drivetrain::FourWD, "10AT") - 0.80).abs() < 1e-6);
        assert!((f(Drivetrain::AWD, "CVT") - 0.80).abs() < 1e-6);
        assert!((f(Drivetrain::RWD, "7DCT") - 0.85).abs() < 1e-6, "DCT counts as automatic");
        assert!((f(Drivetrain::RWD, "6SMT") - 0.88).abs() < 1e-6, "sequential manual counts as manual");
        // EVs use the EV loss whatever the layout; a turbo kit does not change the layout loss
        assert!((wheel_factor(&test_car(Drivetrain::AWD, "1AT"), Aspiration::EV, &l) - 0.92).abs() < 1e-6);
        // silly user settings are clamped
        let bad = LossCfg { two_wd_manual: 500.0, ..l };
        assert!(wheel_factor(&test_car(Drivetrain::RWD, "6MT"), Aspiration::NA, &bad) >= 0.4);
    }
}
