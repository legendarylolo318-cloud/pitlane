//! Local persistence (settings, favorites, saved comparisons, overrides) as RON files
//! inside an app-private directory supplied by the platform layer.

use crate::calc::ModSel;
use crate::data::Overrides;
use crate::units::Settings;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SlotState {
    pub year: u16,
    pub car_id: String,
    pub mods: Vec<ModSel>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Favorite {
    pub year: u16,
    pub car_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedComparison {
    pub name: String,
    pub slots: Vec<SlotState>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub settings: Settings,
    pub favorites: Vec<Favorite>,
    pub saved: Vec<SavedComparison>,
    pub slots: Vec<SlotState>,
}

pub struct Store {
    dir: PathBuf,
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&dir);
        Store { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Default per-user directory for desktop platforms (no extra dependency).
    pub fn default_dir() -> PathBuf {
        let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
        let base = if cfg!(windows) {
            env("APPDATA")
        } else if cfg!(target_os = "macos") {
            env("HOME").map(|h| h.join("Library/Application Support"))
        } else {
            env("XDG_DATA_HOME").or_else(|| env("HOME").map(|h| h.join(".local/share")))
        };
        base.unwrap_or_else(std::env::temp_dir).join("pitlane")
    }

    fn state_path(&self) -> PathBuf {
        self.dir.join("state.ron")
    }
    fn overrides_path(&self) -> PathBuf {
        self.dir.join("overrides.ron")
    }

    /// Returns the state and, if the file existed but was unreadable, a message (the bad file is kept as .bad).
    pub fn load_state(&self) -> (AppState, Option<String>) {
        let p = self.state_path();
        match fs::read_to_string(&p) {
            Err(_) => (AppState::default(), None),
            Ok(t) => match ron::from_str(&t) {
                Ok(s) => (s, None),
                Err(e) => {
                    let _ = fs::rename(&p, p.with_extension("bad"));
                    (AppState::default(), Some(format!("Saved state was unreadable and was reset: {e}")))
                }
            },
        }
    }

    pub fn save_state(&self, s: &AppState) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(s, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
        write_atomic(&self.state_path(), &text)
    }

    pub fn load_overrides(&self) -> (Overrides, Option<String>) {
        match fs::read_to_string(self.overrides_path()) {
            Err(_) => (Overrides::default(), None),
            Ok(t) => match Overrides::from_text(&t) {
                Ok(o) => (o, None),
                Err(e) => (Overrides::default(), Some(format!("Overrides file ignored: {e}"))),
            },
        }
    }

    pub fn save_overrides(&self, o: &Overrides) -> Result<(), String> {
        write_atomic(&self.overrides_path(), &o.to_text()?)
    }

    pub fn write_export(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
        let dir = self.dir.join("exports");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let p = dir.join(name);
        fs::write(&p, bytes).map_err(|e| format!("write {}: {e}", p.display()))?;
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pitlane-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn state_roundtrip_and_corruption() {
        let dir = tmp("state");
        let s = Store::new(dir.clone());
        let mut st = AppState::default();
        st.favorites.push(Favorite { year: 2006, car_id: "x".into() });
        st.slots.push(SlotState { year: 2010, car_id: "y".into(), mods: vec![ModSel { id: "intake".into(), value: 0.0 }] });
        s.save_state(&st).unwrap();
        assert_eq!(s.load_state().0, st);
        fs::write(dir.join("state.ron"), "garbage((").unwrap();
        let (st2, msg) = s.load_state();
        assert_eq!(st2, AppState::default());
        assert!(msg.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn overrides_roundtrip_and_validation() {
        let dir = tmp("ov");
        let s = Store::new(dir.clone());
        assert!(Overrides::from_text("(cars: [])").is_ok());
        assert!(Overrides::from_text("nonsense").is_err());
        fs::write(dir.join("overrides.ron"), "oops").unwrap();
        assert!(s.load_overrides().1.is_some());
        let _ = fs::remove_dir_all(dir);
    }
}
