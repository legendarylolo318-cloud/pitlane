//! End-to-end UI test: drives the real window headlessly (software renderer, no display needed).
use pitlane::{Backend, MainWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Key, Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Model, PhysicalSize, SharedString};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

struct Headless(Rc<MinimalSoftwareWindow>);
impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn pump(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
    slint::platform::update_timers_and_animations();
}

fn strings(m: slint::ModelRc<SharedString>) -> Vec<String> {
    m.iter().map(|s| s.to_string()).collect()
}

fn open(dir: &Path) -> (MainWindow, Rc<dyn std::any::Any>) {
    let (ui, keep) = pitlane::build(dir.to_path_buf()).expect("ui");
    ui.window().set_size(PhysicalSize::new(360, 780));
    ui.show().unwrap();
    (ui, keep)
}

#[test]
fn app_flows() {
    let win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(win.clone()))).unwrap();
    let dir: PathBuf = std::env::temp_dir().join(format!("pitlane-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (ui, keep) = open(&dir);
    let b = ui.global::<Backend>();
    // --- the comparison starts empty
    assert_eq!(b.get_slots().row_count(), 0);
    assert!(!b.get_has_car());
    assert_eq!(b.get_headers().row_count(), 0);
    b.invoke_remove_slot();
    b.invoke_open_edit(false);
    assert_eq!(b.get_dialog(), 0, "nothing to edit yet");

    // --- searching just the car adds it with its first engine for that year
    b.invoke_search_edited("2005 ford ranger".into());
    pump(200);
    assert_eq!(strings(b.get_search_hits()), vec!["2005 Ford Ranger 2.3 Duratec I4".to_string()]);
    b.invoke_search_pick(0);
    assert_eq!(b.get_slots().row_count(), 1);
    assert!(b.get_has_car());
    assert_eq!(b.get_selected_title(), "2005 Ford Ranger");
    let trims = strings(b.get_trims());
    assert_eq!(trims[b.get_trim_index() as usize], "2.3 Duratec I4");
    // an engine word switches the engine picker to that engine
    b.invoke_search_edited("2005 ranger 4.0".into());
    pump(200);
    b.invoke_search_pick(0);
    let trims = strings(b.get_trims());
    assert_eq!(trims[b.get_trim_index() as usize], "4.0 SOHC V6");
    assert_eq!(b.get_slots().row_count(), 1, "search replaces the selected car");
    // --- drivetrain: each engine is listed once; trims sold both ways get a 4WD / RWD choice
    assert_eq!(trims.iter().filter(|t| t.as_str() == "4.0 SOHC V6").count(), 1);
    assert_eq!(strings(b.get_drives()), vec!["4WD".to_string(), "RWD".to_string()]);
    assert_eq!(b.get_drive_index(), 0);
    let w4 = b.get_stats().row_data(3).unwrap().value;
    b.invoke_drive_picked(1);
    assert_eq!(b.get_drive_index(), 1);
    assert!(b.get_detail_line().starts_with("RWD"), "{}", b.get_detail_line());
    assert_ne!(b.get_stats().row_data(3).unwrap().value, w4, "2WD is lighter");
    // switching to an engine that was 2WD-only shows no choice
    b.invoke_trim_picked(trims.iter().position(|t| t == "2.3 Duratec I4").unwrap() as i32);
    assert!(b.get_drives().row_count() <= 1);
    // and back to the V6: the chosen RWD is kept
    b.invoke_trim_picked(trims.iter().position(|t| t == "3.0 V6").unwrap() as i32);
    assert!(b.get_detail_line().starts_with("RWD"), "{}", b.get_detail_line());
    b.invoke_drive_picked(1);
    assert!(b.get_detail_line().starts_with("4WD"), "{}", b.get_detail_line());
    // the year picker keeps the drivetrain too
    let years = strings(b.get_years());
    b.invoke_year_picked(years.iter().position(|y| y == "2008").unwrap() as i32);
    assert!(b.get_detail_line().starts_with("4WD"), "{}", b.get_detail_line());

    b.invoke_search_edited("2018 mustang gt".into());
    pump(200);
    b.invoke_search_pick(0);
    assert_eq!(b.get_selected_title(), "2018 Ford Mustang");

    // --- cascading pickers: a year without that model resets model/trim to valid ones
    let years = strings(b.get_years());
    b.invoke_year_picked(years.iter().position(|y| y == "1990").unwrap() as i32);
    assert_eq!(b.get_selected_title(), "1990 Ford Mustang", "Mustang exists in 1990, so it is kept");
    assert!(strings(b.get_trims()).iter().any(|t| t.contains("5.0")));
    let makes = strings(b.get_makes());
    b.invoke_make_picked(makes.iter().position(|m| m == "Geo").unwrap() as i32);
    assert!(b.get_selected_title().starts_with("1990 Geo "), "{}", b.get_selected_title());
    b.invoke_year_picked(years.iter().position(|y| y == "2020").unwrap() as i32);
    assert!(!b.get_selected_title().contains("Geo"), "Geo did not exist in 2020: {}", b.get_selected_title());
    // every picker shows something valid for the year
    let (mk, md, tr) = (strings(b.get_makes()), strings(b.get_models()), strings(b.get_trims()));
    assert!(!mk.is_empty() && !md.is_empty() && !tr.is_empty());
    assert!((b.get_make_index() as usize) < mk.len() && (b.get_model_index() as usize) < md.len() && (b.get_trim_index() as usize) < tr.len());

    // --- slots: max 5, remove keeps a valid selection, duplicates tie
    for _ in 0..6 {
        b.invoke_add_slot();
    }
    assert_eq!(b.get_slots().row_count(), 5);
    assert!(b.get_status().contains("Maximum"));
    b.invoke_select_slot(4);
    b.invoke_remove_slot();
    assert_eq!(b.get_slots().row_count(), 4);
    assert_eq!(b.get_selected(), 3);
    for _ in 0..4 {
        b.invoke_remove_slot();
    }
    assert_eq!(b.get_slots().row_count(), 0, "every car can be removed");
    assert!(!b.get_has_car());
    b.invoke_add_slot();
    assert_eq!(b.get_slots().row_count(), 1, "+ on an empty comparison adds a starter car");
    assert!(b.get_selected_title().starts_with("2026 "), "{}", b.get_selected_title());
    b.invoke_duplicate_slot();
    assert_eq!(b.get_slots().row_count(), 2);
    let rows = b.get_rows();
    assert!(rows.iter().all(|r| r.cells.iter().all(|c| !c.best)), "identical cars: no winner");

    // --- mods: toggling and slider snapping
    b.invoke_mod_toggled("nitrous".into(), true);
    b.invoke_mod_value("nitrous".into(), 112.0);
    let n = b.get_gain_mods().iter().find(|m| m.id == "nitrous").unwrap();
    assert!(n.enabled && n.value == 100.0, "snapped to the 25 hp step: {}", n.value);
    assert_eq!(b.get_active_mods(), 1);
    let power = b.get_stats().row_data(0).unwrap();
    assert!(power.changed && power.better && power.delta.starts_with('+'), "{power:?}");
    assert!(b.get_mod_note().starts_with("With mods"));
    assert!(b.get_rows().iter().next().unwrap().cells.iter().any(|c| c.best), "modded copy now wins");

    // --- search (debounced)
    b.invoke_search_edited("06 civic si".into());
    assert_eq!(b.get_search_hits().row_count(), 0, "not run until the debounce elapses");
    pump(200);
    assert!(b.get_search_hits().row_count() > 0);
    b.invoke_search_pick(0);
    assert_eq!(b.get_selected_title(), "2006 Honda Civic");
    assert_eq!(b.get_search_hits().row_count(), 0);

    // --- favorites and saved comparisons
    b.invoke_toggle_fav();
    assert!(b.get_is_fav());
    assert_eq!(b.get_favorites().row_count(), 1);
    b.set_save_name("track day".into());
    b.invoke_save_comparison();
    b.set_save_name("track day".into());
    b.invoke_save_comparison();
    assert_eq!(b.get_saved().row_count(), 1, "same name replaces");

    // --- overrides: make/model changes are refused, stat edits are kept
    b.invoke_open_edit(false);
    b.set_ed_model("Not a Civic".into());
    b.invoke_edit_save(false);
    assert!(b.get_status().starts_with("Not saved"), "{}", b.get_status());
    b.invoke_open_edit(false);
    b.set_ed_hp("250".into());
    b.invoke_edit_save(false);
    let power = b.get_stats().row_data(0).unwrap();
    // this slot still carries the 100 hp nitrous shot (95 typical): the overridden 250 hp is the new stock
    assert_eq!((power.value.as_str(), power.delta.as_str()), ("345 hp", "+95 est."));
    // custom car with a new make
    b.invoke_open_edit(true);
    b.set_ed_make("Pitlane Motors".into());
    b.set_ed_model("Prototype".into());
    b.set_ed_year("2024".into());
    b.set_ed_hp("500".into());
    b.invoke_edit_save(true);
    assert_eq!(b.get_selected_title(), "2024 Pitlane Motors Prototype");
    assert!(dir.join("overrides.ron").exists());

    // --- exports
    b.invoke_export_open(1);
    assert!(b.get_export_text().starts_with("Metric,"));
    b.invoke_save_export();
    assert!(b.get_status().starts_with("Saved "), "{}", b.get_status());
    b.invoke_export_png();
    pump(300);
    assert!(b.get_status().starts_with("PNG saved"), "{}", b.get_status());
    let exports: Vec<_> = std::fs::read_dir(dir.join("exports")).unwrap().map(|e| e.unwrap().path()).collect();
    assert!(exports.iter().any(|p| p.extension().unwrap() == "csv") && exports.iter().any(|p| p.extension().unwrap() == "png"));

    // --- Android Back / Escape: closes a dialog, then leaves Compare, then is passed on
    b.set_dialog(6);
    ui.set_tab(1);
    win.dispatch_event(WindowEvent::KeyPressed { text: Key::Back.into() });
    assert_eq!(b.get_dialog(), 0);
    win.dispatch_event(WindowEvent::KeyPressed { text: Key::Escape.into() });
    assert_eq!(ui.get_tab(), 0);

    // --- autosave happens after the debounce; everything survives a restart
    pump(1700);
    drop(ui);
    drop(keep);
    let (ui, _keep) = open(&dir);
    let b = ui.global::<Backend>();
    assert_eq!(b.get_slots().row_count(), 2);
    assert_eq!(b.get_favorites().row_count(), 1);
    assert_eq!(b.get_saved().row_count(), 1);
    // the duplicated slot (#2) holds the custom car, with its mod still applied
    b.invoke_select_slot(1);
    assert_eq!(b.get_selected_title(), "2024 Pitlane Motors Prototype");
    assert!(strings(b.get_makes()).contains(&"Pitlane Motors".to_string()));
    assert!(b.get_gain_mods().iter().any(|m| m.id == "nitrous" && m.enabled));

    let _ = std::fs::remove_dir_all(&dir);
}
