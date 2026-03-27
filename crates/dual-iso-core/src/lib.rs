pub mod error;
pub mod types;
pub mod raw_io;
pub mod pipeline;
pub mod dng_output;

pub use error::DualIsoError;
pub use types::*;

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
        stripe_fix::fix_stripes(&mut bright_buf);
        stripe_fix::fix_stripes(&mut dark_buf);
    }

    if config.bad_pixels != BadPixelFix::Disabled {
        tracing::info!("fixing bad pixels");
        let aggressive = config.bad_pixels == BadPixelFix::Aggressive;
        bad_pixels::fix_bad_pixels(&mut bright_buf, aggressive, config.mark_bad_pixels_black);
        bad_pixels::fix_bad_pixels(&mut dark_buf, aggressive, config.mark_bad_pixels_black);
    }

    let (ev2raw, raw2ev) = build_ev_tables(raw.meta.black_level, raw.meta.white_level);

    tracing::info!("interpolating (method={:?})", config.interp_method);
    let bright_interp = interpolate::interpolate(&bright_buf, config, &raw.meta, &ev2raw, &raw2ev);
    let dark_interp = interpolate::interpolate(&dark_buf, config, &raw.meta, &ev2raw, &raw2ev);

    tracing::info!("blending dual-ISO planes");
    let mut blended = blend::blend_iso_planes(
        &bright_interp,
        &dark_interp,
        &pattern,
        config,
        &ev2raw,
        &raw2ev,
        raw.meta.black_level,
        raw.meta.white_level,
    );

    if config.chroma_smooth != ChromaSmoothSize::None {
        tracing::info!("chroma smoothing ({:?})", config.chroma_smooth);
        chroma_smooth::chroma_smooth(&mut blended, config.chroma_smooth, &raw2ev, &ev2raw);
    }

    let wb_multipliers = white_balance::compute_wb(&blended, &raw.meta, &config.white_balance);
    tracing::debug!(?wb_multipliers, "white balance");

    let result = RawImage { buffer: blended, meta: raw.meta };
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
