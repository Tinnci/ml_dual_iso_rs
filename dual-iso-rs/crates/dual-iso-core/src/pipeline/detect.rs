use crate::error::DualIsoError;
use crate::types::{IsoLinePattern, RawImage};

/// Analyse a dual-ISO raw image and determine which row positions
/// (by `y % 4`) carry the high-ISO ("bright") exposure.
///
/// Strategy: compute the per-row mean of every other row pair across the
/// full image width.  Rows that consistently show higher mean values are
/// the high-ISO rows.
pub fn analyze_iso_lines(raw: &RawImage) -> Result<IsoLinePattern, DualIsoError> {
    let w = raw.buffer.width;
    let h = raw.buffer.height;

    if h < 8 {
        return Err(DualIsoError::ImageTooSmall);
    }

    // Accumulate mean for each of the 4 row-phase slots (y % 4).
    let mut sum = [0f64; 4];
    let mut count = [0usize; 4];

    // Use centre 80% of rows/cols to avoid border artefacts.
    let y0 = h / 10;
    let y1 = h - h / 10;
    let x0 = w / 10;
    let x1 = w - w / 10;

    for y in y0..y1 {
        let phase = y % 4;
        for x in x0..x1 {
            sum[phase] += raw.buffer.pixel(x, y) as f64;
            count[phase] += 1;
        }
    }

    let mean: [f64; 4] = std::array::from_fn(|i| {
        if count[i] > 0 { sum[i] / count[i] as f64 } else { 0.0 }
    });

    // The overall mean separates bright from dark phases.
    let total_mean = mean.iter().sum::<f64>() / 4.0;
    let is_bright: [bool; 4] = std::array::from_fn(|i| mean[i] > total_mean);

    let bright_count = is_bright.iter().filter(|&&b| b).count();
    if bright_count == 0 || bright_count == 4 {
        return Err(DualIsoError::NotDualIso);
    }

    // Estimate ISO ratio from mean difference.
    let bright_mean: f64 = mean.iter().zip(is_bright.iter())
        .filter(|&(_, b)| *b).map(|(m, _)| m).sum::<f64>()
        / bright_count as f64;
    let dark_mean: f64 = mean.iter().zip(is_bright.iter())
        .filter(|&(_, b)| !b).map(|(m, _)| m).sum::<f64>()
        / (4 - bright_count) as f64;

    let ratio = if dark_mean > 0.0 { (bright_mean / dark_mean).log2().round() as u32 } else { 3 };
    let iso_highlight = 100u32 * (1 << ratio);
    let iso_lowlight = 100u32;

    tracing::debug!(
        ?mean, ?is_bright, iso_lowlight, iso_highlight,
        "ISO line detection"
    );

    Ok(IsoLinePattern { is_bright, iso_lowlight, iso_highlight })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RawBuffer, RawMetadata};

    #[test]
    fn detects_alternating_iso() {
        let w = 20;
        let h = 40;
        let mut buf = RawBuffer::new(w, h);
        // Rows 0,1 bright, rows 2,3 dark → phases 0,1 bright
        for y in 0..h {
            let bright = (y % 4) < 2;
            for x in 0..w {
                buf.set_pixel(x, y, if bright { 8000 } else { 2000 });
            }
        }
        let raw = RawImage { buffer: buf, meta: RawMetadata::default() };
        let pat = analyze_iso_lines(&raw).unwrap();
        assert!(pat.is_bright[0]);
        assert!(pat.is_bright[1]);
        assert!(!pat.is_bright[2]);
        assert!(!pat.is_bright[3]);
    }
}
