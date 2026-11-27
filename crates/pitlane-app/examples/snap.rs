//! Headless layout check: renders the real UI with Slint's software renderer at phone, tablet and
//! desktop sizes and writes PNGs. No window system or GPU needed.
//!
//!   cargo run --release -p pitlane-app --example snap --no-default-features --features renderer-software -- <out-dir>
use pitlane::{Backend, MainWindow};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, PhysicalSize};
use std::path::PathBuf;
use std::rc::Rc;

struct Headless(Rc<MinimalSoftwareWindow>);

impl Platform for Headless {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn render(win: &MinimalSoftwareWindow, ui: &MainWindow, w: u32, h: u32, scale: f32, path: &PathBuf) {
    win.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: scale });
    ui.window().set_size(PhysicalSize::new(w, h));
    win.request_redraw();
    // two passes so layouts depending on the first frame's geometry settle
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        let mut buf = vec![slint::Rgb8Pixel::default(); (w * h) as usize];
        win.draw_if_needed(|r| {
            r.render(&mut buf, w as usize);
        });
        win.request_redraw();
        let bytes: Vec<u8> = buf.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        let f = std::fs::File::create(path).expect("png");
        let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
        e.set_color(png::ColorType::Rgb);
        e.set_depth(png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&bytes).unwrap();
    }
}

fn main() {
    let out = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "snapshots".into()));
    std::fs::create_dir_all(&out).unwrap();
    let data = std::env::temp_dir().join(format!("pitlane-snap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    // three cars, one modded, so every view has real content (SNAP_EMPTY=1: the empty first-run state)
    let empty = std::env::var_os("SNAP_EMPTY").is_some();
    if !empty { std::fs::write(
        data.join("state.ron"),
        r#"(slots: [
            (year: 2018, car_id: "ford-mustang-s550-2015-2023-gt-5-0-2018", mods: [(id: "intake", value: 0.0), (id: "tune1", value: 0.0), (id: "nitrous", value: 75.0)]),
            (year: 2017, car_id: "honda-civic-10th-gen-2016-21-type-r-2-0t-fk8", mods: []),
            (year: 2021, car_id: "toyota-supra-a90-gr-supra-2020-3-0-b58-2021", mods: []),
        ])"#,
    )
    .unwrap(); }

    let win = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(win.clone()))).unwrap();
    let (ui, _keep) = pitlane::build(data.clone()).expect("ui");
    ui.show().unwrap();

    // (name, logical width, logical height, scale)
    let sizes = [
        ("phone-small-320x568", 320, 568, 2.0),
        ("phone-360x780", 360, 780, 3.0),
        ("phone-landscape-780x360", 780, 360, 3.0),
        ("tablet-portrait-800x1280", 800, 1280, 2.0),
        ("tablet-landscape-1280x800", 1280, 800, 2.0),
        ("desktop-1100x760", 1100, 760, 1.0),
    ];
    let only: Option<String> = std::env::args().nth(2);
    for (name, lw, lh, scale) in sizes {
        if only.as_deref().map(|o| !name.contains(o)).unwrap_or(false) {
            continue;
        }
        let (pw, ph) = ((lw as f32 * scale) as u32, (lh as f32 * scale) as u32);
        for (tab, tname) in [(0, "build"), (1, "compare")] {
            ui.set_tab(tab);
            ui.global::<Backend>().set_dialog(0);
            render(&win, &ui, pw, ph, scale, &out.join(format!("{name}-{tname}.png")));
        }
        ui.global::<Backend>().invoke_open_edit(false);
        render(&win, &ui, pw, ph, scale, &out.join(format!("{name}-edit-dialog.png")));
        ui.global::<Backend>().set_dialog(0);
        // worst case: full-window redraws (a real window only repaints dirty regions)
        let mut buf = vec![slint::Rgb8Pixel::default(); (pw * ph) as usize];
        let mut times = Vec::new();
        for i in 0..20 {
            ui.set_tab(i % 2);
            let t = std::time::Instant::now();
            win.request_redraw();
            win.draw_if_needed(|r| {
                r.render(&mut buf, pw as usize);
            });
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("{name} ({pw}x{ph} px): full redraw median {:.1} ms, worst {:.1} ms", times[10], times[19]);
    }
    let _ = std::fs::remove_dir_all(&data);
}
