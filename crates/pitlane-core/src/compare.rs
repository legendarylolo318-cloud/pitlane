//! Builds the comparison table / chart data (already formatted for display and export).

use crate::calc::Eval;
use crate::model::*;
use crate::units::*;

pub struct SlotView<'a> {
    pub year: u16,
    pub car: &'a Car,
    pub eval: Eval,
}

#[derive(Clone, Debug, Default)]
pub struct Cell {
    pub text: String,
    pub sub: String,
    pub best: bool,
    /// 0..1 bar fraction, or -1 if the row has no bar.
    pub frac: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub label: String,
    pub cells: Vec<Cell>,
    /// Included in the compact (phone portrait) table.
    pub compact: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Header {
    pub title: String,
    pub subtitle: String,
    pub modded: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Chart {
    pub title: String,
    pub bars: Vec<Cell>,
}

#[derive(Clone, Debug, Default)]
pub struct Table {
    pub headers: Vec<Header>,
    pub rows: Vec<Row>,
    pub charts: Vec<Chart>,
}

#[derive(Clone, Copy, PartialEq)]
enum Better {
    Higher,
    Lower,
    None,
}

pub fn car_title(year: u16, car: &Car) -> String {
    format!("{} {} {}", year, car.make, car.model)
}

fn signed(v: f32, decimals: usize) -> String {
    if v >= 0.0 {
        format!("+{v:.decimals$}")
    } else {
        format!("{v:.decimals$}")
    }
}

/// Values per slot: (shown value, stock value) in display units.
struct Metrics {
    hp: (f32, f32),
    tq: (f32, f32),
    zs: (f32, f32),
    p2w: (f32, f32),
    w2p: (f32, f32),
    weight: (f32, f32),
}

fn metrics(v: &SlotView, s: Settings) -> Metrics {
    let wf = if s.wheel { wheel_factor(v.car, v.eval.aspiration, &s.loss) } else { 1.0 };
    let wf0 = if s.wheel { wheel_factor(v.car, v.car.aspiration, &s.loss) } else { 1.0 };
    let hp1 = v.eval.hp.typ * wf;
    let hp0 = v.car.hp as f32 * wf0;
    let w1 = v.eval.weight_lbs;
    let w0 = v.car.weight_lbs as f32;
    Metrics {
        hp: (power_from_hp(s.power, hp1), power_from_hp(s.power, hp0)),
        tq: (torque_from_lbft(s.system, v.eval.torque.typ * wf), torque_from_lbft(s.system, v.car.torque as f32 * wf0)),
        zs: (accel_from_60(s.system, v.eval.zero_sixty.typ), accel_from_60(s.system, v.car.zero_sixty)),
        p2w: (power_per_ton(s, hp1, w1), power_per_ton(s, hp0, w0)),
        w2p: (weight_per_power(s, hp1, w1), weight_per_power(s, hp0, w0)),
        weight: (weight_from_lbs(s.system, w1), weight_from_lbs(s.system, w0)),
    }
}

fn mark_best(cells: &mut [Cell], vals: &[f32], better: Better) {
    if better == Better::None || vals.len() < 2 {
        return;
    }
    let best = match better {
        Better::Higher => vals.iter().cloned().fold(f32::MIN, f32::max),
        _ => vals.iter().cloned().fold(f32::MAX, f32::min),
    };
    for (c, v) in cells.iter_mut().zip(vals) {
        c.best = (v - best).abs() < best.abs() * 1e-4 + 1e-6;
    }
}

fn fracs(vals: &[f32], better: Better) -> Vec<f32> {
    match better {
        Better::Lower => {
            let min = vals.iter().cloned().fold(f32::MAX, f32::min).max(1e-6);
            vals.iter().map(|v| (min / v.max(1e-6)).clamp(0.0, 1.0)).collect()
        }
        _ => {
            let max = vals.iter().cloned().fold(f32::MIN, f32::max).max(1e-6);
            vals.iter().map(|v| (v / max).clamp(0.0, 1.0)).collect()
        }
    }
}

pub fn build_table(views: &[SlotView], s: Settings) -> Table {
    let mut t = Table::default();
    let ms: Vec<Metrics> = views.iter().map(|v| metrics(v, s)).collect();
    for v in views {
        t.headers.push(Header { title: car_title(v.year, v.car), subtitle: v.car.trim.clone(), modded: v.eval.modded });
    }
    let est = |v: &SlotView| if v.eval.modded { " est." } else { "" };

    // numeric rows with deltas
    struct Spec<'a> {
        label: String,
        unit: &'a str,
        dec: usize,
        better: Better,
        compact: bool,
        pick: fn(&Metrics) -> (f32, f32),
        sub_src: bool,
    }
    let pw = power_label(s.power);
    let specs = [
        Spec { label: if s.wheel { "Wheel power (est.)".into() } else { "Power (crank)".into() }, unit: pw, dec: 0, better: Better::Higher, compact: true, pick: |m| m.hp, sub_src: false },
        Spec { label: if s.wheel { "Wheel torque (est.)".into() } else { "Torque (crank)".into() }, unit: torque_label(s.system), dec: 0, better: Better::Higher, compact: true, pick: |m| m.tq, sub_src: false },
        Spec { label: accel_label(s.system).into(), unit: "s", dec: 1, better: Better::Lower, compact: true, pick: |m| m.zs, sub_src: true },
        Spec {
            label: match s.system {
                UnitSystem::Imperial => format!("Power-to-weight ({pw}/ton)"),
                UnitSystem::Metric => format!("Power-to-weight ({pw}/tonne)"),
            },
            unit: "",
            dec: 0,
            better: Better::Higher,
            compact: true,
            pick: |m| m.p2w,
            sub_src: false,
        },
        Spec { label: format!("Weight-to-power ({}/{pw})", weight_label(s.system)), unit: "", dec: 2, better: Better::Lower, compact: false, pick: |m| m.w2p, sub_src: false },
        Spec { label: "Curb weight".into(), unit: weight_label(s.system), dec: 0, better: Better::Lower, compact: false, pick: |m| m.weight, sub_src: false },
    ];

    let mut chart_idx = Vec::new();
    for (si, sp) in specs.iter().enumerate() {
        let vals: Vec<f32> = ms.iter().map(|m| (sp.pick)(m).0).collect();
        let fr = fracs(&vals, sp.better);
        let mut cells = Vec::new();
        for (i, v) in views.iter().enumerate() {
            let (now, stock) = (sp.pick)(&ms[i]);
            let unit = if sp.unit.is_empty() { String::new() } else { format!(" {}", sp.unit) };
            let text = format!("{:.*}{}{}", sp.dec, now, unit, est(v));
            let changed = v.eval.modded && (now - stock).abs() >= 0.5 * 10f32.powi(-(sp.dec as i32));
            let sub = if changed {
                format!("stock {:.*} ({})", sp.dec, stock, signed(now - stock, sp.dec))
            } else if sp.sub_src {
                v.car.zero_sixty_source.label().to_string()
            } else {
                String::new()
            };
            cells.push(Cell { text, sub, best: false, frac: fr[i] });
        }
        mark_best(&mut cells, &vals, sp.better);
        if si < 4 {
            chart_idx.push(t.rows.len());
        }
        t.rows.push(Row { label: sp.label.clone(), cells, compact: sp.compact });
    }

    // textual rows
    let mut text_row = |label: &str, compact: bool, f: &dyn Fn(&SlotView) -> String| {
        let cells = views.iter().map(|v| Cell { text: f(v), sub: String::new(), best: false, frac: -1.0 }).collect();
        t.rows.push(Row { label: label.into(), cells, compact });
    };
    text_row("Engine", true, &|v| {
        if v.car.aspiration == Aspiration::EV {
            "Electric".to_string()
        } else {
            format!("{:.1} L, {} cyl ({})", v.car.displacement_l, v.car.cylinders, v.car.engine_code)
        }
    });
    text_row("Aspiration", false, &|v| {
        if v.eval.aspiration != v.car.aspiration {
            format!("{} (was {})", v.eval.aspiration.label(), v.car.aspiration.label())
        } else {
            v.car.aspiration.label().to_string()
        }
    });
    text_row("Drivetrain", true, &|v| v.car.drivetrain.label().to_string());
    text_row("Transmission", false, &|v| v.car.transmissions.join(" / "));
    text_row("Redline", false, &|v| if v.car.redline_rpm == 0 { "unknown".into() } else { format!("{} rpm", v.car.redline_rpm) });
    text_row("Power peak", false, &|v| if v.car.hp_rpm == 0 { "rpm unknown".into() } else { format!("{} rpm", v.car.hp_rpm) });
    text_row("Torque peak", false, &|v| if v.car.torque_rpm == 0 { "rpm unknown".into() } else { format!("{} rpm", v.car.torque_rpm) });
    text_row("Fuel", false, &|v| v.car.fuel.label().to_string());
    text_row("Data confidence", false, &|v| v.car.confidence.label().to_string());

    // best-value lookups for charts
    for &ri in &chart_idx {
        let row = &t.rows[ri];
        t.charts.push(Chart { title: row.label.clone(), bars: row.cells.clone() });
    }
    t
}

fn flat(c: &Cell) -> String {
    if c.sub.is_empty() {
        c.text.clone()
    } else {
        format!("{} [{}]", c.text, c.sub)
    }
}

pub fn to_text(t: &Table) -> String {
    let mut out = String::from("Pitlane comparison (modded figures are estimates)\n\n");
    for (i, h) in t.headers.iter().enumerate() {
        out.push_str(&format!("#{} {} - {}{}\n", i + 1, h.title, h.subtitle, if h.modded { " (modded, est.)" } else { "" }));
    }
    out.push('\n');
    for r in &t.rows {
        out.push_str(&format!("{}:\n", r.label));
        for (i, c) in r.cells.iter().enumerate() {
            out.push_str(&format!("  #{} {}{}\n", i + 1, flat(c), if c.best { "  *best*" } else { "" }));
        }
    }
    out
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub fn to_csv(t: &Table) -> String {
    let mut out = String::from("Metric");
    for h in &t.headers {
        out.push(',');
        out.push_str(&csv_field(&format!("{} {}", h.title, h.subtitle)));
    }
    out.push('\n');
    for r in &t.rows {
        out.push_str(&csv_field(&r.label));
        for c in &r.cells {
            out.push(',');
            out.push_str(&csv_field(&flat(c)));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calc::{evaluate, ModSel};
    use crate::data::{mod_defs, Db};

    #[test]
    fn table_marks_best_and_exports() {
        let db = Db::bundled();
        let a = db.all_cars().find(|c| c.hp > 600).expect("a fast car");
        let b = db.all_cars().find(|c| c.hp < 150).expect("a slow car");
        let views = vec![
            SlotView { year: a.year_start, car: a, eval: evaluate(a, &[ModSel { id: "intake".into(), value: 0.0 }], mod_defs()) },
            SlotView { year: b.year_start, car: b, eval: evaluate(b, &[], mod_defs()) },
        ];
        let t = build_table(&views, Settings::default());
        let power = &t.rows[0];
        assert!(power.cells[0].best && !power.cells[1].best);
        assert!(power.cells[0].text.contains("est."));
        assert!(power.cells[0].sub.contains("stock"));
        let zs = &t.rows[2];
        assert!(zs.cells[0].best, "faster car should win 0-60");
        assert_eq!(t.charts.len(), 4);
        assert!(to_csv(&t).lines().count() > 10);
        assert!(to_text(&t).contains("*best*"));
        let m = Settings { system: UnitSystem::Metric, power: PowerUnit::Kw, wheel: true, ..Settings::default() };
        let t2 = build_table(&views, m);
        assert!(t2.rows[0].cells[0].text.contains("kW"));
        assert!(t2.rows[2].label.contains("100 km/h"));
    }
}
