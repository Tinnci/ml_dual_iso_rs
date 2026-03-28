pub mod dng_output;
pub mod error;
pub mod pipeline;
pub mod raw_io;
pub mod types;

pub use error::DualIsoError;
pub use types::*;

/// Quickly analyse a `RawImage` to determine whether it is dual-ISO.
///
/// This is a fast pre-processing check: it only examines row-phase
/// statistics.  No pixel interpolation or blending is performed.
pub fn quick_analyze(raw: &RawImage) -> DualIsoAnalysis {
    use pipeline::detect;

    let w = raw.buffer.width;
    let h = raw.buffer.height;

    if h < 8 {
        return DualIsoAnalysis {
            is_dual_iso: false,
            confidence: 0.0,
            phase_means: [0.0; 4],
            pattern: "----".into(),
            iso_low: 0,
            iso_high: 0,
            status: "Image too small for analysis".into(),
        };
    }

    // Compute per-phase means (centre 80%).
    let y0 = h / 10;
    let y1 = h - h / 10;
    let x0 = w / 10;
    let x1 = w - w / 10;

    let mut sum = [0f64; 4];
    let mut count = [0usize; 4];
    for y in y0..y1 {
        let phase = y % 4;
        for x in x0..x1 {
            sum[phase] += raw.buffer.pixel(x, y) as f64;
            count[phase] += 1;
        }
    }

    let phase_means: [f64; 4] = std::array::from_fn(|i| {
        if count[i] > 0 {
            sum[i] / count[i] as f64
        } else {
            0.0
        }
    });

    let total_mean = phase_means.iter().sum::<f64>() / 4.0;
    let is_bright: [bool; 4] = std::array::from_fn(|i| phase_means[i] > total_mean);
    let bright_count = is_bright.iter().filter(|&&b| b).count();

    let pattern: String = is_bright
        .iter()
        .map(|&b| if b { 'B' } else { 'd' })
        .collect();

    // Strict dual-ISO criteria (from original cr2hdr):
    //   - exactly 2 bright and 2 dark phases
    //   - alternating: is_bright[0] != is_bright[2] && is_bright[1] != is_bright[3]
    let alternating =
        bright_count == 2 && is_bright[0] != is_bright[2] && is_bright[1] != is_bright[3];

    // Compute EV separation between bright/dark groups.
    let (bright_mean, dark_mean) = if bright_count > 0 && bright_count < 4 {
        let bm: f64 = phase_means
            .iter()
            .zip(is_bright.iter())
            .filter(|&(_, &b)| b)
            .map(|(m, _)| m)
            .sum::<f64>()
            / bright_count as f64;
        let dm: f64 = phase_means
            .iter()
            .zip(is_bright.iter())
            .filter(|&(_, &b)| !b)
            .map(|(m, _)| m)
            .sum::<f64>()
            / (4 - bright_count) as f64;
        (bm, dm)
    } else {
        (total_mean, total_mean)
    };

    let ev_sep = if dark_mean > 1.0 {
        (bright_mean / dark_mean).log2()
    } else {
        0.0
    };

    // Confidence: alternating pattern + meaningful EV separation (>0.5 stop).
    let confidence = if alternating && ev_sep > 0.5 {
        (ev_sep / 4.0).clamp(0.3, 1.0) // ~1.0 at 4-stop separation
    } else if bright_count == 2 && ev_sep > 0.3 {
        0.2 // weak signal
    } else {
        0.0
    };

    let is_dual_iso = alternating && ev_sep > 0.5;

    let iso_ratio = ev_sep.round() as u32;
    let iso_low = 100;
    let iso_high = if is_dual_iso {
        100 * (1 << iso_ratio)
    } else {
        0
    };

    let status = if is_dual_iso {
        format!(
            "Dual ISO detected: ~{:.1} EV separation ({}/{})",
            ev_sep, iso_low, iso_high
        )
    } else if bright_count == 2 {
        format!("Unlikely dual ISO (EV sep = {ev_sep:.2}, pattern = {pattern})")
    } else {
        "Not dual ISO".into()
    };

    // Also try detect::analyze_iso_lines to see if the pipeline would accept it.
    let pipeline_ok = detect::analyze_iso_lines(raw).is_ok();

    let final_status = if is_dual_iso && pipeline_ok {
        status
    } else if is_dual_iso && !pipeline_ok {
        format!("{status} (pipeline may reject)")
    } else {
        status
    };

    DualIsoAnalysis {
        is_dual_iso,
        confidence,
        phase_means,
        pattern,
        iso_low,
        iso_high,
        status: final_status,
    }
}

/// Run the full dual-ISO processing pipeline on a RawImage.
/// Returns the processed 16-bit output image ready for DNG writing.
pub fn process(raw: RawImage, config: &ProcessConfig) -> Result<RawImage, DualIsoError> {
    use pipeline::*;

    tracing::info!("detecting ISO line pattern");
    let pattern = detect::analyze_iso_lines(&raw)?;
    tracing::debug!(?pattern, "ISO line pattern");

    tracing::info!("deinterlacing ISO planes");
    let (mut bright_buf, mut dark_buf) = deinterlace::split_iso_planes(&raw.buffer, &pattern);

    if config.use_stripe_fix {
        tracing::info!("fixing horizontal stripes");
        rayon::join(
            || stripe_fix::fix_stripes(&mut bright_buf),
            || stripe_fix::fix_stripes(&mut dark_buf),
        );
    }

    if config.bad_pixels != BadPixelFix::Disabled {
        tracing::info!("fixing bad pixels");
        let aggressive = config.bad_pixels == BadPixelFix::Aggressive;
        let mark = config.mark_bad_pixels_black;
        rayon::join(
            || bad_pixels::fix_bad_pixels(&mut bright_buf, aggressive, mark),
            || bad_pixels::fix_bad_pixels(&mut dark_buf, aggressive, mark),
        );
    }

    let (ev2raw, raw2ev) = build_ev_tables(raw.meta.black_level, raw.meta.white_level);

    tracing::info!("interpolating (method={:?})", config.interp_method);
    let (bright_interp, dark_interp) = rayon::join(
        || interpolate::interpolate(&bright_buf, config, &raw.meta, &ev2raw, &raw2ev),
        || interpolate::interpolate(&dark_buf, config, &raw.meta, &ev2raw, &raw2ev),
    );

    tracing::info!("blending dual-ISO planes");
    let mut blended = blend::blend_iso_planes(blend::BlendParams {
        bright: &bright_interp,
        dark: &dark_interp,
        _pattern: &pattern,
        _config: config,
        ev2raw: &ev2raw,
        raw2ev: &raw2ev,
        black_level: raw.meta.black_level,
        white_level: raw.meta.white_level,
    });

    if config.chroma_smooth != ChromaSmoothSize::None {
        tracing::info!("chroma smoothing ({:?})", config.chroma_smooth);
        chroma_smooth::chroma_smooth(&mut blended, config.chroma_smooth, &raw2ev, &ev2raw);
    }

    let wb_multipliers = white_balance::compute_wb(&blended, &raw.meta, &config.white_balance);
    tracing::debug!(?wb_multipliers, "white balance");

    let result = RawImage {
        buffer: blended,
        meta: raw.meta,
    };
    Ok(result)
}

/// Build linear-to-EV and EV-to-linear lookup tables.
/// EV_RESOLUTION slots per stop, range -10..+14 EV.
pub fn build_ev_tables(black_level: u16, white_level: u16) -> (Vec<u16>, Vec<i32>) {
    const EV_RES: i32 = crate::types::EV_RESOLUTION;
    const MAX_EV: i32 = 14 * EV_RES;
    const MIN_EV: i32 = -10 * EV_RES;
    const LUT_SIZE: usize = (MAX_EV - MIN_EV) as usize;

    // ev2raw: EV index (offset by 10*EV_RES) → raw 16-bit value
    let mut ev2raw = vec![0u16; LUT_SIZE];
    let range = (white_level - black_level) as f64;
    for (i, slot) in ev2raw.iter_mut().enumerate() {
        let ev = (i as i32 + MIN_EV) as f64 / EV_RES as f64;
        let linear = range * 2f64.powf(ev);
        let raw = (linear + black_level as f64).round() as i64;
        *slot = raw.clamp(0, u16::MAX as i64) as u16;
    }

    // raw2ev: raw value (0..0xFFFFF) → EV index
    let mut raw2ev = vec![i32::MIN; 0x10_0000];
    for (raw_val, ev_idx) in raw2ev.iter_mut().enumerate() {
        let linear = (raw_val as f64) - black_level as f64;
        if linear <= 0.0 {
            *ev_idx = MIN_EV;
        } else {
            let ev = linear.log2() * EV_RES as f64 + 0.5;
            *ev_idx = (ev as i32).clamp(MIN_EV, MAX_EV - 1);
        }
    }

    (ev2raw, raw2ev)
}
