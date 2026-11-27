# Pitlane dataset

All car data lives in this directory as [RON](https://github.com/ron-rs/ron) text. `crates/pitlane-core/build.rs`
parses every `*.ron` file here at build time, **validates** it (a bad row fails the build), expands it, and writes
two compact blobs into `OUT_DIR` (`cars.bin`, an interned fixed-record table, and `mods.bin`, bincode) that are embedded with `include_bytes!`. No network or file access is needed at run time.

| File | Contents |
|------|----------|
| `ford.ron`, `gm.ron`, `stellantis.ron`, `honda.ron`, `toyota.ron`, `subaru.ron` | one file per parent group |
| `mods.ron` | mod definitions (see below) |

## Run-time loading
`build.rs` writes `cars.bin`, a string-interned table of fixed-size car records (layout documented on `CARS_MAGIC` in
`crates/pitlane-core/src/model.rs`). It is embedded with `include_bytes!` and read in place. Text fields borrow from the
binary (`Cow<'static, str>`), so no `String` is allocated per field. A make's cars are decoded the first time that make is
needed (`Db::cars_of_make`, `trims_for`, ...). Search builds a small normalized index on first use and reuses it.

## Car record format
Each file is `(group: "...", gens: [ Gen, ... ])`. A `Gen` is a generation of one model; each `Trim` inside it is one
trim/engine combination valid for that generation's years.

```ron
(make:"Honda", model:"Civic", gen:"8th gen (2006-11)", y0:2006, y1:2011, trims:[
 (n:"Si 2.0 (FG2/FA5)", code:"K20Z3", d:2.0, c:4, a:NA, hp:197, hpr:7800, tq:139, tqr:6100,
  z:6.7, zs:Mag, w:2900, dt:FWD, tx:"6MT", rl:8000, conf:Verified),
])
```

| Field | Meaning | Required |
|-------|---------|----------|
| `make`, `model`, `gen` | make / model / generation label | yes |
| `y0`, `y1` | first / last model year of the generation (1990-2026) | yes |
| `n` | trim / engine name | yes |
| `code` | engine code | yes |
| `d` | displacement, litres (`0.0` for EV) | yes |
| `c` | cylinders (`0` for EV) | yes |
| `a` | aspiration: `NA`, `Turbo`, `Supercharged`, `Hybrid`, `EV` (twin-turbo / compound = `Turbo`) | yes |
| `hp`, `hpr` | crank horsepower, rpm at peak (`0` = unknown) | `hp` yes |
| `tq`, `tqr` | torque lb-ft, rpm at peak (`0` = unknown) | `tq` yes |
| `z` | 0-60 mph seconds | yes |
| `zs` | source of `z`: `Mfr` (manufacturer claim), `Mag` (magazine test), `Est` (estimated; default) | no |
| `w` | curb weight, lb | yes |
| `dt` | `FWD`, `RWD`, `AWD`, `FourWD` | yes |
| `tx` | transmissions, `/`-separated (`"6MT/8AT"`). The **first** entry decides manual vs automatic for the wheel-hp loss. | yes |
| `rl` | redline rpm (`0` = unknown) | no |
| `f` | fuel: `Gas` (default), `Diesel`, `Hybrid`, `Electric`, `Flex` | no |
| `conf` | `Verified` or `Estimated` (default) | no |
| `y0`, `y1` (inside a trim) | narrows the generation's years for this trim only (e.g. an engine offered for part of the run) | no |

The unique `id` is generated: `make-model-generation-trim` (slugified). Duplicate ids fail the build.

### What `conf` and `zs` mean (be honest when you extend this)
* `conf:Verified` - horsepower, torque and curb weight match the widely-published factory figures **as recalled by the
  author**. They have not been machine-checked against a source database. Treat as "very likely right", not as audited.
* `conf:Estimated` (default) - one or more of hp/torque/weight is approximate (mid-cycle changes, rounded weights, trim variance).
* `zs:Mfr` / `zs:Mag` / `zs:Est` - where the 0-60 came from. Most `Est` values are inferred from similar cars. **0-60 and weights
  are the least reliable columns.** If you do not know a number, use a realistic value and mark it `Estimated` / `Est`.

### Validation (build + `cargo test`)
* years within 1990-2026 and `y0 <= y1`
* required text fields non-empty, at least one transmission
* hp 40-1500, torque 40-1500, weight 1500-10000 lb, 0-60 1.5-20 s, displacement 0.6-9.0 L, 2-12 cylinders (EVs: 0/0)
* no duplicate ids; every required brand present; every make listed for a year has models for that year
* no trim name listed twice for the same make/model/year, and no two identical cars under different names
* reference cars match published figures (`reference_cars_match_published_figures`)

## User overrides
The app can edit any stat or add a custom car. Edits are stored in `overrides.ron` in the app's private data directory:

```ron
(cars: [ ( id: "ford-mustang-s550-2015-2023-gt-5-0-2018", make: "Ford", ..., transmission: "6MT/10AT", ... ) ])
```
A car whose `id` matches a bundled car replaces it; any other `id` is added (new makes appear automatically). An override
keeps the bundled car's make, model and years (the app enforces this); change those by adding a custom car instead. If an id
appears twice in an import, the last one wins.
The in-app Import/Export screen copies/pastes this text. Overrides are validated with the same rules as the bundled data.

## Mods (`mods.ron`)
Each mod is a list of **effects**. For a car the *first* effect whose `applies_to` contains the car's current aspiration and
whose `requires` mods are all selected is used. All numbers are data:

| Field | Meaning |
|-------|---------|
| `min_pct`, `max_pct` | power change in % of *current* power; `typical` is the midpoint |
| `flat_hp` | added hp |
| `unit_pct_min/max`, `unit_hp_min/max`, `unit_lbs`, `unit_div` | per-slider-unit scaling (e.g. altitude: `-3.0 %` per 1000 ft = `unit_div:1000`) |
| `torque_factor` | torque change relative to the power change (1.0 = same %) |
| `weight_lbs` | fixed weight change |
| `zs_pct_min/max` | direct 0-60 change (negative = quicker) |
| `traction_pct` | raises the traction limit in the 0-60 model |
| `makes_aspiration` | later mods treat the car as this aspiration (turbo kit) |
| `requires` | other mod ids needed for this effect |
| `note` | warning shown when the effect is used |
| `conflicts` (on the mod) | mods that cannot be combined (the later-defined one is ignored with a warning) |

Mods are applied in file order (hardware first, conditions last). Percentage effects compound on the running engine power, so
stacking has diminishing returns. Flat effects (nitrous shot, custom +/- hp) are added at the end and are never scaled by other
mods' percentages. A mod that is ignored (wrong fuel, wrong aspiration, losing a conflict) never satisfies another mod's
`requires`.
All gain percentages are rough, typical bolt-on figures, **not** measurements of any specific car.
