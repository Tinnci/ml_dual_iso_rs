# ml_dual_iso_rs

A pure-Rust rewrite of the [Magic Lantern](https://magiclantern.fm/) `cr2hdr` dual-ISO post-processing tool.

Converts dual-ISO RAW files (CR2 / DNG) captured by Magic Lantern-equipped Canon cameras into a single high-dynamic-range DNG, recovering roughly **3 extra stops of dynamic range** compared to a single-ISO exposure.

---

## Features

- **Pure Rust** – no dcraw, no exiftool, no external C tools required
- **rawler** integration for native CR2 / Canon DNG decoding
- **AMaZE-inspired edge-aware interpolation** (mean2/3 or edge mode)
- Chroma smoothing (2×2 / 3×3 / 5×5 median), dithering, bad-pixel correction, stripe-fix
- White balance modes: Auto (graymax / graymed), EXIF, Kelvin temperature, Custom multipliers
- Parallel processing of multiple files via **rayon**
- **CLI** (`cr2hdr`) – drop-in replacement for the original tool
- **GUI** (`dual-iso-gui`) – egui/eframe native desktop application with drag-and-drop
- **xtask** task runner for development workflow

---

## Quick Start

```bash
# Clone
git clone https://github.com/Tinnci/ml_dual_iso_rs.git
cd ml_dual_iso_rs

# Run the GUI
cargo xtask run-gui

# Convert a single file via CLI
cargo xtask run-cli -- my_shot.CR2

# Convert with options
cargo xtask run-cli -- --iso-100 100 --iso-1600 1600 --amaze --cs3x3 my_shot.CR2
```

---

## Workspace Layout

```
ml_dual_iso_rs/
├── Cargo.toml               # workspace root (resolver v3, edition 2024)
├── xtask/                   # task runner  (cargo xtask <task>)
├── crates/
│   ├── dual-iso-core/       # core library – pipeline algorithms
│   └── dual-iso-gui/        # egui/eframe desktop GUI
├── bins/
│   └── cr2hdr/              # CLI binary (clap)
└── original_src/            # original C source for reference
```

---

## xtask Tasks

| Command | Description |
|---|---|
| `cargo xtask check` | `fmt --check` + `clippy -D warnings` |
| `cargo xtask fmt` | Auto-format all crates |
| `cargo xtask test` | Run all unit tests |
| `cargo xtask dist` | Release build → `dist/` folder |
| `cargo xtask run-cli [-- ARGS]` | Build & run `cr2hdr` CLI, pass extra args after `--` |
| `cargo xtask run-gui` | Build & run the GUI application |

---

## CLI Usage

```
cr2hdr [OPTIONS] <FILES>...

Options:
  --iso-100 <N>       Low-ISO value (default: auto-detect)
  --iso-1600 <N>      High-ISO value (default: auto-detect)
  --amaze             Use AMaZE edge-aware interpolation (slower, better quality)
  --cs2x2             Chroma smooth 2×2
  --cs3x3             Chroma smooth 3×3 (default)
  --cs5x5             Chroma smooth 5×5
  --no-dither         Disable output dithering
  --no-fixbp          Skip bad-pixel correction
  --wb-kelvin <K>     Set white balance by colour temperature in Kelvin
  --compress          Apply lossless DNG compression
  -o <DIR>            Output directory (default: same as input)
```

---

## Pipeline Overview

```
CR2/DNG input
    │
    ▼
rawler decode  →  RawImage (u16 Bayer mosaic)
    │
    ├─► detect ISO lines   (row-variance analysis)
    ├─► deinterlace        (split bright / dark planes)
    ├─► interpolate        (mean2/3 or AMaZE edge)
    ├─► blend              (EV-space sigmoid blend)
    ├─► chroma smooth      (median filter on R-G, B-G)
    ├─► bad pixels         (neighbourhood median)
    ├─► stripe fix         (per-row offset correction)
    ├─► white balance      (scale R/B channels)
    └─► dither             (Gaussian noise σ=0.5)
         │
         ▼
      DNG output  (hand-crafted TIFF/DNG writer)
```

---

## Requirements

- Rust **1.80+** (edition 2024)
- macOS / Linux / Windows (cross-platform)

---

## Credits

Original C implementation by **a1ex** and contributors: [Magic Lantern dual_iso module](https://bitbucket.org/hudson/magic-lantern/).  
AMaZE algorithm from **RawTherapee**.  
Kelvin→RGB conversion from CIE daylight locus formulae.

---

## License

Licensed under the **GNU General Public License v2.0** – see [LICENSE](LICENSE).
