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
}
