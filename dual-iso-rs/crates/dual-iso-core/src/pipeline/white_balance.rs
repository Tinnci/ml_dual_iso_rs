use crate::types::{RawBuffer, RawMetadata, WhiteBalance};

/// Compute white-balance multipliers [R, G, B] for the given mode.
/// Returns multipliers normalised so that G = 1.0.
pub fn compute_wb(buf: &RawBuffer, meta: &RawMetadata, mode: &WhiteBalance) -> [f32; 3] {
    match mode {
        WhiteBalance::Custom(r, g, b) => {
            let gn = if *g != 0.0 { *g } else { 1.0 };
            [r / gn, 1.0, b / gn]
        }
        WhiteBalance::Exif => {
            // Use camera pre-multipliers from the decoded raw file.
            let [r, g, b] = meta.pre_mul;
            let gn = if g != 0.0 { g } else { 1.0 };
            [r / gn, 1.0, b / gn]
        }
        WhiteBalance::GrayMax => gray_wb(buf, meta, true),
        WhiteBalance::GrayMedian => gray_wb(buf, meta, false),
    }
}

/// Estimate white balance by analysing "nearly grey" pixels.
///
/// For each 2×2 Bayer block, compute R−G and B−G differences.  Pixels
/// where both differences are small are "grey".  Return the multipliers
/// that make those pixels perfectly grey.
fn gray_wb(buf: &RawBuffer, meta: &RawMetadata, use_max: bool) -> [f32; 3] {
    let w = buf.width;
    let h = buf.height;
    let black = meta.black_level as i32;

    // Use a histogram of R-G and B-G chrominance.
    const HIST_SIZE: usize = 4096;
    const OFFSET: i32 = HIST_SIZE as i32 / 2;
    let mut hist_rg = vec![0u32; HIST_SIZE];
    let mut hist_bg = vec![0u32; HIST_SIZE];

    for y in (2..(h - 2)).step_by(2) {
        for x in (2..(w - 2)).step_by(2) {
            let r  = buf.pixel(x,     y)     as i32 - black;
            let g1 = buf.pixel(x + 1, y)     as i32 - black;
            let g2 = buf.pixel(x,     y + 1) as i32 - black;
            let b  = buf.pixel(x + 1, y + 1) as i32 - black;
            if r <= 0 || g1 <= 0 || g2 <= 0 || b <= 0 { continue; }
            let g = (g1 + g2) / 2;
            // Use log ratios to be exposure-invariant.
            let rg = ((r as f32).log2() - (g as f32).log2()) * 256.0;
            let bg = ((b as f32).log2() - (g as f32).log2()) * 256.0;
            let irg = (rg as i32 + OFFSET).clamp(0, HIST_SIZE as i32 - 1) as usize;
            let ibg = (bg as i32 + OFFSET).clamp(0, HIST_SIZE as i32 - 1) as usize;
            hist_rg[irg] += 1;
            hist_bg[ibg] += 1;
        }
    }

    let rg_peak = if use_max { peak_max(&hist_rg) } else { peak_median(&hist_rg) };
    let bg_peak = if use_max { peak_max(&hist_bg) } else { peak_median(&hist_bg) };

    let rg_log = (rg_peak as f32 - OFFSET as f32) / 256.0;
    let bg_log = (bg_peak as f32 - OFFSET as f32) / 256.0;

    // Multipliers: scale R and B to make them equal to G.
    let r_mul = 2f32.powf(-rg_log);
    let b_mul = 2f32.powf(-bg_log);

    tracing::debug!(r_mul, b_mul, rg_peak, bg_peak, "computed WB multipliers");
    [r_mul, 1.0, b_mul]
}

fn peak_max(hist: &[u32]) -> usize {
    hist.iter().enumerate().max_by_key(|&(_, v)| v).map(|(i, _)| i).unwrap_or(hist.len() / 2)
}

fn peak_median(hist: &[u32]) -> usize {
    let total: u32 = hist.iter().sum();
    let mut cum = 0u32;
    for (i, v) in hist.iter().enumerate() {
        cum += v;
        if cum >= total / 2 { return i; }
    }
    hist.len() / 2
}

/// Convert colour temperature (Kelvin) to approximate RGB multipliers.
/// Based on the CIE daylight locus fit from the original kelvin.c.
pub fn kelvin_to_rgb(temp_k: f64) -> [f32; 3] {
    let xd = if temp_k <= 4000.0 {
        0.27475e9 / (temp_k * temp_k * temp_k)
        - 0.98598e6 / (temp_k * temp_k)
        + 1.17444e3 / temp_k
        + 0.145986
    } else if temp_k <= 7000.0 {
        -4.607e9 / (temp_k * temp_k * temp_k)
        + 2.9678e6 / (temp_k * temp_k)
        + 0.09911e3 / temp_k
        + 0.244063
    } else {
        -2.0064e9 / (temp_k * temp_k * temp_k)
        + 1.9018e6 / (temp_k * temp_k)
        + 0.24748e3 / temp_k
        + 0.237040
    };
    let yd = -3.0 * xd * xd + 2.87 * xd - 0.275;

    let x = xd / yd;
    let y = 1.0;
    let z = (1.0 - xd - yd) / yd;

    // XYZ → linear sRGB (from original kelvin.c matrix)
    let r = ( 3.24071 * x - 1.53726 * y - 0.498571 * z) as f32;
    let g = (-0.969258 * x + 1.87599 * y + 0.0415557 * z) as f32;
    let b = ( 0.0556352 * x - 0.203996 * y + 1.05707 * z) as f32;

    let max = r.max(g).max(b);
    [r / max, g / max, b / max]
}
