//! Pitlane core: bundled dataset, mod math, units, search, persistence and comparison tables.
//! No UI code lives here.

pub mod calc;
pub mod compare;
pub mod data;
pub mod model;
pub mod search;
pub mod store;
pub mod units;

pub use calc::{evaluate, Eval, ModSel, Range, Warning};
pub use data::{mod_defs, Db, Overrides};
pub use model::*;
pub use units::{LossCfg, PowerUnit, Settings, UnitSystem};
