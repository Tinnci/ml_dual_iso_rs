use crate::types::RawBuffer;

/// Detect and fix bad (hot/dead) pixels in a Bayer buffer.
///
/// A pixel is considered bad if its value deviates from the local
/// neighbourhood median by more than a threshold that scales with the
/// dynamic range.  Bad pixels are replaced with the neighbourhood median.
///
/// * `aggressive` – lower detection threshold (catch subtler defects at
///   the cost of some fine detail).
/// * `mark_black` – instead of interpolating, set bad pixels to 0 (for
///   troubleshooting / visualisation).
pub fn fix_bad_pixels(buf: &mut RawBuffer, aggressive: bool, mark_black: bool) {
    let w = buf.width;
    let h = buf.height;

    // Threshold: pixels deviating more than this fraction of the white level
    // from the neighbourhood median are considered bad.
    let threshold: u32 = if aggressive { 512 } else { 1024 };

    let src = buf.clone();
    let mut bad_count = 0usize;

    for y in 2..(h - 2) {
        for x in 2..(w - 2) {
            let v = src.pixel(x, y) as u32;
            let med = neighbourhood_median_same_channel(&src, x, y, 2) as u32;

            if v.abs_diff(med) > threshold {
                bad_count += 1;
                if mark_black {
                    buf.set_pixel(x, y, 0);
                } else {
                    buf.set_pixel(x, y, med as u16);
                }
            }
        }
    }

    if bad_count > 0 {
        tracing::debug!(bad_count, "bad pixels fixed");
    }
}

/// Compute the median of same-Bayer-channel neighbours within `radius`
/// blocks (skipping the centre pixel).
fn neighbourhood_median_same_channel(buf: &RawBuffer, cx: usize, cy: usize, radius: usize) -> u16 {
    let mut vals: Vec<u16> = Vec::with_capacity(8);
    let step: i64 = 2; // same channel every 2 pixels in Bayer
    let r = (radius as i64) * step;
    for dy in (-r..=r).step_by(step as usize) {
        for dx in (-r..=r).step_by(step as usize) {
            if dx == 0 && dy == 0 {
                continue;
            }
            vals.push(buf.pixel_clamped(cx as i64 + dx, cy as i64 + dy));
        }
    }
    median_u16(&mut vals)
}

fn median_u16(v: &mut [u16]) -> u16 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}
