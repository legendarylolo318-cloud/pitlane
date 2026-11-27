//! Pitlane UI: Slint front-end over `pitlane-core`. Shared by desktop (`main.rs`) and Android (`android_main`).
//!
//! Performance notes:
//! * nothing runs while idle: no repeating timers or animations; autosave and search are
//!   debounced single-shot timers that only exist after a change;
//! * every Slint model is created once and updated in place by diffing (`sync`), so a slider
//!   drag or keystroke only touches the rows whose text actually changed.

use pitlane_core::compare::{build_table, car_title, to_csv, to_text, SlotView};
use pitlane_core::search::{search, Hit};
use pitlane_core::store::{AppState, Favorite, SavedComparison, SlotState, Store};
use pitlane_core::units::*;
use pitlane_core::*;
use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

slint::include_modules!();

const MAX_SLOTS: usize = 5;
const SLOT_COLORS: [(u8, u8, u8); MAX_SLOTS] = [(0x4e, 0x79, 0xa7), (0xf2, 0x8e, 0x2b), (0x59, 0xa1, 0x4f), (0xb0, 0x7a, 0xa1), (0xe1, 0x57, 0x59)];
const DATA_NOTE: &str = "Data accuracy: figures are approximate. 'verified' = matches commonly published factory specs as recalled by the author (not machine-checked); 'estimated' = approximate. 0-60 times and weights are the least reliable. Mod gains are typical ranges, not measurements.";
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);
const SAVE_DEBOUNCE: Duration = Duration::from_millis(1500);

// ---------------------------------------------------------------- platform helpers

#[cfg(not(target_os = "android"))]
fn clipboard_set(text: &str) -> Result<(), String> {
    thread_local! { static CLIP: RefCell<Option<arboard::Clipboard>> = const { RefCell::new(None) }; }
    CLIP.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
        }
        match c.as_mut() {
            Some(cb) => cb.set_text(text.to_string()).map_err(|e| e.to_string()),
            None => Err("clipboard unavailable".into()),
        }
    })
}

#[cfg(target_os = "android")]
fn clipboard_set(_text: &str) -> Result<(), String> {
    Err("Clipboard is not wired up on Android yet: use 'Save file', or long-press the text, Select all, Copy.".into())
}

// ---------------------------------------------------------------- models (created once, diffed)

fn sync<T: Clone + PartialEq + 'static>(m: &VecModel<T>, new: Vec<T>) {
    let old = m.row_count();
    for (i, v) in new.iter().enumerate().take(old) {
        if m.row_data(i).as_ref() != Some(v) {
            m.set_row_data(i, v.clone());
        }
    }
    if new.len() < old {
        for i in (new.len()..old).rev() {
            m.remove(i);
        }
    } else {
        for v in new.into_iter().skip(old) {
            m.push(v);
        }
    }
}

fn mrc<T: Clone + 'static>(m: &Rc<VecModel<T>>) -> ModelRc<T> {
    ModelRc::from(m.clone() as Rc<dyn Model<Data = T>>)
}

/// A list of rows whose cells are themselves a model. Inner models are kept per row index so an
/// unchanged row compares equal (same `ModelRc`) and is never re-instantiated by the UI.
#[derive(Default)]
struct Nested {
    outer: Rc<VecModel<RowData>>,
    inner: RefCell<Vec<Rc<VecModel<CellData>>>>,
}

impl Nested {
    fn set(&self, rows: Vec<(SharedString, Vec<CellData>)>) {
        let mut inner = self.inner.borrow_mut();
        while inner.len() < rows.len() {
            inner.push(Rc::new(VecModel::default()));
        }
        inner.truncate(rows.len());
        let mut out = Vec::with_capacity(rows.len());
        for (i, (label, cells)) in rows.into_iter().enumerate() {
            sync(&inner[i], cells);
            out.push(RowData { label, cells: mrc(&inner[i]) });
        }
        sync(&self.outer, out);
    }
}

#[derive(Default)]
struct Models {
    years: Rc<VecModel<SharedString>>,
    makes: Rc<VecModel<SharedString>>,
    models: Rc<VecModel<SharedString>>,
    trims: Rc<VecModel<SharedString>>,
    slots: Rc<VecModel<SlotInfo>>,
    headers: Rc<VecModel<SlotInfo>>,
    gain: Rc<VecModel<ModRow>>,
    neg: Rc<VecModel<ModRow>>,
    rows: Nested,
    compact: Nested,
    charts: Nested,
    hits: Rc<VecModel<SharedString>>,
    favorites: Rc<VecModel<SharedString>>,
    saved: Rc<VecModel<SavedItem>>,
}

impl Models {
    /// Hands every model to the UI once; afterwards only their contents change.
    fn bind(&self, b: &Backend) {
        sync(&self.years, (YEAR_MIN..=YEAR_MAX).rev().map(|y| ss(y.to_string())).collect());
        b.set_years(mrc(&self.years));
        b.set_makes(mrc(&self.makes));
        b.set_models(mrc(&self.models));
        b.set_trims(mrc(&self.trims));
        b.set_slots(mrc(&self.slots));
        b.set_headers(mrc(&self.headers));
        b.set_gain_mods(mrc(&self.gain));
        b.set_neg_mods(mrc(&self.neg));
        b.set_rows(mrc(&self.rows.outer));
        b.set_compact_rows(mrc(&self.compact.outer));
        b.set_charts(mrc(&self.charts.outer));
        b.set_search_hits(mrc(&self.hits));
        b.set_favorites(mrc(&self.favorites));
        b.set_saved(mrc(&self.saved));
        b.set_data_note(ss(DATA_NOTE));
    }
}

// ---------------------------------------------------------------- app state

struct Ctx {
    db: Db,
    store: Store,
    state: AppState,
    sel: usize,
    hits: Vec<Hit>,
    status: String,
    export_kind: u8,
    edit_new: bool,
    dirty: bool,
    m: Rc<Models>,
}

type Shared = Rc<RefCell<Ctx>>;

fn year_of_index(i: i32) -> u16 {
    (YEAR_MAX as i32 - i.max(0)).clamp(YEAR_MIN as i32, YEAR_MAX as i32) as u16
}

fn ss(s: impl AsRef<str>) -> SharedString {
    SharedString::from(s.as_ref())
}

fn default_slot(db: &Db) -> Option<SlotState> {
    let c = db.find("ford-mustang-s550-2015-2023-gt-5-0-2018").or_else(|| db.all_cars().next())?;
    let year = if c.covers_year(2018) { 2018 } else { c.year_end };
    Some(SlotState { year, car_id: c.id.to_string(), mods: vec![] })
}

impl Ctx {
    /// Drops slots whose car no longer exists (e.g. a removed custom car), clamps years and count.
    fn ensure_slots(&mut self) {
        let db = &self.db;
        self.state.slots.retain(|s| db.find(&s.car_id).is_some());
        for s in &mut self.state.slots {
            if let Some(c) = db.find(&s.car_id) {
                s.year = s.year.clamp(c.year_start, c.year_end);
            }
        }
        self.state.slots.truncate(MAX_SLOTS);
        if self.state.slots.is_empty() {
            if let Some(s) = default_slot(&self.db) {
                self.state.slots.push(s);
            }
        }
        self.sel = self.sel.min(self.state.slots.len().saturating_sub(1));
    }

    fn car(&self, slot: usize) -> Option<&Car> {
        self.state.slots.get(slot).and_then(|s| self.db.find(&s.car_id))
    }

    /// Resolve the picker cascade: keep the preferred make/model/trim when they exist for `year`,
    /// otherwise fall back to the first valid entry at that level (and everything below it).
    fn resolve(&self, year: u16, make: Option<&str>, model: Option<&str>, trim: Option<&str>) -> Option<Car> {
        let makes = self.db.makes_for_year(year);
        let mk = make.filter(|m| makes.contains(m)).or_else(|| makes.first().copied())?;
        let models = self.db.models_for(year, mk);
        let md = model.filter(|m| models.contains(m)).or_else(|| models.first().copied())?;
        let trims = self.db.trims_for(year, mk, md);
        trims.iter().find(|c| Some(&*c.trim) == trim).or_else(|| trims.first()).map(|c| (*c).clone())
    }

    fn assign(&mut self, year: u16, car: &Car) {
        if let Some(s) = self.state.slots.get_mut(self.sel) {
            s.year = year;
            s.car_id = car.id.to_string();
        }
        self.dirty = true;
    }

    fn settings(&self) -> Settings {
        self.state.settings
    }
}

// ---------------------------------------------------------------- formatting

struct Fmt {
    s: Settings,
}

impl Fmt {
    fn hp(&self, v: f32, wf: f32) -> f32 {
        power_from_hp(self.s.power, v * wf)
    }
    fn tq(&self, v: f32, wf: f32) -> f32 {
        torque_from_lbft(self.s.system, v * wf)
    }
    fn signed(v: f32, d: usize) -> String {
        if v >= 0.0 {
            format!("+{v:.d$}")
        } else {
            format!("{v:.d$}")
        }
    }
}

fn wf_for(car: &Car, ev: &Eval, s: &Settings) -> f32 {
    if s.wheel {
        wheel_factor(car, ev.aspiration, &s.loss)
    } else {
        1.0
    }
}

fn stock_line(car: &Car, s: &Settings) -> String {
    let f = Fmt { s: *s };
    let wf = if s.wheel { wheel_factor(car, car.aspiration, &s.loss) } else { 1.0 };
    let eng = if car.aspiration == Aspiration::EV { "Electric".to_string() } else { format!("{:.1} L {}-cyl {}", car.displacement_l, car.cylinders, car.aspiration.label()) };
    format!(
        "Stock{}: {:.0} {} - {:.0} {} - {} {:.1} s ({}) - {:.0} {} - {} - {} - {} [{}]",
        if s.wheel { " (wheel, est.)" } else { "" },
        f.hp(car.hp as f32, wf),
        power_label(s.power),
        f.tq(car.torque as f32, wf),
        torque_label(s.system),
        accel_label(s.system),
        accel_from_60(s.system, car.zero_sixty),
        car.zero_sixty_source.label(),
        weight_from_lbs(s.system, car.weight_lbs as f32),
        weight_label(s.system),
        car.drivetrain.label(),
        car.transmission,
        eng,
        car.confidence.label()
    )
}

fn mod_line(car: &Car, ev: &Eval, s: &Settings) -> String {
    if !ev.modded {
        return String::new();
    }
    let f = Fmt { s: *s };
    let wf = wf_for(car, ev, s);
    let wf0 = if s.wheel { wheel_factor(car, car.aspiration, &s.loss) } else { 1.0 };
    let hp0 = f.hp(car.hp as f32, wf0);
    let (lo, typ, hi) = (f.hp(ev.hp.min, wf), f.hp(ev.hp.typ, wf), f.hp(ev.hp.max, wf));
    let z0 = accel_from_60(s.system, car.zero_sixty);
    let z1 = accel_from_60(s.system, ev.zero_sixty.typ);
    format!(
        "Modded (est.): {:.0} {} ({}, range {:.0}-{:.0}) - {:.0} {} ({}) - {} {:.1} s ({}) - {:.0} {}",
        typ,
        power_label(s.power),
        Fmt::signed(typ - hp0, 0),
        lo,
        hi,
        f.tq(ev.torque.typ, wf),
        torque_label(s.system),
        Fmt::signed(f.tq(ev.torque.typ, wf) - f.tq(car.torque as f32, wf0), 0),
        accel_label(s.system),
        z1,
        Fmt::signed(z1 - z0, 1),
        weight_from_lbs(s.system, ev.weight_lbs),
        weight_label(s.system),
    )
}

fn gain_text(car: &Car, full: &Eval, without: &Eval, s: &Settings) -> String {
    let f = Fmt { s: *s };
    let wf = wf_for(car, full, s);
    let (lo, typ, hi) = (f.hp(full.hp.min - without.hp.min, wf), f.hp(full.hp.typ - without.hp.typ, wf), f.hp(full.hp.max - without.hp.max, wf));
    let mut parts = Vec::new();
    if typ.abs() >= 0.5 || hi.abs() >= 1.0 {
        parts.push(format!("{} to {} {} (typical {}) est.", Fmt::signed(lo.min(hi), 0), Fmt::signed(lo.max(hi), 0), power_label(s.power), Fmt::signed(typ, 0)));
        let t = f.tq(full.torque.typ - without.torque.typ, wf);
        parts.push(format!("{} {}", Fmt::signed(t, 0), torque_label(s.system)));
    }
    let dw = full.weight_lbs - without.weight_lbs;
    if dw.abs() >= 0.5 {
        parts.push(format!("{} {}", Fmt::signed(weight_from_lbs(s.system, dw), 0), weight_label(s.system)));
    }
    let dz = accel_from_60(s.system, full.zero_sixty.typ) - accel_from_60(s.system, without.zero_sixty.typ);
    if dz.abs() >= 0.005 {
        parts.push(format!("{} {} s est.", accel_label(s.system), Fmt::signed(dz, 2)));
    }
    parts.join("  -  ")
}

fn mod_rows(car: &Car, slot: &SlotState, full: &Eval, kind: ModKind, s: &Settings) -> Vec<ModRow> {
    let defs = mod_defs();
    defs.iter()
        .filter(|d| d.kind == kind)
        .map(|d| {
            let sel = slot.mods.iter().find(|m| m.id == d.id);
            let (value, min, max, step, label, unit) = match &d.slider {
                Some(sl) => (sel.map(|m| m.value).unwrap_or(sl.default), sl.min, sl.max, sl.step, sl.label.as_str(), sl.unit.as_str()),
                None => (0.0, 0.0, 1.0, 1.0, "", ""),
            };
            let gain = if sel.is_some() {
                let rest: Vec<ModSel> = slot.mods.iter().filter(|m| m.id != d.id).cloned().collect();
                gain_text(car, full, &evaluate(car, &rest, defs), s)
            } else {
                String::new()
            };
            let warning = full.warnings.iter().filter(|w| w.mod_id == d.id).map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            ModRow {
                id: ss(&d.id),
                name: ss(&d.name),
                desc: ss(&d.desc),
                enabled: sel.is_some(),
                has_slider: d.slider.is_some(),
                value,
                min,
                max,
                step,
                slider_label: ss(label),
                value_text: ss(format!("{value:.0} {unit}")),
                gain: ss(gain),
                warning: ss(warning),
            }
        })
        .collect()
}

/// Trim names shown in the picker; a generation is appended when two trims share a name
/// (e.g. a model year sold in two generations).
fn trim_labels(trims: &[&Car]) -> Vec<SharedString> {
    trims
        .iter()
        .map(|t| {
            if trims.iter().filter(|o| o.trim == t.trim).count() > 1 {
                ss(format!("{} - {}", t.trim, t.generation))
            } else {
                ss(&*t.trim)
            }
        })
        .collect()
}

fn slot_color(i: usize) -> Color {
    let (r, g, b) = SLOT_COLORS[i % MAX_SLOTS];
    Color::from_rgb_u8(r, g, b)
}

// ---------------------------------------------------------------- refresh

/// Pushes the current state into the UI. Cheap to call: models are diffed, so only changed rows
/// are touched and the renderer only repaints what changed.
fn refresh(ui: &MainWindow, c: &Ctx) {
    let b = ui.global::<Backend>();
    let m = &c.m;
    let s = c.settings();
    let defs = mod_defs();

    // evaluate every slot once
    let views: Vec<(usize, SlotView)> = c
        .state
        .slots
        .iter()
        .enumerate()
        .filter_map(|(i, sl)| c.db.find(&sl.car_id).map(|car| (i, SlotView { year: sl.year, car, eval: evaluate(car, &sl.mods, defs) })))
        .collect();

    sync(
        &m.slots,
        c.state
            .slots
            .iter()
            .enumerate()
            .map(|(i, sl)| {
                let car = c.db.find(&sl.car_id);
                SlotInfo {
                    title: car.map(|x| ss(car_title(sl.year, x))).unwrap_or_default(),
                    subtitle: car.map(|x| ss(&*x.trim)).unwrap_or_default(),
                    modded: !sl.mods.is_empty(),
                    color: slot_color(i),
                }
            })
            .collect(),
    );
    b.set_selected(c.sel as i32);

    // pickers + mods for the selected slot
    if let (Some(slot), Some((_, view))) = (c.state.slots.get(c.sel), views.iter().find(|(i, _)| *i == c.sel)) {
        let car = view.car;
        b.set_year_index((YEAR_MAX - slot.year.clamp(YEAR_MIN, YEAR_MAX)) as i32);
        let makes = c.db.makes_for_year(slot.year);
        sync(&m.makes, makes.iter().map(ss).collect());
        b.set_make_index(makes.iter().position(|x| *x == car.make).unwrap_or(0) as i32);
        let models = c.db.models_for(slot.year, &car.make);
        sync(&m.models, models.iter().map(ss).collect());
        b.set_model_index(models.iter().position(|x| *x == car.model).unwrap_or(0) as i32);
        let trims = c.db.trims_for(slot.year, &car.make, &car.model);
        sync(&m.trims, trim_labels(&trims));
        b.set_trim_index(trims.iter().position(|t| t.id == car.id).unwrap_or(0) as i32);

        b.set_selected_title(ss(car_title(slot.year, car)));
        b.set_stock_line(ss(stock_line(car, &s)));
        b.set_mod_line(ss(mod_line(car, &view.eval, &s)));
        b.set_is_fav(c.state.favorites.iter().any(|f| f.car_id == car.id && f.year == slot.year));
        sync(&m.gain, mod_rows(car, slot, &view.eval, ModKind::Gain, &s));
        sync(&m.neg, mod_rows(car, slot, &view.eval, ModKind::Negative, &s));
    } else {
        b.set_selected_title(ss("No car selected"));
        b.set_stock_line(ss(""));
        b.set_mod_line(ss(""));
    }

    // comparison table + charts
    let views: Vec<SlotView> = views.into_iter().map(|(_, v)| v).collect();
    let t = build_table(&views, s);
    let cell = |x: &pitlane_core::compare::Cell| CellData { text: ss(&x.text), sub: ss(&x.sub), best: x.best, frac: x.frac };
    sync(
        &m.headers,
        t.headers.iter().enumerate().map(|(i, h)| SlotInfo { title: ss(&h.title), subtitle: ss(&h.subtitle), modded: h.modded, color: slot_color(i) }).collect(),
    );
    m.rows.set(t.rows.iter().map(|r| (ss(&r.label), r.cells.iter().map(cell).collect())).collect());
    m.compact.set(t.rows.iter().filter(|r| r.compact).map(|r| (ss(&r.label), r.cells.iter().map(cell).collect())).collect());
    m.charts.set(t.charts.iter().map(|ch| (ss(&ch.title), ch.bars.iter().map(cell).collect())).collect());

    // toolbar & lists
    b.set_units_text(ss(match s.system {
        UnitSystem::Imperial => "Imperial",
        UnitSystem::Metric => "Metric",
    }));
    b.set_power_text(ss(power_label(s.power)));
    b.set_wheel_text(ss(if s.wheel { "Wheel (est.)" } else { "Crank" }));
    b.set_status(ss(&c.status));
    sync(&m.hits, c.hits.iter().map(|h| ss(&h.label)).collect());
    sync(
        &m.favorites,
        c.state
            .favorites
            .iter()
            .map(|f| ss(c.db.find(&f.car_id).map(|x| format!("{} {} {} {}", f.year, x.make, x.model, x.trim)).unwrap_or_else(|| format!("(missing) {}", f.car_id))))
            .collect(),
    );
    sync(&m.saved, c.state.saved.iter().map(|sv| SavedItem { name: ss(&sv.name), detail: ss(format!("{} cars", sv.slots.len())) }).collect());
}

fn persist(c: &mut Ctx) {
    if let Err(e) = c.store.save_state(&c.state) {
        c.status = format!("Could not save: {e}");
    }
    c.dirty = false;
}

fn rebuild_db(c: &mut Ctx, ov: Overrides) {
    c.db = Db::with_overrides(ov);
    c.ensure_slots();
}

// ---------------------------------------------------------------- edit dialog

fn fill_edit(ui: &MainWindow, c: &Ctx, new_car: bool) {
    let b = ui.global::<Backend>();
    let Some(car) = c.car(c.sel) else { return };
    let year = c.state.slots.get(c.sel).map(|s| s.year).unwrap_or(car.year_end);
    b.set_ed_heading(ss(if new_car { "Add a custom car" } else { "Edit this car's stats" }));
    b.set_ed_new(new_car);
    b.set_ed_make(ss(&*car.make));
    b.set_ed_model(ss(&*car.model));
    b.set_ed_trim(ss(if new_car { format!("{} (custom)", car.trim) } else { car.trim.to_string() }));
    b.set_ed_year(ss(year.to_string()));
    b.set_ed_disp(ss(format!("{:.1}", car.displacement_l)));
    b.set_ed_cyl(ss(car.cylinders.to_string()));
    b.set_ed_hp(ss(car.hp.to_string()));
    b.set_ed_tq(ss(car.torque.to_string()));
    b.set_ed_zs(ss(format!("{:.1}", car.zero_sixty)));
    b.set_ed_weight(ss(car.weight_lbs.to_string()));
    b.set_ed_rl(ss(car.redline_rpm.to_string()));
    b.set_ed_asp(Aspiration::ALL.iter().position(|a| *a == car.aspiration).unwrap_or(0) as i32);
    b.set_ed_dt(Drivetrain::ALL.iter().position(|d| *d == car.drivetrain).unwrap_or(0) as i32);
    b.set_ed_can_reset(!new_car && c.db.overrides().cars.iter().any(|o| o.id == car.id));
}

fn parse<T: std::str::FromStr>(s: &SharedString, what: &str) -> Result<T, String> {
    s.trim().parse::<T>().map_err(|_| format!("'{}' is not a valid {what}", s.trim()))
}

fn read_edit(ui: &MainWindow, base: &Car, as_new: bool, year_now: u16) -> Result<(Car, u16), String> {
    let b = ui.global::<Backend>();
    let mut car = base.clone();
    let text = |s: SharedString| -> Text { s.trim().to_string().into() };
    car.make = text(b.get_ed_make());
    car.model = text(b.get_ed_model());
    car.trim = text(b.get_ed_trim());
    car.displacement_l = parse(&b.get_ed_disp(), "displacement")?;
    car.cylinders = parse(&b.get_ed_cyl(), "cylinder count")?;
    car.hp = parse(&b.get_ed_hp(), "horsepower")?;
    car.torque = parse(&b.get_ed_tq(), "torque")?;
    car.zero_sixty = parse(&b.get_ed_zs(), "0-60 time")?;
    car.weight_lbs = parse(&b.get_ed_weight(), "weight")?;
    car.redline_rpm = parse(&b.get_ed_rl(), "redline")?;
    car.aspiration = *Aspiration::ALL.get(b.get_ed_asp().max(0) as usize).unwrap_or(&Aspiration::NA);
    car.drivetrain = *Drivetrain::ALL.get(b.get_ed_dt().max(0) as usize).unwrap_or(&Drivetrain::RWD);
    let year: u16 = parse(&b.get_ed_year(), "year")?;
    if car.aspiration == Aspiration::EV {
        car.displacement_l = 0.0;
        car.cylinders = 0;
        car.fuel = Fuel::Electric;
    } else if car.fuel == Fuel::Electric {
        car.fuel = if car.aspiration == Aspiration::Hybrid { Fuel::Hybrid } else { Fuel::Gas };
    }
    if as_new {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        car.id = format!("custom-{}-{}-{}", model::slug(&car.make), model::slug(&car.model), stamp).into();
        car.generation = "custom".into();
        car.engine_code = "custom".into();
        car.year_start = year;
        car.year_end = year;
        if car.make != base.make {
            car.parent_group = "Custom".into();
        }
        car.confidence = Confidence::Estimated;
        car.zero_sixty_source = SourceKind::Est;
    } else {
        // An override keeps the bundled id, so it must stay in the same make/model/years;
        // anything else would create a second car with the same id.
        if car.make != base.make || car.model != base.model {
            return Err("make/model can only be changed on a new custom car (use 'Save as a new custom car')".into());
        }
        if year != year_now {
            return Err("the year can only be changed on a new custom car (use 'Save as a new custom car')".into());
        }
        car.confidence = Confidence::Estimated;
    }
    validate_car(&car)?;
    Ok((car, if as_new { year } else { year_now }))
}

// ---------------------------------------------------------------- wiring

/// Saves shortly after the last change instead of polling on a timer.
fn schedule_save(saver: &Rc<Timer>, cx: &Shared) {
    let cx = Rc::downgrade(cx);
    saver.start(TimerMode::SingleShot, SAVE_DEBOUNCE, move || {
        if let Some(cx) = cx.upgrade() {
            if let Ok(mut c) = cx.try_borrow_mut() {
                if c.dirty {
                    persist(&mut c);
                }
            }
        }
    });
}

macro_rules! hook {
    ($ui:ident, $ctx:ident, $saver:ident, $reg:ident, |$w:ident, $c:ident $(, $a:ident : $t:ty)*| $body:block) => {{
        let wk = $ui.as_weak();
        let cx = $ctx.clone();
        let sv = $saver.clone();
        $ui.global::<Backend>().$reg(move |$($a : $t),*| {
            let Some($w) = wk.upgrade() else { return };
            let dirty = {
                #[allow(unused_mut)]
                let Ok(mut $c) = cx.try_borrow_mut() else { return };
                #[allow(clippy::redundant_closure_call)]
                (|| $body)();
                $c.dirty
            };
            if dirty {
                schedule_save(&sv, &cx);
            }
        });
    }};
}

/// Builds the window and wires every callback. Shared by `run_with_dir` and the snapshot tool.
pub fn build(dir: PathBuf) -> Option<(MainWindow, Rc<dyn std::any::Any>)> {
    let store = Store::new(dir);
    let (state, state_err) = store.load_state();
    let (overrides, ov_err) = store.load_overrides();
    let db = Db::with_overrides(overrides);
    let mut ctx = Ctx {
        db,
        store,
        state,
        sel: 0,
        hits: vec![],
        status: state_err.or(ov_err).unwrap_or_default(),
        export_kind: 0,
        edit_new: false,
        dirty: false,
        m: Rc::new(Models::default()),
    };
    ctx.ensure_slots();
    let ctx: Shared = Rc::new(RefCell::new(ctx));
    let saver = Rc::new(Timer::default());
    let searcher = Rc::new(Timer::default());

    let ui = match MainWindow::new() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Pitlane: could not create the window: {e}");
            return None;
        }
    };
    ctx.borrow().m.bind(&ui.global::<Backend>());

    hook!(ui, ctx, saver, on_select_slot, |w, c, i: i32| {
        c.sel = (i.max(0) as usize).min(c.state.slots.len().saturating_sub(1));
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_add_slot, |w, c| {
        if c.state.slots.len() >= MAX_SLOTS {
            c.status = "Maximum of 5 cars".into();
        } else if let Some(cur) = c.state.slots.get(c.sel).cloned() {
            // a new slot starts as the stock version of the current car; pick another with the pickers
            c.state.slots.push(SlotState { mods: vec![], ..cur });
            c.sel = c.state.slots.len() - 1;
            c.dirty = true;
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_duplicate_slot, |w, c| {
        if c.state.slots.len() >= MAX_SLOTS {
            c.status = "Maximum of 5 cars".into();
        } else if let Some(cur) = c.state.slots.get(c.sel).cloned() {
            let at = c.sel + 1;
            c.state.slots.insert(at, cur);
            c.sel = at;
            c.dirty = true;
            c.status = "Duplicated - change its mods to compare stock vs modded".into();
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_remove_slot, |w, c| {
        if c.state.slots.len() > 1 {
            let i = c.sel.min(c.state.slots.len() - 1);
            c.state.slots.remove(i);
            c.sel = i.min(c.state.slots.len() - 1);
            c.dirty = true;
        } else {
            c.status = "At least one car is needed".into();
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_year_picked, |w, c, i: i32| {
        let year = year_of_index(i);
        if let Some(cur) = c.car(c.sel).cloned() {
            // keeps make/model/trim when they exist for the new year, otherwise resets the
            // first level that does not (and everything below it)
            if let Some(car) = c.resolve(year, Some(&cur.make), Some(&cur.model), Some(&cur.trim)) {
                c.assign(year, &car);
            } else {
                c.status = format!("No cars for {year}");
            }
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_make_picked, |w, c, i: i32| {
        if let Some(sl) = c.state.slots.get(c.sel).cloned() {
            let pick = c.db.makes_for_year(sl.year).get(i.max(0) as usize).map(|m| m.to_string());
            // a new make resets model and trim to that make's first valid ones for the year
            if let Some(car) = c.resolve(sl.year, pick.as_deref(), None, None) {
                c.assign(sl.year, &car);
            }
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_model_picked, |w, c, i: i32| {
        if let (Some(sl), Some(cur)) = (c.state.slots.get(c.sel).cloned(), c.car(c.sel).cloned()) {
            let pick = c.db.models_for(sl.year, &cur.make).get(i.max(0) as usize).map(|m| m.to_string());
            if let Some(car) = c.resolve(sl.year, Some(&cur.make), pick.as_deref(), None) {
                c.assign(sl.year, &car);
            }
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_trim_picked, |w, c, i: i32| {
        if let (Some(sl), Some(cur)) = (c.state.slots.get(c.sel).cloned(), c.car(c.sel).cloned()) {
            let pick = c.db.trims_for(sl.year, &cur.make, &cur.model).get(i.max(0) as usize).map(|t| (*t).clone());
            if let Some(car) = pick {
                c.assign(sl.year, &car);
            }
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_mod_toggled, |w, c, id: SharedString, on: bool| {
        let sel = c.sel;
        if let Some(sl) = c.state.slots.get_mut(sel) {
            sl.mods.retain(|m| m.id != id.as_str());
            if on {
                let default = mod_defs().iter().find(|d| d.id == id.as_str()).and_then(|d| d.slider.as_ref().map(|s| s.default)).unwrap_or(0.0);
                sl.mods.push(ModSel { id: id.to_string(), value: default });
            }
        }
        c.dirty = true;
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_mod_value, |w, c, id: SharedString, v: f32| {
        let step = mod_defs().iter().find(|d| d.id == id.as_str()).and_then(|d| d.slider.as_ref().map(|s| s.step)).unwrap_or(1.0).max(f32::EPSILON);
        let sel = c.sel;
        let mut changed = false;
        if let Some(m) = c.state.slots.get_mut(sel).and_then(|sl| sl.mods.iter_mut().find(|m| m.id == id.as_str())) {
            let snapped = (v / step).round() * step;
            // sliders report every pixel of a drag; only recompute when the snapped value moves
            if v.is_finite() && snapped != m.value {
                m.value = snapped;
                changed = true;
            }
        }
        if changed {
            c.dirty = true;
            refresh(&w, &c);
        }
    });
    {
        // debounced: a burst of keystrokes runs one search
        let cx = ctx.clone();
        let tm = searcher.clone();
        ui.global::<Backend>().on_search_edited(move |t: SharedString| {
            let q = t.trim().to_string();
            if q.is_empty() {
                tm.stop();
                if let Ok(mut c) = cx.try_borrow_mut() {
                    c.hits.clear();
                    sync(&c.m.hits, vec![]);
                }
                return;
            }
            let cx = Rc::downgrade(&cx);
            tm.start(TimerMode::SingleShot, SEARCH_DEBOUNCE, move || {
                let Some(cx) = cx.upgrade() else { return };
                let Ok(mut c) = cx.try_borrow_mut() else { return };
                c.hits = search(&c.db, &q, 8);
                let labels = c.hits.iter().map(|h| ss(&h.label)).collect();
                sync(&c.m.hits, labels);
            });
        });
    }
    hook!(ui, ctx, saver, on_search_pick, |w, c, i: i32| {
        if let Some(h) = c.hits.get(i.max(0) as usize).cloned() {
            if let Some(car) = c.db.find(&h.car_id).cloned() {
                c.assign(h.year, &car);
                c.status = format!("Loaded {}", h.label);
            }
        }
        c.hits.clear();
        w.global::<Backend>().set_search_text(ss(""));
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_toggle_units, |w, c| {
        c.state.settings.system = if c.state.settings.system == UnitSystem::Imperial { UnitSystem::Metric } else { UnitSystem::Imperial };
        c.dirty = true;
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_cycle_power, |w, c| {
        c.state.settings.power = match c.state.settings.power {
            PowerUnit::Hp => PowerUnit::Kw,
            PowerUnit::Kw => PowerUnit::Ps,
            PowerUnit::Ps => PowerUnit::Hp,
        };
        c.dirty = true;
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_toggle_wheel, |w, c| {
        c.state.settings.wheel = !c.state.settings.wheel;
        c.dirty = true;
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_toggle_fav, |w, c| {
        if let Some(sl) = c.state.slots.get(c.sel).cloned() {
            let before = c.state.favorites.len();
            c.state.favorites.retain(|f| !(f.car_id == sl.car_id && f.year == sl.year));
            if c.state.favorites.len() == before {
                c.state.favorites.push(Favorite { year: sl.year, car_id: sl.car_id });
            }
            c.dirty = true;
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_pick_fav, |w, c, i: i32| {
        if let Some(f) = c.state.favorites.get(i.max(0) as usize).cloned() {
            match c.db.find(&f.car_id).cloned() {
                Some(car) => {
                    c.assign(f.year.clamp(car.year_start, car.year_end), &car);
                    w.global::<Backend>().set_dialog(0);
                    w.set_tab(0);
                }
                None => c.status = "That car is no longer in the data (override removed?)".into(),
            }
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_save_comparison, |w, c| {
        let typed = w.global::<Backend>().get_save_name().trim().to_string();
        let name = if typed.is_empty() { format!("Comparison {}", c.state.saved.len() + 1) } else { typed };
        let slots = c.state.slots.clone();
        // saving under an existing name replaces it instead of piling up duplicates
        match c.state.saved.iter_mut().find(|s| s.name == name) {
            Some(existing) => existing.slots = slots,
            None => c.state.saved.push(SavedComparison { name: name.clone(), slots }),
        }
        c.status = format!("Saved '{name}'");
        w.global::<Backend>().set_save_name(ss(""));
        persist(&mut c);
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_load_saved, |w, c, i: i32| {
        if let Some(sv) = c.state.saved.get(i.max(0) as usize).cloned() {
            let wanted = sv.slots.len();
            c.state.slots = sv.slots;
            c.sel = 0;
            c.ensure_slots();
            let missing = wanted.saturating_sub(c.state.slots.len());
            c.status = if missing > 0 { format!("Loaded '{}' ({missing} car(s) no longer exist)", sv.name) } else { format!("Loaded '{}'", sv.name) };
            c.dirty = true;
            w.global::<Backend>().set_dialog(0);
            w.set_tab(1);
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_delete_saved, |w, c, i: i32| {
        let i = i.max(0) as usize;
        if i < c.state.saved.len() {
            c.state.saved.remove(i);
            c.dirty = true;
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_export_open, |w, c, kind: i32| {
        c.export_kind = kind.clamp(0, 1) as u8;
        let text = export_text(&c);
        let b = w.global::<Backend>();
        b.set_export_text(ss(text));
        b.set_export_kind(c.export_kind as i32);
        b.set_dialog(2);
    });
    hook!(ui, ctx, saver, on_copy_export, |w, c| {
        let text = export_text(&c);
        c.status = match clipboard_set(&text) {
            Ok(()) => "Copied to clipboard".into(),
            Err(e) => e,
        };
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_save_export, |w, c| {
        let text = export_text(&c);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let name = format!("pitlane-{stamp}.{}", if c.export_kind == 1 { "csv" } else { "txt" });
        c.status = match c.store.write_export(&name, text.as_bytes()) {
            Ok(p) => format!("Saved {}", p.display()),
            Err(e) => format!("Export failed: {e}"),
        };
        refresh(&w, &c);
    });
    {
        let wk = ui.as_weak();
        let cx = ctx.clone();
        ui.global::<Backend>().on_export_png(move || {
            let Some(w) = wk.upgrade() else { return };
            w.global::<Backend>().set_dialog(0);
            w.set_tab(1);
            let wk2 = wk.clone();
            let cx2 = Rc::downgrade(&cx);
            // let the comparison view lay out and paint before taking the snapshot
            Timer::single_shot(Duration::from_millis(250), move || {
                let (Some(w), Some(cx)) = (wk2.upgrade(), cx2.upgrade()) else { return };
                let msg = save_png(&w, &cx);
                if let Ok(mut c) = cx.try_borrow_mut() {
                    c.status = msg;
                    refresh(&w, &c);
                };
            });
        });
    }
    hook!(ui, ctx, saver, on_open_edit, |w, c, new_car: bool| {
        c.edit_new = new_car;
        fill_edit(&w, &c, new_car);
        w.global::<Backend>().set_dialog(3);
    });
    hook!(ui, ctx, saver, on_edit_save, |w, c, as_new: bool| {
        let as_new = as_new || c.edit_new;
        let (Some(base), Some(sl)) = (c.car(c.sel).cloned(), c.state.slots.get(c.sel).cloned()) else { return };
        match read_edit(&w, &base, as_new, sl.year) {
            Ok((car, year)) => {
                let mut ov = c.db.overrides().clone();
                ov.upsert(car.clone());
                match c.store.save_overrides(&ov) {
                    Ok(()) => {
                        rebuild_db(&mut c, ov);
                        c.assign(year, &car);
                        c.status = format!("Saved {}", car.trim);
                        w.global::<Backend>().set_dialog(0);
                    }
                    Err(e) => c.status = e,
                }
            }
            Err(e) => c.status = format!("Not saved: {e}"),
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_edit_reset, |w, c| {
        if let Some(id) = c.car(c.sel).map(|car| car.id.to_string()) {
            let mut ov = c.db.overrides().clone();
            ov.remove(&id);
            match c.store.save_overrides(&ov) {
                Ok(()) => {
                    rebuild_db(&mut c, ov);
                    c.status = "Override removed".into();
                    c.dirty = true;
                    w.global::<Backend>().set_dialog(0);
                }
                Err(e) => c.status = e,
            }
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_overrides_open, |w, c| {
        let b = w.global::<Backend>();
        b.set_overrides_text(ss(c.db.overrides().to_text().unwrap_or_else(|e| format!("// could not serialize: {e}"))));
        b.set_dialog(4);
    });
    hook!(ui, ctx, saver, on_overrides_copy, |w, c| {
        let text = w.global::<Backend>().get_overrides_text().to_string();
        c.status = match clipboard_set(&text) {
            Ok(()) => "Overrides copied".into(),
            Err(e) => e,
        };
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_overrides_import, |w, c| {
        let text = w.global::<Backend>().get_overrides_text().to_string();
        match Overrides::from_text(&text) {
            Ok(ov) => match c.store.save_overrides(&ov) {
                Ok(()) => {
                    let n = ov.cars.len();
                    rebuild_db(&mut c, ov);
                    c.status = format!("Imported {n} override/custom cars");
                    c.dirty = true;
                    w.global::<Backend>().set_dialog(0);
                }
                Err(e) => c.status = e,
            },
            Err(e) => c.status = format!("Import failed (nothing changed): {e}"),
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_settings_save, |w, c| {
        let b = w.global::<Backend>();
        let p = |s: SharedString| s.trim().parse::<f32>().ok().filter(|v| (0.0..=60.0).contains(v));
        match (p(b.get_loss_2wd_man()), p(b.get_loss_2wd_auto()), p(b.get_loss_awd_man()), p(b.get_loss_awd_auto()), p(b.get_loss_ev())) {
            (Some(a), Some(bb), Some(cc), Some(d), Some(e)) => {
                c.state.settings.loss = LossCfg { two_wd_manual: a, two_wd_auto: bb, awd_manual: cc, awd_auto: d, ev: e };
                c.status = "Loss settings applied".into();
                c.dirty = true;
                b.set_dialog(0);
            }
            _ => c.status = "Each value must be a number between 0 and 60".into(),
        }
        refresh(&w, &c);
    });
    hook!(ui, ctx, saver, on_clear_status, |w, c| {
        c.status.clear();
        refresh(&w, &c);
    });

    // initial loss settings text and first refresh
    {
        let c = ctx.borrow();
        let l = c.state.settings.loss;
        let b = ui.global::<Backend>();
        b.set_loss_2wd_man(ss(l.two_wd_manual.to_string()));
        b.set_loss_2wd_auto(ss(l.two_wd_auto.to_string()));
        b.set_loss_awd_man(ss(l.awd_manual.to_string()));
        b.set_loss_awd_auto(ss(l.awd_auto.to_string()));
        b.set_loss_ev(ss(l.ev.to_string()));
        refresh(&ui, &c);
    }
    {
        let cx = ctx.clone();
        ui.window().on_close_requested(move || {
            if let Ok(mut c) = cx.try_borrow_mut() {
                persist(&mut c);
            }
            slint::CloseRequestResponse::HideWindow
        });
    }
    // Keep the state alive as long as the caller holds the handle.
    let keep: Rc<dyn std::any::Any> = Rc::new((ctx, saver, searcher));
    Some((ui, keep))
}

pub fn run_with_dir(dir: PathBuf) {
    let t0 = std::time::Instant::now();
    let Some((ui, keep)) = build(dir) else { return };

    // PITLANE_TRACE=1 prints the time to the first rendered frame (used for the perf report).
    if std::env::var_os("PITLANE_TRACE").is_some() {
        let mut done = false;
        let hooked = ui.window().set_rendering_notifier(move |st, _| {
            if !done && matches!(st, slint::RenderingState::AfterRendering) {
                done = true;
                eprintln!("pitlane: first frame after {:.1} ms", t0.elapsed().as_secs_f64() * 1000.0);
            }
        });
        if hooked.is_err() {
            // The software renderer has no rendering notifier: report when the event loop first idles.
            Timer::single_shot(Duration::ZERO, move || {
                eprintln!("pitlane: first frame after {:.1} ms (event loop idle)", t0.elapsed().as_secs_f64() * 1000.0);
            });
        }
    }

    if let Err(e) = ui.run() {
        eprintln!("Pitlane: event loop error: {e}");
    }
    if let Some((ctx, _, _)) = keep.downcast_ref::<(Shared, Rc<Timer>, Rc<Timer>)>() {
        if let Ok(mut c) = ctx.try_borrow_mut() {
            persist(&mut c);
        }
    }
}

fn export_text(c: &Ctx) -> String {
    let views: Vec<SlotView> =
        c.state.slots.iter().filter_map(|sl| c.db.find(&sl.car_id).map(|car| SlotView { year: sl.year, car, eval: evaluate(car, &sl.mods, mod_defs()) })).collect();
    let t = build_table(&views, c.settings());
    if c.export_kind == 1 {
        to_csv(&t)
    } else {
        to_text(&t)
    }
}

/// Encodes a window snapshot as PNG (rendered only on demand, never kept around).
pub fn snapshot_png(w: &MainWindow) -> Result<Vec<u8>, String> {
    let snap = w.window().take_snapshot().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, snap.width(), snap.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(snap.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}

fn save_png(w: &MainWindow, cx: &Shared) -> String {
    let bytes = match snapshot_png(w) {
        Ok(b) => b,
        Err(e) => return format!("PNG export failed: {e}"),
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    match cx.try_borrow() {
        Ok(c) => match c.store.write_export(&format!("pitlane-{stamp}.png"), &bytes) {
            Ok(p) => format!("PNG saved: {}", p.display()),
            Err(e) => format!("PNG export failed: {e}"),
        },
        Err(_) => "PNG export failed: busy".into(),
    }
}

pub fn run_desktop() {
    run_with_dir(Store::default_dir());
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(app: slint::android::AndroidApp) {
    if slint::android::init(app.clone()).is_err() {
        return;
    }
    // app-private storage (/data/data/<package>/files); never shared storage
    let dir = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp")).join("pitlane");
    run_with_dir(dir);
}
