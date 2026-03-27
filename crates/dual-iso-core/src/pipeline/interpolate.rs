use rayon::prelude::*;

use crate::types::{EV_RESOLUTION, InterpolationMethod, ProcessConfig, RawBuffer, RawMetadata};

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
            out.set_pixel(x, y_out, (a + b).div_ceil(2) as u16);
        }
    }
    // Last missing row: replicate the last known row.
    let last = h - 1;
    for x in 0..w {
        out.set_pixel(x, last * 2 + 1, buf.pixel(x, last));
    }

    out
}

// ─── AMaZE edge-directed interpolation in EV space ──────────────────────────

/// AMaZE-inspired edge-directed row interpolation.
///
/// Improvements over simple bilinear:
///   1. All blending is done in EV (log) space via the ev2raw/raw2ev lookup
///      tables, which gives perceptually uniform results and handles shadows
///      much better than linear averaging.
///   2. Adaptive horizontal ↔ vertical weighting: for each missing pixel we
///      compute directional gradient magnitudes in EV space and up-weight the
///      direction that crosses fewer hard edges.
///   3. Diagonal neighbours (±2 columns in the same Bayer phase) are used to
///      construct two additional diagonal estimates, further reducing aliasing
///      on angled edges.
fn interp_amaze_edge(buf: &RawBuffer, ev2raw: &[u16], raw2ev: &[i32]) -> RawBuffer {
    let w = buf.width;
    let h = buf.height;
    let out_h = h * 2;
    let ev2raw_len = ev2raw.len() as i32;
    let min_ev: i32 = -10 * EV_RESOLUTION;

    // ── Helper: raw → EV index (safe) ───────────────────────────────────────
    // raw2ev has 1M entries; pixel values are u16 (≤65535), so always in range.
    let r2e = |px: u16| raw2ev[px as usize];

    // ── Helper: EV index → raw (clamped) ────────────────────────────────────
    let e2r = |ev: i32| -> u16 {
        let idx = (ev - min_ev).clamp(0, ev2raw_len - 1) as usize;
        ev2raw[idx]
    };

    // ── Step 1: copy known rows into even output positions ───────────────────
    let mut out = RawBuffer::new(w, out_h);
    for y in 0..h {
        let dst = y * 2;
        out.data[dst * w..(dst + 1) * w].copy_from_slice(&buf.data[y * w..(y + 1) * w]);
    }

    // ── Step 2: interpolate each missing odd row in parallel ─────────────────
    //
    // For missing row at output y_out = 2*y + 1:
    //   top row  in input = y     (output y_out - 1)
    //   bot row  in input = y + 1 (output y_out + 1)
    //
    // Bayer channels repeat every 2 in both axes, so "same-channel" horizontal
    // neighbours are ±2 pixels away.
    //
    // We evaluate four blending estimates:
    //   V  – pure vertical:    EV_avg(top[x],   bot[x])
    //   D1 – diagonal ↗↙:     EV_avg(top[x+2], bot[x-2])
    //   D2 – diagonal ↖↘:     EV_avg(top[x-2], bot[x+2])
    //   H  – horizontal mean: EV_avg(avg_h(top), avg_h(bot)) at same x
    //
    // Each estimate is weighted by 1/(1+gradient²) where the gradient is the
    // directional EV difference that the estimate must "cross".

    let missing_rows: Vec<(usize, Vec<u16>)> = (0..h.saturating_sub(1))
        .into_par_iter()
        .map(|y| {
            let y_out = y * 2 + 1;
            let t = &buf.data[y * w..(y + 1) * w];
            let b = &buf.data[(y + 1) * w..(y + 2) * w];

            let mut row = vec![0u16; w];
            for x in 0..w {
                // EV of immediate vertical neighbours
                let ev_t = r2e(t[x]);
                let ev_b = r2e(b[x]);

                // EV of diagonal same-phase neighbours (clamp at edges)
                let xl2 = x.saturating_sub(2);
                let xr2 = (x + 2).min(w - 1);

                let ev_tl = r2e(t[xl2]);
                let ev_tr = r2e(t[xr2]);
                let ev_bl = r2e(b[xl2]);
                let ev_br = r2e(b[xr2]);

                // ── Four estimates (EV space) ──────────────────────────────
                // V: straightforward top↔bottom average
                let est_v = (ev_t + ev_b) / 2;

                // D1: top-right ↔ bottom-left  (↗ diagonal edge)
                let est_d1 = (ev_tr + ev_bl) / 2;

                // D2: top-left ↔ bottom-right  (↖ diagonal edge)
                let est_d2 = (ev_tl + ev_br) / 2;

                // H: horizontal same-channel average within each row, then V
                let ev_th = (ev_tl + ev_tr) / 2;
                let ev_bh = (ev_bl + ev_br) / 2;
                let est_h = (ev_th + ev_bh) / 2;

                // ── Gradient magnitudes for each direction ─────────────────
                // "Cost" of the vertical estimate = vertical EV change
                let gv = (ev_t - ev_b).unsigned_abs();

                // Cost of D1 = top-right – bottom-left spread
                let gd1 = (ev_tr - ev_bl).unsigned_abs();

                // Cost of D2 = top-left – bottom-right spread
                let gd2 = (ev_tl - ev_br).unsigned_abs();

                // Cost of H = horizontal smoothness (large = strong horiz edge)
                let gh = ((ev_tl - ev_tr).unsigned_abs() + (ev_bl - ev_br).unsigned_abs()) / 2;

                // ── Inverse-gradient weights (soft, squared denominator) ───
                // Scale by EV_RESOLUTION so we work in units of ~1 EV for the
                // squared denominator, preventing numerical issues.
                let eps = EV_RESOLUTION as u32; // 1 EV
                let wv = 1.0_f64 / (1.0 + (gv as f64 / eps as f64).powi(2));
                let wd1 = 1.0_f64 / (1.0 + (gd1 as f64 / eps as f64).powi(2));
                let wd2 = 1.0_f64 / (1.0 + (gd2 as f64 / eps as f64).powi(2));
                let wh = 1.0_f64 / (1.0 + (gh as f64 / eps as f64).powi(2));

                let total_w = wv + wd1 + wd2 + wh;
                let ev_mixed = if total_w > 1e-12 {
                    ((est_v as f64 * wv
                        + est_d1 as f64 * wd1
                        + est_d2 as f64 * wd2
                        + est_h as f64 * wh)
                        / total_w) as i32
                } else {
                    est_v
                };

                row[x] = e2r(ev_mixed);
            }
            (y_out, row)
        })
        .collect();

    // Write rows back (order doesn't matter; each y_out is unique).
    for (y_out, row) in missing_rows {
        out.data[y_out * w..(y_out + 1) * w].copy_from_slice(&row);
    }

    // Last odd row (no lower neighbour): replicate last known row.
    let last_odd = (h - 1) * 2 + 1;
    if last_odd < out_h {
        let src = (h - 1) * 2;
        out.data.copy_within(src * w..(src + 1) * w, last_odd * w);
    }

    out
}
