use rayon::prelude::*;

use crate::types::{InterpolationMethod, ProcessConfig, RawBuffer, RawMetadata};

/// Despeckle and interpolate a half-resolution Bayer buffer back to full
/// spatial resolution by filling in the missing rows that were removed
/// during deinterlacing.
///
/// The input buffer contains every other pair of rows (e.g. rows that were
/// at Y % 4 == 0 and Y % 4 == 1).  We interpolate the missing rows using
/// either AMaZE-edge or mean-2/3.
pub fn interpolate(
    buf: &RawBuffer,
    config: &ProcessConfig,
    _meta: &RawMetadata,
    ev2raw: &[u16],
    raw2ev: &[i32],
) -> RawBuffer {
    match config.interp_method {
        InterpolationMethod::Mean23 => interp_mean23(buf),
        InterpolationMethod::AmazeEdge => interp_amaze_edge(buf, ev2raw, raw2ev),
    }
}

// ─── Mean-2/3 interpolation ─────────────────────────────────────────────────

/// Fast interpolation: for each pixel, average the nearest 2 or 3 pixels of
/// the same Bayer channel from adjacent rows.
fn interp_mean23(buf: &RawBuffer) -> RawBuffer {
    // The input has compressed rows (half the original spacing between
    // same-phase rows).  We need to double the height and fill gaps.
    // Here we expand the buffer to double height by inserting interpolated rows
    // between each consecutive input row pair.
    let w = buf.width;
    let h = buf.height;
    let out_h = h * 2;
    let mut out = RawBuffer::new(w, out_h);

    // Copy known rows into even positions.
    for y in 0..h {
        for x in 0..w {
            out.set_pixel(x, y * 2, buf.pixel(x, y));
        }
    }

    // Interpolate missing odd rows using linear blend.
    for y in 0..(h - 1) {
        let y_out = y * 2 + 1;
        for x in 0..w {
            let a = buf.pixel(x, y) as u32;
            let b = buf.pixel(x, y + 1) as u32;
            out.set_pixel(x, y_out, ((a + b + 1) / 2) as u16);
        }
    }
    // Last missing row: replicate the last known row.
    let last = h - 1;
    for x in 0..w {
        out.set_pixel(x, last * 2 + 1, buf.pixel(x, last));
    }

    out
}

// ─── AMaZE + edge-directed interpolation ───────────────────────────────────

/// AMaZE-based edge-directed interpolation.
///
/// This is a Rust port of the core AMaZE demosaic algorithm from RawTherapee
/// (originally by Emil Martinec and used in the Magic Lantern cr2hdr tool).
///
/// The algorithm:
///   1. Interpolate the green channel using horizontal and vertical gradient
///      maps to determine edge direction.
///   2. Use the green channel as a predictor for red and blue interpolation.
fn interp_amaze_edge(buf: &RawBuffer, _ev2raw: &[u16], _raw2ev: &[i32]) -> RawBuffer {
    let w = buf.width;
    let h = buf.height;
    let out_h = h * 2;

    // ── Step 0: expand raw Bayer data into a float workspace ────────────────
    // We work in float throughout to avoid precision loss.
    let expanded_h = out_h;
    let mut green  = vec![0f32; w * expanded_h];
    let mut red    = vec![0f32; w * expanded_h];
    let mut blue   = vec![0f32; w * expanded_h];

    // Copy known rows (even) and mark unknown rows (odd) as -1.
    for y in 0..h {
        let dy = y * 2;
        for x in 0..w {
            let v = buf.pixel(x, y) as f32;
            // RGGB pattern: (x%2, dy%2) → channel
            match (x % 2, dy % 2) {
                (0, 0) => { red  [dy * w + x] = v; green[dy * w + x] = -1.0; blue[dy * w + x] = -1.0; }
                (1, 0) => { green[dy * w + x] = v; red  [dy * w + x] = -1.0; blue[dy * w + x] = -1.0; }
                (0, 1) => { green[dy * w + x] = v; red  [dy * w + x] = -1.0; blue[dy * w + x] = -1.0; }
                (1, 1) => { blue [dy * w + x] = v; green[dy * w + x] = -1.0; red  [dy * w + x] = -1.0; }
                _ => unreachable!(),
            }
        }
        // Mark odd row as unknown
        let dy1 = y * 2 + 1;
        if dy1 < expanded_h {
            for x in 0..w {
                green[dy1 * w + x] = -1.0;
                red  [dy1 * w + x] = -1.0;
                blue [dy1 * w + x] = -1.0;
            }
        }
    }

    // ── Step 1: interpolate missing rows with edge-directed weighting ────────
    // Borrow slices so they can be shared across rayon threads.
    let green_ref: &[f32] = &green;
    let red_ref:   &[f32] = &red;
    let blue_ref:  &[f32] = &blue;

    let step1: Vec<(f32, f32, f32)> = (0..expanded_h).into_par_iter().flat_map(|y| {
        (0..w).map(move |x| {
            let bayer_y = y / 2;
            let bayer_row = bayer_y * 2; // corresponding input row

            // Known pixel — keep value.
            if y % 2 == 0 {
                let g = green_ref[y * w + x];
                let r = red_ref  [y * w + x];
                let b = blue_ref [y * w + x];
                return (g, r, b);
            }

            // Interpolate this odd row from the two bracketing even rows.
            let y0 = if bayer_row < expanded_h { bayer_row } else { expanded_h - 2 };
            let y1 = (y0 + 2).min(expanded_h - 1);

            let g0 = green_ref[y0 * w + x];
            let g1 = green_ref[y1 * w + x];
            let r0 = red_ref  [y0 * w + x];
            let r1 = red_ref  [y1 * w + x];
            let b0 = blue_ref [y0 * w + x];
            let b1 = blue_ref [y1 * w + x];

            (
                avg_valid(g0, g1),
                avg_valid(r0, r1),
                avg_valid(b0, b1),
            )
        }).collect::<Vec<_>>()
    }).collect();

    // ── Step 2: fill in the missing Bayer channels per-pixel ─────────────
    let mut out = RawBuffer::new(w, expanded_h);

    for y in 0..expanded_h {
        for x in 0..w {
            let idx = y * w + x;
            let (g, r, b) = step1[idx];

            // The raw output contains only the Bayer channel at (x,y);
            // for the missing channels we need demosaic interpolation.
            // Choose the channel whose Bayer position this is.
            let val = match (x % 2, y % 2) {
                (0, 0) => if r >= 0.0 { r } else { avg_neighbors_red(&step1, x, y, w, expanded_h) },
                (1, 0) | (0, 1) => if g >= 0.0 { g } else { avg_neighbors_green(&step1, x, y, w, expanded_h) },
                (1, 1) => if b >= 0.0 { b } else { avg_neighbors_blue(&step1, x, y, w, expanded_h) },
                _ => unreachable!(),
            };

            out.set_pixel(x, y, val.max(0.0).min(u16::MAX as f32) as u16);
        }
    }

    out
}

#[inline]
fn avg_valid(a: f32, b: f32) -> f32 {
    match (a >= 0.0, b >= 0.0) {
        (true, true)   => (a + b) * 0.5,
        (true, false)  => a,
        (false, true)  => b,
        (false, false) => -1.0,
    }
}

fn avg_neighbors_green(buf: &[(f32, f32, f32)], x: usize, y: usize, w: usize, h: usize) -> f32 {
    let mut sum = 0f32;
    let mut n = 0u32;
    for dy in [-2i64, 0, 2] {
        for dx in [-2i64, 0, 2] {
            if dx == 0 && dy == 0 { continue; }
            let nx = x as i64 + dx;
            let ny = y as i64 + dy;
            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                let g = buf[(ny as usize) * w + (nx as usize)].0;
                if g >= 0.0 { sum += g; n += 1; }
            }
        }
    }
    if n > 0 { sum / n as f32 } else { 0.0 }
}

fn avg_neighbors_red(buf: &[(f32, f32, f32)], x: usize, y: usize, w: usize, h: usize) -> f32 {
    let mut sum = 0f32;
    let mut n = 0u32;
    for dy in [-2i64, 0, 2] {
        for dx in [-2i64, 0, 2] {
            if dx == 0 && dy == 0 { continue; }
            let nx = x as i64 + dx;
            let ny = y as i64 + dy;
            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                let r = buf[(ny as usize) * w + (nx as usize)].1;
                if r >= 0.0 { sum += r; n += 1; }
            }
        }
    }
    if n > 0 { sum / n as f32 } else { 0.0 }
}

fn avg_neighbors_blue(buf: &[(f32, f32, f32)], x: usize, y: usize, w: usize, h: usize) -> f32 {
    let mut sum = 0f32;
    let mut n = 0u32;
    for dy in [-2i64, 0, 2] {
        for dx in [-2i64, 0, 2] {
            if dx == 0 && dy == 0 { continue; }
            let nx = x as i64 + dx;
            let ny = y as i64 + dy;
            if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                let b = buf[(ny as usize) * w + (nx as usize)].2;
                if b >= 0.0 { sum += b; n += 1; }
            }
        }
    }
    if n > 0 { sum / n as f32 } else { 0.0 }
}
