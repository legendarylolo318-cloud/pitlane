//! Pitlane UI: Slint front-end over `pitlane-core`. Shared by desktop (`main.rs`) and Android (`android_main`).

use pitlane_core::compare::{build_table, car_title, to_csv, to_text, SlotView};
use pitlane_core::search::{search, Hit};
use pitlane_core::store::{AppState, Favorite, SavedComparison, SlotState, Store};
use pitlane_core::units::*;
use pitlane_core::*;
use slint::{Color, ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

slint::include_modules!();

const MAX_SLOTS: usize = 5;
const SLOT_COLORS: [(u8, u8, u8); MAX_SLOTS] = [(0x4e, 0x79, 0xa7), (0xf2, 0x8e, 0x2b), (0x59, 0xa1, 0x4f), (0xb0, 0x7a, 0xa1), (0xe1, 0x57, 0x59)];
const DATA_NOTE: &str = "Data accuracy: figures are approximate. 'verified' = matches commonly published factory specs as recalled by the author (not machine-checked); 'estimated' = approximate. 0-60 times and weights are the least reliable. Mod gains are typical ranges, not measurements.";

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
    Err("Clipboard is not wired up on Android yet: long-press the text box, Select all, Copy.".into())
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
    gain_model: Rc<VecModel<ModRow>>,
    neg_model: Rc<VecModel<ModRow>>,
}

type Shared = Rc<RefCell<Ctx>>;

fn year_of_index(i: i32) -> u16 {
    (YEAR_MAX as i32 - i.max(0)).clamp(YEAR_MIN as i32, YEAR_MAX as i32) as u16
}

fn vm<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn ss(s: impl AsRef<str>) -> SharedString {
    SharedString::from(s.as_ref())
}

fn default_slot(db: &Db) -> Option<SlotState> {
    let c = db.find("ford-mustang-s550-2015-2023-gt-5-0-2018").or_else(|| db.all_cars().next())?;
    let year = if c.covers_year(2018) { 2018 } else { c.year_end };
    Some(SlotState { year, car_id: c.id.clone(), mods: vec![] })
}

impl Ctx {
    fn ensure_slots(&mut self) {
        let db = &self.db;
        self.state.slots.retain(|s| db.find(&s.car_id).is_some());
        self.state.slots.truncate(MAX_SLOTS);
        if self.state.slots.is_empty() {
            if let Some(s) = default_slot(&self.db) {
                self.state.slots.push(s);
            }
        }
        self.sel = self.sel.min(self.state.slots.len().saturating_sub(1));
    }

    fn car(&self, slot: usize) -> Option<Car> {
        self.state.slots.get(slot).and_then(|s| self.db.find(&s.car_id)).cloned()
    }

    /// Resolve the picker cascade: keep the preferred make/model/trim when they exist for `year`.
    fn resolve(&self, year: u16, make: Option<&str>, model: Option<&str>, trim: Option<&str>) -> Option<Car> {
        let makes = self.db.makes_for_year(year);
        let mk = make.filter(|m| makes.contains(m)).or_else(|| makes.first().copied())?;
        let models = self.db.models_for(year, mk);
        let md = model.filter(|m| models.contains(m)).or_else(|| models.first().copied())?;
        let trims = self.db.trims_for(year, mk, md);
        trims.iter().find(|c| Some(c.trim.as_str()) == trim).or_else(|| trims.first()).map(|c| (*c).clone())
    }

    fn assign(&mut self, year: u16, car: &Car) {
        if let Some(s) = self.state.slots.get_mut(self.sel) {
            s.year = year;
            s.car_id = car.id.clone();
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
        car.transmissions.join("/"),
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

fn mod_rows(car: &Car, slot: &SlotState, kind: ModKind, s: &Settings) -> Vec<ModRow> {
    let defs = mod_defs();
    let full = evaluate(car, &slot.mods, defs);
    defs.iter()
        .filter(|d| d.kind == kind)
        .map(|d| {
            let sel = slot.mods.iter().find(|m| m.id == d.id);
            let (value, min, max, step, label, unit) = match &d.slider {
                Some(sl) => (sel.map(|m| m.value).unwrap_or(sl.default), sl.min, sl.max, sl.step, sl.label.clone(), sl.unit.clone()),
                None => (0.0, 0.0, 1.0, 1.0, String::new(), String::new()),
            };
            let gain = if sel.is_some() {
                let rest: Vec<ModSel> = slot.mods.iter().filter(|m| m.id != d.id).cloned().collect();
                gain_text(car, &full, &evaluate(car, &rest, defs), s)
            } else {
                String::new()
            };
            let warning = full.warnings.iter().filter(|w| w.mod_id == d.id).map(|w| w.text.clone()).collect::<Vec<_>>().join(" ");
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

fn set_or_replace(model: &Rc<VecModel<ModRow>>, rows: Vec<ModRow>, full: bool) {
    if !full && model.row_count() == rows.len() {
        for (i, r) in rows.into_iter().enumerate() {
            model.set_row_data(i, r);
        }
    } else {
        model.set_vec(rows);
    }
}

// ---------------------------------------------------------------- refresh

fn refresh(ui: &MainWindow, c: &Ctx, full: bool) {
    let b = ui.global::<Backend>();
    let s = c.settings();
    let cars: Vec<Option<Car>> = (0..c.state.slots.len()).map(|i| c.car(i)).collect();

    // slot chips
    let slots: Vec<SlotInfo> = c
        .state
        .slots
        .iter()
        .zip(&cars)
        .enumerate()
        .map(|(i, (sl, car))| {
            let (r, g, bl) = SLOT_COLORS[i % MAX_SLOTS];
            SlotInfo {
                title: ss(car.as_ref().map(|x| car_title(sl.year, x)).unwrap_or_default()),
                subtitle: ss(car.as_ref().map(|x| x.trim.clone()).unwrap_or_default()),
                modded: !sl.mods.is_empty(),
                color: Color::from_rgb_u8(r, g, bl),
            }
        })
        .collect();
    b.set_slots(vm(slots.clone()));
    b.set_selected(c.sel as i32);

    // pickers for the selected slot
    if let (Some(slot), Some(Some(car))) = (c.state.slots.get(c.sel), cars.get(c.sel)) {
        let years: Vec<SharedString> = (YEAR_MIN..=YEAR_MAX).rev().map(|y| ss(y.to_string())).collect();
        b.set_years(vm(years));
        b.set_year_index((YEAR_MAX - slot.year) as i32);
        let makes = c.db.makes_for_year(slot.year);
        b.set_make_index(makes.iter().position(|m| *m == car.make).unwrap_or(0) as i32);
        b.set_makes(vm(makes.iter().map(ss).collect()));
        let models = c.db.models_for(slot.year, &car.make);
        b.set_model_index(models.iter().position(|m| *m == car.model).unwrap_or(0) as i32);
        b.set_models(vm(models.iter().map(ss).collect()));
        let trims = c.db.trims_for(slot.year, &car.make, &car.model);
        b.set_trim_index(trims.iter().position(|t| t.id == car.id).unwrap_or(0) as i32);
        b.set_trims(vm(trims.iter().map(|t| ss(&t.trim)).collect()));

        let ev = evaluate(car, &slot.mods, mod_defs());
        b.set_selected_title(ss(car_title(slot.year, car)));
        b.set_stock_line(ss(stock_line(car, &s)));
        b.set_mod_line(ss(mod_line(car, &ev, &s)));
        b.set_is_fav(c.state.favorites.iter().any(|f| f.car_id == car.id && f.year == slot.year));
        set_or_replace(&c.gain_model, mod_rows(car, slot, ModKind::Gain, &s), full);
        set_or_replace(&c.neg_model, mod_rows(car, slot, ModKind::Negative, &s), full);
    } else {
        b.set_selected_title(ss("No car selected"));
        b.set_stock_line(ss(""));
        b.set_mod_line(ss(""));
    }
    b.set_gain_mods(ModelRc::from(c.gain_model.clone() as Rc<dyn Model<Data = ModRow>>));
    b.set_neg_mods(ModelRc::from(c.neg_model.clone() as Rc<dyn Model<Data = ModRow>>));
    b.set_warnings(vm(Vec::<SharedString>::new()));

    // comparison
    let evals: Vec<Option<(Car, Eval)>> = c
        .state
        .slots
        .iter()
        .zip(&cars)
        .map(|(sl, car)| car.as_ref().map(|x| (x.clone(), evaluate(x, &sl.mods, mod_defs()))))
        .collect();
    let views: Vec<SlotView> = c
        .state
        .slots
        .iter()
        .zip(&evals)
        .filter_map(|(sl, e)| e.as_ref().map(|(car, ev)| SlotView { year: sl.year, car, eval: ev.clone() }))
        .collect();
    let t = build_table(&views, s);
    let cell = |x: &pitlane_core::compare::Cell| CellData { text: ss(&x.text), sub: ss(&x.sub), best: x.best, frac: x.frac };
    let row = |r: &pitlane_core::compare::Row| RowData { label: ss(&r.label), cells: vm(r.cells.iter().map(cell).collect()) };
    let headers: Vec<SlotInfo> = t
        .headers
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let (r, g, bl) = SLOT_COLORS[i % MAX_SLOTS];
            SlotInfo { title: ss(&h.title), subtitle: ss(&h.subtitle), modded: h.modded, color: Color::from_rgb_u8(r, g, bl) }
        })
        .collect();
    b.set_headers(vm(headers));
    b.set_rows(vm(t.rows.iter().map(row).collect()));
    b.set_compact_rows(vm(t.rows.iter().filter(|r| r.compact).map(row).collect()));
    b.set_charts(vm(t.charts.iter().map(|ch| ChartData { title: ss(&ch.title), bars: vm(ch.bars.iter().map(cell).collect()) }).collect()));

    // toolbar & lists
    b.set_units_text(ss(match s.system {
        UnitSystem::Imperial => "Imperial",
        UnitSystem::Metric => "Metric",
    }));
    b.set_power_text(ss(power_label(s.power)));
    b.set_wheel_text(ss(if s.wheel { "Wheel (est.)" } else { "Crank" }));
    b.set_status(ss(&c.status));
    b.set_data_note(ss(DATA_NOTE));
    b.set_search_hits(vm(c.hits.iter().map(|h| ss(&h.label)).collect()));
    b.set_favorites(vm(c
        .state
        .favorites
        .iter()
        .map(|f| ss(c.db.find(&f.car_id).map(|x| format!("{} {} {} {}", f.year, x.make, x.model, x.trim)).unwrap_or_else(|| format!("(missing) {}", f.car_id))))
        .collect()));
    b.set_saved(vm(c.state.saved.iter().map(|sv| SavedItem { name: ss(&sv.name), detail: ss(format!("{} cars", sv.slots.len())) }).collect()));
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
    b.set_ed_make(ss(&car.make));
    b.set_ed_model(ss(&car.model));
    b.set_ed_trim(ss(if new_car { format!("{} (custom)", car.trim) } else { car.trim.clone() }));
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
    car.make = b.get_ed_make().trim().to_string();
    car.model = b.get_ed_model().trim().to_string();
    car.trim = b.get_ed_trim().trim().to_string();
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
    }
    if as_new {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        car.id = format!("custom-{}-{}-{}", model::slug(&car.make), model::slug(&car.model), stamp);
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
        // overriding a stat keeps the id; the user-entered numbers are the user's responsibility
        if !car.covers_year(year_now) {
            return Err("Year does not match this car's years".into());
        }
        car.confidence = Confidence::Estimated;
    }
    validate_car(&car)?;
    Ok((car, if as_new { year } else { year_now }))
}

// ---------------------------------------------------------------- wiring

macro_rules! hook {
    ($ui:ident, $ctx:ident, $reg:ident, |$w:ident, $c:ident $(, $a:ident : $t:ty)*| $body:block) => {{
        let wk = $ui.as_weak();
        let cx = $ctx.clone();
        $ui.global::<Backend>().$reg(move |$($a : $t),*| {
            let Some($w) = wk.upgrade() else { return };
            #[allow(unused_mut)]
            let Ok(mut $c) = cx.try_borrow_mut() else { return };
            $body
        });
    }};
}

fn load_slot_from(c: &mut Ctx, year: u16, car: &Car, keep_mods: bool) {
    let mods = if keep_mods { c.state.slots.get(c.sel).map(|s| s.mods.clone()).unwrap_or_default() } else { vec![] };
    if let Some(s) = c.state.slots.get_mut(c.sel) {
        s.mods = mods;
    }
    c.assign(year, car);
}

pub fn run_with_dir(dir: PathBuf) {
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
        gain_model: Rc::new(VecModel::default()),
        neg_model: Rc::new(VecModel::default()),
    };
    ctx.ensure_slots();
    let ctx: Shared = Rc::new(RefCell::new(ctx));

    let ui = match MainWindow::new() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("Pitlane: could not create the window: {e}");
            return;
        }
    };

    hook!(ui, ctx, on_select_slot, |w, c, i: i32| {
        c.sel = (i.max(0) as usize).min(c.state.slots.len().saturating_sub(1));
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_add_slot, |w, c| {
        if c.state.slots.len() >= MAX_SLOTS {
            c.status = "Maximum of 5 cars".into();
        } else if let Some(cur) = c.state.slots.get(c.sel).cloned() {
            c.state.slots.push(SlotState { mods: vec![], ..cur });
            c.sel = c.state.slots.len() - 1;
            c.dirty = true;
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_duplicate_slot, |w, c| {
        if c.state.slots.len() >= MAX_SLOTS {
            c.status = "Maximum of 5 cars".into();
        } else if let Some(cur) = c.state.slots.get(c.sel).cloned() {
            let at = c.sel + 1;
            c.state.slots.insert(at, cur);
            c.sel += 1;
            c.dirty = true;
            c.status = "Duplicated - change its mods to compare stock vs modded".into();
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_remove_slot, |w, c| {
        if c.state.slots.len() > 1 {
            let i = c.sel;
            c.state.slots.remove(i);
            c.sel = c.sel.min(c.state.slots.len() - 1);
            c.dirty = true;
        } else {
            c.status = "At least one car is needed".into();
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_year_picked, |w, c, i: i32| {
        let year = year_of_index(i);
        if let Some(cur) = c.car(c.sel) {
            if let Some(car) = c.resolve(year, Some(&cur.make), Some(&cur.model), Some(&cur.trim)) {
                load_slot_from(&mut c, year, &car, true);
            }
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_make_picked, |w, c, i: i32| {
        if let Some(sl) = c.state.slots.get(c.sel).cloned() {
            let makes = c.db.makes_for_year(sl.year);
            let pick = makes.get(i.max(0) as usize).map(|m| m.to_string());
            if let Some(car) = c.resolve(sl.year, pick.as_deref(), None, None) {
                load_slot_from(&mut c, sl.year, &car, true);
            }
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_model_picked, |w, c, i: i32| {
        if let (Some(sl), Some(cur)) = (c.state.slots.get(c.sel).cloned(), c.car(c.sel)) {
            let models: Vec<String> = c.db.models_for(sl.year, &cur.make).iter().map(|m| m.to_string()).collect();
            if let Some(car) = c.resolve(sl.year, Some(&cur.make), models.get(i.max(0) as usize).map(|s| s.as_str()), None) {
                load_slot_from(&mut c, sl.year, &car, true);
            }
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_trim_picked, |w, c, i: i32| {
        if let (Some(sl), Some(cur)) = (c.state.slots.get(c.sel).cloned(), c.car(c.sel)) {
            let pick = c.db.trims_for(sl.year, &cur.make, &cur.model).get(i.max(0) as usize).map(|t| (*t).clone());
            if let Some(car) = pick {
                load_slot_from(&mut c, sl.year, &car, true);
            }
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_mod_toggled, |w, c, id: SharedString, on: bool| {
        let sel = c.sel;
        if let Some(sl) = c.state.slots.get_mut(sel) {
            sl.mods.retain(|m| m.id != id.as_str());
            if on {
                let default = mod_defs().iter().find(|d| d.id == id.as_str()).and_then(|d| d.slider.as_ref().map(|s| s.default)).unwrap_or(0.0);
                sl.mods.push(ModSel { id: id.to_string(), value: default });
            }
        }
        c.dirty = true;
        refresh(&w, &c, false);
    });
    hook!(ui, ctx, on_mod_value, |w, c, id: SharedString, v: f32| {
        let step = mod_defs().iter().find(|d| d.id == id.as_str()).and_then(|d| d.slider.as_ref().map(|s| s.step)).unwrap_or(1.0).max(f32::EPSILON);
        let sel = c.sel;
        if let Some(m) = c.state.slots.get_mut(sel).and_then(|sl| sl.mods.iter_mut().find(|m| m.id == id.as_str())) {
            if v.is_finite() {
                m.value = (v / step).round() * step;
            }
        }
        c.dirty = true;
        refresh(&w, &c, false);
    });
    hook!(ui, ctx, on_search_edited, |w, c, t: SharedString| {
        c.hits = search(&c.db, t.as_str(), 8);
        w.global::<Backend>().set_search_hits(vm(c.hits.iter().map(|h| ss(&h.label)).collect()));
    });
    hook!(ui, ctx, on_search_pick, |w, c, i: i32| {
        if let Some(h) = c.hits.get(i.max(0) as usize).cloned() {
            if let Some(car) = c.db.find(&h.car_id).cloned() {
                load_slot_from(&mut c, h.year, &car, true);
                c.status = format!("Loaded {}", h.label);
            }
        }
        c.hits.clear();
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_toggle_units, |w, c| {
        c.state.settings.system = if c.state.settings.system == UnitSystem::Imperial { UnitSystem::Metric } else { UnitSystem::Imperial };
        c.dirty = true;
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_cycle_power, |w, c| {
        c.state.settings.power = match c.state.settings.power {
            PowerUnit::Hp => PowerUnit::Kw,
            PowerUnit::Kw => PowerUnit::Ps,
            PowerUnit::Ps => PowerUnit::Hp,
        };
        c.dirty = true;
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_toggle_wheel, |w, c| {
        c.state.settings.wheel = !c.state.settings.wheel;
        c.dirty = true;
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_toggle_fav, |w, c| {
        if let Some(sl) = c.state.slots.get(c.sel).cloned() {
            let before = c.state.favorites.len();
            c.state.favorites.retain(|f| !(f.car_id == sl.car_id && f.year == sl.year));
            if c.state.favorites.len() == before {
                c.state.favorites.push(Favorite { year: sl.year, car_id: sl.car_id });
            }
            c.dirty = true;
        }
        refresh(&w, &c, false);
    });
    hook!(ui, ctx, on_pick_fav, |w, c, i: i32| {
        if let Some(f) = c.state.favorites.get(i.max(0) as usize).cloned() {
            match c.db.find(&f.car_id).cloned() {
                Some(car) => {
                    load_slot_from(&mut c, f.year, &car, true);
                    w.global::<Backend>().set_dialog(0);
                    w.set_tab(0);
                }
                None => c.status = "That car is no longer in the data (override removed?)".into(),
            }
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_save_comparison, |w, c| {
        let typed = w.global::<Backend>().get_save_name().trim().to_string();
        let name = if typed.is_empty() { format!("Comparison {}", c.state.saved.len() + 1) } else { typed };
        let slots = c.state.slots.clone();
        c.state.saved.push(SavedComparison { name: name.clone(), slots });
        c.status = format!("Saved '{name}'");
        w.global::<Backend>().set_save_name(ss(""));
        persist(&mut c);
        refresh(&w, &c, false);
    });
    hook!(ui, ctx, on_load_saved, |w, c, i: i32| {
        if let Some(sv) = c.state.saved.get(i.max(0) as usize).cloned() {
            c.state.slots = sv.slots;
            c.sel = 0;
            c.ensure_slots();
            c.status = format!("Loaded '{}'", sv.name);
            c.dirty = true;
            w.global::<Backend>().set_dialog(0);
            w.set_tab(1);
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_delete_saved, |w, c, i: i32| {
        let i = i.max(0) as usize;
        if i < c.state.saved.len() {
            c.state.saved.remove(i);
            c.dirty = true;
        }
        refresh(&w, &c, false);
    });
    hook!(ui, ctx, on_export_open, |w, c, kind: i32| {
        c.export_kind = kind as u8;
        let text = export_text(&c);
        let b = w.global::<Backend>();
        b.set_export_text(ss(text));
        b.set_dialog(2);
    });
    hook!(ui, ctx, on_copy_export, |w, c| {
        let text = export_text(&c);
        c.status = match clipboard_set(&text) {
            Ok(()) => "Copied to clipboard".into(),
            Err(e) => e,
        };
        refresh(&w, &c, false);
    });
    {
        let wk = ui.as_weak();
        let cx = ctx.clone();
        ui.global::<Backend>().on_export_png(move || {
            let Some(w) = wk.upgrade() else { return };
            w.global::<Backend>().set_dialog(0);
            w.set_tab(1);
            let wk2 = wk.clone();
            let cx2 = cx.clone();
            slint::Timer::single_shot(std::time::Duration::from_millis(250), move || {
                let Some(w) = wk2.upgrade() else { return };
                let msg = save_png(&w, &cx2);
                if let Ok(mut c) = cx2.try_borrow_mut() {
                    c.status = msg;
                    refresh(&w, &c, false);
                }
            });
        });
    }
    hook!(ui, ctx, on_open_edit, |w, c, new_car: bool| {
        c.edit_new = new_car;
        fill_edit(&w, &c, new_car);
        w.global::<Backend>().set_dialog(3);
    });
    hook!(ui, ctx, on_edit_save, |w, c, as_new: bool| {
        let as_new = as_new || c.edit_new;
        let (Some(base), Some(sl)) = (c.car(c.sel), c.state.slots.get(c.sel).cloned()) else { return };
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
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_edit_reset, |w, c| {
        if let Some(car) = c.car(c.sel) {
            let mut ov = c.db.overrides().clone();
            ov.remove(&car.id);
            match c.store.save_overrides(&ov) {
                Ok(()) => {
                    rebuild_db(&mut c, ov);
                    c.status = "Override removed".into();
                    w.global::<Backend>().set_dialog(0);
                }
                Err(e) => c.status = e,
            }
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_overrides_open, |w, c| {
        let b = w.global::<Backend>();
        b.set_overrides_text(ss(c.db.overrides().to_text().unwrap_or_else(|e| format!("// could not serialize: {e}"))));
        b.set_dialog(4);
    });
    hook!(ui, ctx, on_overrides_copy, |w, c| {
        let text = w.global::<Backend>().get_overrides_text().to_string();
        c.status = match clipboard_set(&text) {
            Ok(()) => "Overrides copied".into(),
            Err(e) => e,
        };
        refresh(&w, &c, false);
    });
    hook!(ui, ctx, on_overrides_import, |w, c| {
        let text = w.global::<Backend>().get_overrides_text().to_string();
        match Overrides::from_text(&text) {
            Ok(ov) => match c.store.save_overrides(&ov) {
                Ok(()) => {
                    let n = ov.cars.len();
                    rebuild_db(&mut c, ov);
                    c.status = format!("Imported {n} override/custom cars");
                    w.global::<Backend>().set_dialog(0);
                }
                Err(e) => c.status = e,
            },
            Err(e) => c.status = format!("Import failed (nothing changed): {e}"),
        }
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_settings_save, |w, c| {
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
        refresh(&w, &c, true);
    });
    hook!(ui, ctx, on_clear_status, |w, c| {
        c.status.clear();
        refresh(&w, &c, false);
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
        refresh(&ui, &c, true);
    }

    // clear stale dialog status whenever a dialog is closed via the Close button is handled by status text; autosave:
    let timer = slint::Timer::default();
    {
        let cx = ctx.clone();
        timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(2), move || {
            if let Ok(mut c) = cx.try_borrow_mut() {
                if c.dirty {
                    persist(&mut c);
                }
            }
        });
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

    if let Err(e) = ui.run() {
        eprintln!("Pitlane: event loop error: {e}");
    }
    let last = ctx.try_borrow_mut();
    if let Ok(mut c) = last {
        persist(&mut c);
    }
}

fn export_text(c: &Ctx) -> String {
    let s = c.settings();
    let cars: Vec<Option<Car>> = (0..c.state.slots.len()).map(|i| c.car(i)).collect();
    let evals: Vec<(u16, Car, Eval)> = c
        .state
        .slots
        .iter()
        .zip(cars)
        .filter_map(|(sl, car)| car.map(|x| (sl.year, x.clone(), evaluate(&x, &sl.mods, mod_defs()))))
        .collect();
    let views: Vec<SlotView> = evals.iter().map(|(y, car, ev)| SlotView { year: *y, car, eval: ev.clone() }).collect();
    let t = build_table(&views, s);
    if c.export_kind == 1 {
        to_csv(&t)
    } else {
        to_text(&t)
    }
}

fn save_png(w: &MainWindow, cx: &Shared) -> String {
    let snap = match w.window().take_snapshot() {
        Ok(s) => s,
        Err(e) => return format!("PNG export failed: {e}"),
    };
    let (width, height) = (snap.width(), snap.height());
    let mut bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut bytes, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = match enc.write_header() {
            Ok(w) => w,
            Err(e) => return format!("PNG export failed: {e}"),
        };
        if let Err(e) = writer.write_image_data(snap.as_bytes()) {
            return format!("PNG export failed: {e}");
        }
    }
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
    // app-private storage
    let dir = app.internal_data_path().unwrap_or_else(|| PathBuf::from("/data/local/tmp")).join("pitlane");
    run_with_dir(dir);
}
