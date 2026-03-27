use crate::types::RawBuffer;
// rayon reserved for future parallel stripe fix

/// Fix horizontal stripe noise (banding) by computing and subtracting
/// per-row offsets.
///
/// Strategy: for each row compute the median of the difference between
/// same-channel pixels in neighbouring rows.  Apply a correction to remove
/// the systematic offset component.
pub fn fix_stripes(buf: &mut RawBuffer) {
    let w = buf.width;
    let h = buf.height;
    if h < 4 {
        return;
    }

    // Compute per-row correction offset (same channel = step of 2).
    let offsets: Vec<i32> = (0..h)
        .map(|y| {
            if y == 0 || y == h - 1 {
                return 0i32;
            }
            let mut diffs: Vec<i32> = Vec::with_capacity(w / 4);
            // Compare to the row 2 above (same Bayer channel phase).
            if y >= 2 {
                for x in (2..w - 2).step_by(2) {
                    let v = buf.pixel(x, y) as i32;
                    let vp = buf.pixel(x, y - 2) as i32;
                    diffs.push(v - vp);
                }
            }
            median_i32_mut(&mut diffs)
        })
        .collect();

    // Apply correction: subtract the accumulation of offsets from the
    // first non-zero row down.
    let mut cumulative = 0i32;
    for (y, &off) in offsets.iter().enumerate().take(h) {
        cumulative += off;
        if cumulative == 0 {
            continue;
        }
        for x in 0..w {
            let v = buf.pixel(x, y) as i32 - cumulative;
            buf.set_pixel(x, y, v.clamp(0, u16::MAX as i32) as u16);
        }
    }
}

fn median_i32_mut(v: &mut [i32]) -> i32 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}
