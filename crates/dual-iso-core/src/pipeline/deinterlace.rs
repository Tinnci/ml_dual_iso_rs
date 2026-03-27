use crate::types::{IsoLinePattern, RawBuffer};

/// Split an interleaved dual-ISO Bayer buffer into two half-height buffers:
/// one containing the bright (high-ISO) rows and one the dark (low-ISO) rows.
///
/// Each output buffer has the same width as the input but roughly half the
/// height (depending on how many phases are bright vs dark).
pub fn split_iso_planes(input: &RawBuffer, pattern: &IsoLinePattern) -> (RawBuffer, RawBuffer) {
    let w = input.width;
    let h = input.height;

    // Count output rows for each plane.
    let bright_rows: Vec<usize> = (0..h).filter(|&y| pattern.is_bright[y % 4]).collect();
    let dark_rows: Vec<usize> = (0..h).filter(|&y| !pattern.is_bright[y % 4]).collect();

    let mut bright = RawBuffer::new(w, bright_rows.len());
    let mut dark = RawBuffer::new(w, dark_rows.len());

    for (dst_y, &src_y) in bright_rows.iter().enumerate() {
        let src_off = src_y * w;
        let dst_off = dst_y * w;
        bright.data[dst_off..dst_off + w].copy_from_slice(&input.data[src_off..src_off + w]);
    }
    for (dst_y, &src_y) in dark_rows.iter().enumerate() {
        let src_off = src_y * w;
        let dst_off = dst_y * w;
        dark.data[dst_off..dst_off + w].copy_from_slice(&input.data[src_off..src_off + w]);
    }

    (bright, dark)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IsoLinePattern;

    #[test]
    fn split_produces_correct_height() {
        let mut buf = RawBuffer::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                buf.set_pixel(x, y, if y % 4 < 2 { 8000 } else { 2000 });
            }
        }
        let pat = IsoLinePattern {
            is_bright: [true, true, false, false],
            iso_lowlight: 100,
            iso_highlight: 800,
        };
        let (bright, dark) = split_iso_planes(&buf, &pat);
        assert_eq!(bright.height, 4);
        assert_eq!(dark.height, 4);
        assert_eq!(bright.pixel(0, 0), 8000);
        assert_eq!(dark.pixel(0, 0), 2000);
    }
}
