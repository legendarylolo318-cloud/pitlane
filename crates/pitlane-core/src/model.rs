//! Plain data types shared by the library and `build.rs` (which `#[path]`-includes this file).
//! Must only depend on `serde`.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Text field of a [`Car`]. Bundled cars borrow it from the embedded, interned string table
/// (no allocation); user overrides / custom cars own it.
pub type Text = Cow<'static, str>;

pub const YEAR_MIN: u16 = 1990;
pub const YEAR_MAX: u16 = 2026;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Aspiration {
    NA,
    Turbo,
    Supercharged,
    Hybrid,
    EV,
}

impl Aspiration {
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub const ALL: [Aspiration; 5] = [
        Aspiration::NA,
        Aspiration::Turbo,
        Aspiration::Supercharged,
        Aspiration::Hybrid,
        Aspiration::EV,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Aspiration::NA => "NA",
            Aspiration::Turbo => "Turbo",
            Aspiration::Supercharged => "Supercharged",
            Aspiration::Hybrid => "Hybrid",
            Aspiration::EV => "EV",
        }
    }
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Drivetrain {
    FWD,
    RWD,
    AWD,
    FourWD,
}

impl Drivetrain {
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub const ALL: [Drivetrain; 4] = [Drivetrain::FWD, Drivetrain::RWD, Drivetrain::AWD, Drivetrain::FourWD];
    pub fn label(self) -> &'static str {
        match self {
            Drivetrain::FWD => "FWD",
            Drivetrain::RWD => "RWD",
            Drivetrain::AWD => "AWD",
            Drivetrain::FourWD => "4WD",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Fuel {
    #[default]
    Gas,
    Diesel,
    Hybrid,
    Electric,
    Flex,
}

impl Fuel {
    pub const ALL: [Fuel; 5] = [Fuel::Gas, Fuel::Diesel, Fuel::Hybrid, Fuel::Electric, Fuel::Flex];
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub fn label(self) -> &'static str {
        match self {
            Fuel::Gas => "Gasoline",
            Fuel::Diesel => "Diesel",
            Fuel::Hybrid => "Gas hybrid",
            Fuel::Electric => "Electric",
            Fuel::Flex => "Gas / E85 flex",
        }
    }
}

/// Where the 0-60 figure came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum SourceKind {
    Mfr,
    Mag,
    #[default]
    Est,
}

impl SourceKind {
    pub const ALL: [SourceKind; 3] = [SourceKind::Mfr, SourceKind::Mag, SourceKind::Est];
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Mfr => "manufacturer",
            SourceKind::Mag => "magazine",
            SourceKind::Est => "estimated",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Confidence {
    Verified,
    #[default]
    Estimated,
}

impl Confidence {
    pub const ALL: [Confidence; 2] = [Confidence::Verified, Confidence::Estimated];
    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.get(v as usize).copied()
    }
    pub fn label(self) -> &'static str {
        match self {
            Confidence::Verified => "verified",
            Confidence::Estimated => "estimated",
        }
    }
}

/// One fully-expanded car (one trim/engine of one generation).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Car {
    pub id: Text,
    pub make: Text,
    pub parent_group: Text,
    pub model: Text,
    pub generation: Text,
    pub year_start: u16,
    pub year_end: u16,
    pub trim: Text,
    pub engine_code: Text,
    pub displacement_l: f32,
    pub cylinders: u8,
    pub aspiration: Aspiration,
    pub hp: u16,
    pub hp_rpm: u16,
    pub torque: u16,
    pub torque_rpm: u16,
    pub zero_sixty: f32,
    pub zero_sixty_source: SourceKind,
    pub weight_lbs: u16,
    pub drivetrain: Drivetrain,
    /// Transmissions, `/`-separated (`"6MT/10AT"`). The first one decides manual vs automatic.
    pub transmission: Text,
    /// 0 = unknown.
    pub redline_rpm: u16,
    pub fuel: Fuel,
    pub confidence: Confidence,
}

impl Car {
    pub fn transmissions(&self) -> impl Iterator<Item = &str> {
        self.transmission.split('/').map(str::trim).filter(|s| !s.is_empty())
    }
    /// Automatic if the first listed transmission is not a manual ("MT", incl. sequential "SMT").
    pub fn is_auto(&self) -> bool {
        self.transmissions().next().map(|t| !t.contains("MT")).unwrap_or(false)
    }
    pub fn covers_year(&self, y: u16) -> bool {
        self.year_start <= y && y <= self.year_end
    }
}

// ---------- compact bundled format (written by build.rs, read by data.rs) ----------

/// `cars.bin` layout, all little endian:
/// ```text
///   magic  b"PLC1"
///   u32 string_count, then string_count x (u32 offset, u16 len)   -- interned UTF-8 strings
///   u32 bytes_len, then the string bytes
///   u32 make_count, then make_count x (u16 name, u16 group, u64 year_mask, u32 first_car, u32 car_count)
///   u32 car_count, then car_count fixed-size records (CAR_RECORD_LEN bytes):
///     u16 x 8 string ids: id make group model generation trim engine_code transmission
///     u16 x 8: year_start year_end hp hp_rpm torque torque_rpm weight_lbs redline_rpm
///     f32 x 2: displacement_l zero_sixty
///     u8  x 6: cylinders aspiration drivetrain fuel zero_sixty_source confidence
/// ```
pub const CARS_MAGIC: &[u8; 4] = b"PLC1";
pub const CAR_RECORD_LEN: usize = 16 + 16 + 8 + 6;

// ---------- authoring format (data/*.ron) ----------

fn d_u16() -> u16 {
    0
}

#[derive(Clone, Debug, Deserialize)]
pub struct TrimSrc {
    pub n: String,
    pub code: String,
    pub d: f32,
    pub c: u8,
    pub a: Aspiration,
    pub hp: u16,
    #[serde(default = "d_u16")]
    pub hpr: u16,
    pub tq: u16,
    #[serde(default = "d_u16")]
    pub tqr: u16,
    pub z: f32,
    #[serde(default)]
    pub zs: SourceKind,
    pub w: u16,
    pub dt: Drivetrain,
    pub tx: String,
    #[serde(default = "d_u16")]
    pub rl: u16,
    #[serde(default)]
    pub f: Fuel,
    #[serde(default)]
    pub conf: Confidence,
    #[serde(default)]
    pub y0: Option<u16>,
    #[serde(default)]
    pub y1: Option<u16>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GenSrc {
    pub make: String,
    pub model: String,
    #[serde(rename = "gen")]
    pub generation: String,
    pub y0: u16,
    pub y1: u16,
    pub trims: Vec<TrimSrc>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FileSrc {
    pub group: String,
    pub gens: Vec<GenSrc>,
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut last_dash = true;
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

pub fn expand(file: &FileSrc) -> Vec<Car> {
    let mut out = Vec::new();
    for g in &file.gens {
        for t in &g.trims {
            let tx: Vec<&str> = t.tx.split('/').map(str::trim).filter(|s| !s.is_empty()).collect();
            out.push(Car {
                id: format!("{}-{}-{}-{}", slug(&g.make), slug(&g.model), slug(&g.generation), slug(&t.n)).into(),
                make: g.make.clone().into(),
                parent_group: file.group.clone().into(),
                model: g.model.clone().into(),
                generation: g.generation.clone().into(),
                year_start: t.y0.unwrap_or(g.y0),
                year_end: t.y1.unwrap_or(g.y1),
                trim: t.n.clone().into(),
                engine_code: t.code.clone().into(),
                displacement_l: t.d,
                cylinders: t.c,
                aspiration: t.a,
                hp: t.hp,
                hp_rpm: t.hpr,
                torque: t.tq,
                torque_rpm: t.tqr,
                zero_sixty: t.z,
                zero_sixty_source: t.zs,
                weight_lbs: t.w,
                drivetrain: t.dt,
                transmission: tx.join("/").into(),
                redline_rpm: t.rl,
                fuel: t.f,
                confidence: t.conf,
            });
        }
    }
    out
}

/// Sanity checks shared by the build script, the runtime override importer and the tests.
pub fn validate_car(c: &Car) -> Result<(), String> {
    let id = &c.id;
    if id.is_empty() || c.make.trim().is_empty() || c.model.trim().is_empty() || c.trim.trim().is_empty() {
        return Err(format!("{id}: missing required text field"));
    }
    if c.parent_group.trim().is_empty() || c.engine_code.trim().is_empty() || c.transmissions().next().is_none() {
        return Err(format!("{id}: missing group/engine code/transmission"));
    }
    if c.year_start < YEAR_MIN || c.year_end > YEAR_MAX || c.year_start > c.year_end {
        return Err(format!("{id}: bad year range {}-{}", c.year_start, c.year_end));
    }
    if !(40..=1500).contains(&c.hp) {
        return Err(format!("{id}: hp {} out of range", c.hp));
    }
    if !(40..=1500).contains(&c.torque) {
        return Err(format!("{id}: torque {} out of range", c.torque));
    }
    if !(1500..=10_000).contains(&c.weight_lbs) {
        return Err(format!("{id}: weight {} out of range", c.weight_lbs));
    }
    if !c.displacement_l.is_finite() || !(1.5..=20.0).contains(&c.zero_sixty) {
        return Err(format!("{id}: 0-60 {} out of range", c.zero_sixty));
    }
    if c.aspiration == Aspiration::EV {
        if c.cylinders != 0 || c.displacement_l != 0.0 {
            return Err(format!("{id}: EV must have 0 cylinders/displacement"));
        }
    } else if !(0.6..=9.0).contains(&c.displacement_l) || !(2..=12).contains(&c.cylinders) {
        return Err(format!("{id}: displacement/cylinders out of range"));
    }
    if c.redline_rpm != 0 && !(3000..=11_000).contains(&c.redline_rpm) {
        return Err(format!("{id}: redline out of range"));
    }
    Ok(())
}

// ---------- mod definitions (data/mods.ron) ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModKind {
    Gain,
    Negative,
}

fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Slider {
    pub label: String,
    pub unit: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub step: f32,
}

/// What a mod does for a given aspiration. The first effect whose `applies_to` contains the
/// car's (current) aspiration and whose `requires` are all selected is used.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ModEffect {
    /// Empty = all aspirations.
    #[serde(default)]
    pub applies_to: Vec<Aspiration>,
    #[serde(default)]
    pub requires: Vec<String>,
    /// Percent change of current hp (fixed part). min <= max; typical is the midpoint.
    #[serde(default)]
    pub min_pct: f32,
    #[serde(default)]
    pub max_pct: f32,
    #[serde(default)]
    pub flat_hp: f32,
    /// Percent per slider unit (see `unit_div` on the mod's slider value).
    #[serde(default)]
    pub unit_pct_min: f32,
    #[serde(default)]
    pub unit_pct_max: f32,
    /// Flat hp per slider unit.
    #[serde(default)]
    pub unit_hp_min: f32,
    #[serde(default)]
    pub unit_hp_max: f32,
    #[serde(default = "one")]
    pub unit_div: f32,
    /// Torque change relative to the hp change (1.0 = same percentage).
    #[serde(default = "one")]
    pub torque_factor: f32,
    #[serde(default)]
    pub weight_lbs: f32,
    #[serde(default)]
    pub unit_lbs: f32,
    /// Direct change of the 0-60 time in percent (negative = quicker). Min/max.
    #[serde(default)]
    pub zs_pct_min: f32,
    #[serde(default)]
    pub zs_pct_max: f32,
    /// Raises the traction limit used by the 0-60 model, percent.
    #[serde(default)]
    pub traction_pct: f32,
    /// If set, the car counts as this aspiration for all later mods.
    #[serde(default)]
    pub makes_aspiration: Option<Aspiration>,
    /// Shown as a warning whenever this effect is used.
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModDef {
    pub id: String,
    pub name: String,
    pub kind: ModKind,
    pub desc: String,
    #[serde(default)]
    pub slider: Option<Slider>,
    /// Empty = all fuels.
    #[serde(default)]
    pub fuels: Vec<Fuel>,
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// Shown when no effect applies.
    #[serde(default)]
    pub incompatible_note: String,
    pub effects: Vec<ModEffect>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModsFile {
    pub mods: Vec<ModDef>,
}
