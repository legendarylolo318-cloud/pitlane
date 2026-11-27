//! Dataset load / search benchmark: `cargo run --release -p pitlane-core --example bench`
use pitlane_core::search::search;
use pitlane_core::Db;
use std::time::Instant;

fn rss_kb(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

fn main() {
    let r0 = rss_kb("VmRSS:");
    let t = Instant::now();
    let db = Db::bundled();
    let open = t.elapsed();
    let t = Instant::now();
    let n = db.cars_of_make("Ford").len();
    let one_make = t.elapsed();
    let t = Instant::now();
    let first = search(&db, "06 civic si", 8).len();
    let first_search = t.elapsed();
    let queries = ["lx570", "99 supra tt", "hellcat", "'15 sti", "c8 z06", "mustang gt", "f150 raptor", "type r", "2002 wrx", "trackhawk"];
    let t = Instant::now();
    let mut hits = 0;
    for _ in 0..100 {
        for q in queries {
            hits += search(&db, q, 8).len();
        }
    }
    let per_search = t.elapsed() / 1000;
    let total = db.all_cars().count();
    println!(
        "cars={total} open={:?} first_make({n})={:?} first_search({first})={:?} avg_search={:?} (hits {hits}) rss_delta_kb={} peak_rss_kb={}",
        open,
        one_make,
        first_search,
        per_search,
        rss_kb("VmRSS:").saturating_sub(r0),
        rss_kb("VmHWM:")
    );
}
