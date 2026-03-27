use crate::types::{IsoLinePattern, ProcessConfig, RawBuffer};

const EV_RES: i32 = crate::types::EV_RESOLUTION;

/// Blend the bright (high-ISO) and dark (low-ISO) Bayer planes into a
/// single 16-bit HDR buffer.
///
/// Strategy:
///   - Convert pixels to EV space.
///   - For pixels well below the bright plane's clipping point, use the
///     bright (low-noise shadow) values.
///   - For pixels well above the dark plane's noise floor, use the dark
///     (highlight-rich) values.
///   - Use a smooth sigmoid transition in the crossover region.
pub fn blend_iso_planes(
    bright: &RawBuffer,
    dark: &RawBuffer,
    _pattern: &IsoLinePattern,
    _config: &ProcessConfig,
    ev2raw: &[u16],
    raw2ev: &[i32],
    black_level: u16,
    white_level: u16,
) -> RawBuffer {
    assert_eq!(bright.width, dark.width);
    assert_eq!(bright.height, dark.height);

    let w = bright.width;
    let h = bright.height;
    let mut out = RawBuffer::new(w, h);

    // Estimate EV stop ratio between bright and dark planes.
    let ev_delta = estimate_ev_delta(bright, dark, raw2ev, black_level);
    tracing::debug!(ev_delta, "ISO blend EV delta (bright above dark)");

    // Blending crossover point: half the dynamic range of the bright plane
    // above black level.
    let white_ev = raw2ev_clamped(raw2ev, white_level as usize);
    let black_ev = raw2ev_clamped(raw2ev, black_level as usize);
    let range_ev = white_ev - black_ev;
    // Crossover threshold in EV where we start preferring the dark plane.
    let crossover = black_ev + range_ev * 3 / 4;
    let blend_width = EV_RES * 2; // 2-stop transition zone

    for y in 0..h {
        for x in 0..w {
            let bv = bright.pixel(x, y);
            let dv = dark.pixel(x, y);

            let bev = raw2ev_clamped(raw2ev, bv as usize);
            let dev = raw2ev_clamped(raw2ev, dv as usize);

            // Dark plane pixel shifted to same scale as bright.
            let dev_shifted = dev + ev_delta;

            // Sigmoid blend weight: 0 = all bright, 1 = all dark.
            let weight = sigmoid_blend(bev, crossover, blend_width);

            let blended_ev = lerp(bev, dev_shifted, weight);
            let blended_raw = ev2raw_clamped(ev2raw, blended_ev, black_level, white_level);

            out.set_pixel(x, y, blended_raw);
        }
    }

    out
}

/// Estimate how many EV stops separate the bright and dark planes.
fn estimate_ev_delta(
    bright: &RawBuffer,
    dark: &RawBuffer,
    raw2ev: &[i32],
    black_level: u16,
) -> i32 {
    let w = bright.width;
    let h = bright.height;
    let x0 = w / 4;
    let x1 = 3 * w / 4;
    let y0 = h / 4;
    let y1 = 3 * h / 4;

    let mut diffs = Vec::with_capacity((x1 - x0) * (y1 - y0) / 16);
    for y in (y0..y1).step_by(4) {
        for x in (x0..x1).step_by(4) {
            let bv = bright.pixel(x, y) as usize;
            let dv = dark.pixel(x, y) as usize;
            if bv > black_level as usize + 512 && dv > black_level as usize + 512 {
                diffs.push(raw2ev_clamped(raw2ev, bv) - raw2ev_clamped(raw2ev, dv));
            }
        }
    }
    if diffs.is_empty() {
        return EV_RES * 3; // default 3-stop assumption
    }
    diffs.sort_unstable();
    diffs[diffs.len() / 2] // median
}

#[inline]
fn raw2ev_clamped(raw2ev: &[i32], v: usize) -> i32 {
    let v = v.min(raw2ev.len() - 1);
    raw2ev[v]
}

#[inline]
fn ev2raw_clamped(ev2raw: &[u16], ev: i32, black: u16, white: u16) -> u16 {
    const MIN_EV: i32 = -10 * EV_RES;
    let idx = (ev - MIN_EV).max(0) as usize;
    if idx >= ev2raw.len() {
        return white;
    }
    ev2raw[idx].clamp(black, white)
}

/// Smooth sigmoid: returns 0 when `ev` ≪ `center`, 1 when `ev` ≫ `center`.
#[inline]
fn sigmoid_blend(ev: i32, center: i32, width: i32) -> f32 {
    let t = (ev - center) as f32 / width as f32;
    1.0 / (1.0 + (-t * 2.0).exp())
}

#[inline]
fn lerp(a: i32, b: i32, t: f32) -> i32 {
    a + ((b - a) as f32 * t) as i32
}
