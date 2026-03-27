use crate::types::{ChromaSmoothSize, RawBuffer};

const EV_RES: i32 = crate::types::EV_RESOLUTION;

/// Apply chroma smoothing in the Bayer domain.
///
/// For each 2×2 Bayer block we compute the luma (green average) and then
/// take the median of the R-G and B-G chrominance values in the surrounding
/// neighbourhood.  This reduces colour noise without affecting luminance.
pub fn chroma_smooth(buf: &mut RawBuffer, size: ChromaSmoothSize, raw2ev: &[i32], ev2raw: &[u16]) {
    let radius: i64 = match size {
        ChromaSmoothSize::None => return,
        ChromaSmoothSize::TwoByTwo => 2,
        ChromaSmoothSize::ThreeByThree => 3,
        ChromaSmoothSize::FiveByFive => 5,
    };

    let w = buf.width;
    let h = buf.height;
    // Work on a copy so reads and writes don't interfere.
    let src = buf.clone();

    for y in (4..(h as i64 - 5)).step_by(2) {
        for x in (4..(w as i64 - 4)).step_by(2) {
            // Green average for the 2×2 block at (x,y).
            let g1 = src.pixel_clamped(x + 1, y) as usize;
            let g2 = src.pixel_clamped(x, y + 1) as usize;
            let ge = (raw2ev_safe(raw2ev, g1) + raw2ev_safe(raw2ev, g2)) / 2;

            // Skip very dark areas — noise there looks bad anyway.
            if ge < 2 * EV_RES {
                continue;
            }

            // Collect neighbourhood samples.
            let r_max = 2 * ((radius / 2 + 1) as usize);
            let half = radius / 2;
            let mut med_r: Vec<i32> = Vec::with_capacity(r_max * r_max);
            let mut med_b: Vec<i32> = Vec::with_capacity(r_max * r_max);

            let mut i = -half * 2;
            while i <= half * 2 {
                let mut j = -half * 2;
                while j <= half * 2 {
                    // For 2×2 mode, skip the diagonal corners.
                    if radius == 2 && i.abs() + j.abs() == 4 {
                        j += 2;
                        continue;
                    }
                    let r = src.pixel_clamped(x + i, y + j) as usize;
                    let g1 = src.pixel_clamped(x + i + 1, y + j) as usize;
                    let g2 = src.pixel_clamped(x + i, y + j + 1) as usize;
                    let b = src.pixel_clamped(x + i + 1, y + j + 1) as usize;
                    let ge_local = (raw2ev_safe(raw2ev, g1) + raw2ev_safe(raw2ev, g2)) / 2;
                    med_r.push(raw2ev_safe(raw2ev, r) - ge_local);
                    med_b.push(raw2ev_safe(raw2ev, b) - ge_local);
                    j += 2;
                }
                i += 2;
            }

            let dr = median_i32(&mut med_r);
            let db = median_i32(&mut med_b);

            if ge + dr <= EV_RES {
                continue;
            }
            if ge + db <= EV_RES {
                continue;
            }

            let r_out = ev2raw_safe(ev2raw, ge + dr);
            let b_out = ev2raw_safe(ev2raw, ge + db);
            buf.set_pixel(x as usize, y as usize, r_out);
            buf.set_pixel(x as usize + 1, y as usize + 1, b_out);
        }
    }
}

#[inline]
fn raw2ev_safe(raw2ev: &[i32], v: usize) -> i32 {
    raw2ev[v.min(raw2ev.len() - 1)]
}

#[inline]
fn ev2raw_safe(ev2raw: &[u16], ev: i32) -> u16 {
    const MIN_EV: i32 = -10 * EV_RES;
    let idx = (ev - MIN_EV).max(0) as usize;
    if idx >= ev2raw.len() {
        return ev2raw[ev2raw.len() - 1];
    }
    ev2raw[idx]
}

fn median_i32(v: &mut [i32]) -> i32 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}
