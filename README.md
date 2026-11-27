# Pitlane

A cross-platform (Linux, Windows, Android) car spec comparison tool written entirely in Rust with [Slint](https://slint.dev).
Compare up to 5 cars (1990-2026, US market), stack mods, and see estimated modded hp / torque / 0-60.

<!-- Screenshots: add images to docs/ and link them here -->
![screenshot placeholder](docs/screenshot.png)

## Features
Cascading Year -> Make -> Model -> Trim pickers; table/card views with best-value highlighting and bar charts; a data-driven mods
system (aspiration-aware gains, negative modifiers, altitude, warnings for meaningless combos); imperial/metric, hp/kW/PS, crank vs
estimated wheel power; fuzzy search ("06 civic si", "lx570"); favorites and saved comparisons; editable stats, custom cars and
overrides import/export; PNG export and CSV/text export; light/dark following the system.

## Layout
* `crates/pitlane-core` - dataset loader, mod math, units, search, persistence, comparison tables. No UI. All unit tests live here.
* `crates/pitlane-app` - Slint UI (`ui/app.slint`), desktop `main.rs`, and the Android `android_main` entry in `lib.rs`.
* `data/` - RON dataset converted to bincode by `build.rs` ([schema](data/README.md)).

## Build
Prerequisites: stable Rust. Linux needs the usual Slint/winit system libraries (fontconfig, xkbcommon, wayland/xcb, GL).

```bash
cargo build --release          # desktop binary: target/release/pitlane
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Renderer: the default is Slint's GPU renderer (FemtoVG). For the software renderer:
`cargo build --release --no-default-features --features renderer-software`.

### Linux AppImage (x86_64)
See `.github/workflows/release.yml` (packages `target/release/pitlane` with `packaging/pitlane.desktop` using appimagetool).

### Windows (.exe)
```bash
rustup target add x86_64-pc-windows-msvc
cargo build --release -p pitlane-app --bin pitlane --target x86_64-pc-windows-msvc
```
Release builds use the GUI subsystem (no console window).

### Android (APK)
```bash
rustup target add aarch64-linux-android
cargo install cargo-apk --locked
# ANDROID_HOME / ANDROID_NDK_ROOT must point at an SDK + NDK
cargo apk build --release -p pitlane-app --lib --no-default-features --features renderer-software
```
Android metadata is in `crates/pitlane-app/Cargo.toml` (`[package.metadata.android]`). Data is stored in the app-private directory.
`[profile.release-android]` (opt-level "s") is available via `--profile release-android`.

## Releases
Pushing a tag like `v0.1.0` runs the workflow, which tests, builds the AppImage, Windows exe and Android APK and attaches them to a GitHub Release.

## Data accuracy disclaimer
The dataset was authored by hand from commonly published specifications, **not** imported from an authoritative database and not
machine-checked against sources. `verified` means "matches widely published factory hp/torque/weight as recalled by the author";
`estimated` means approximate. 0-60 times and curb weights are the least reliable columns (`source` tells you whether a time is a
manufacturer claim, a magazine test, or an estimate). Coverage of 1990-2026 is broad for popular and performance models but not
exhaustive. Mod gains and 0-60 recalculations are rough estimates (labelled "est."), not predictions for any specific car.
You can correct any number in the app; your edits live in `overrides.ron`.

## Known limitations
See the summary in the project hand-off notes: Android and Windows builds are configured but were not built on the development machine;
clipboard on Android and the hardware back button mapping are untested; the dataset is not exhaustive.
