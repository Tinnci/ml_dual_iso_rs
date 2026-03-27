use std::path::Path;

use crate::error::DualIsoError;
use crate::types::{BayerPattern, RawBuffer, RawImage, RawMetadata};

/// Read a CR2 or DNG file using the `rawler` crate and convert it into
/// the crate-internal `RawImage` representation.
pub fn read_raw(path: &Path) -> Result<RawImage, DualIsoError> {
    use rawler::RawImageData;
    use rawler::rawimage::RawPhotometricInterpretation;

    // rawler's simplest public API: decode_file
    let raw = rawler::decode_file(path).map_err(|e| DualIsoError::DecodeError(e.to_string()))?;

    // Extract 16-bit pixel data.
    let (data, width, height) = match raw.data {
        RawImageData::Integer(v) => (v, raw.width, raw.height),
        RawImageData::Float(_) => {
            return Err(DualIsoError::UnsupportedFormat(
                "floating-point raw not supported".into(),
            ));
        }
    };

    // Determine Bayer pattern from photometric interpretation.
    let cfa_name = match &raw.photometric {
        RawPhotometricInterpretation::Cfa(cfg) => cfg.cfa.name.clone(),
        _ => "RGGB".to_string(),
    };
    let bayer_pattern = cfa_to_bayer(&cfa_name);

    // Black level: use the first rational value.
    let black_level = raw
        .blacklevel
        .levels
        .first()
        .map(|r| r.as_f32() as u16)
        .unwrap_or(0);

    // White level: first entry from the WhiteLevel wrapper.
    let white_level = raw
        .whitelevel
        .0
        .first()
        .map(|&v| v.min(u16::MAX as u32) as u16)
        .unwrap_or(u16::MAX);

    let pre_mul = [raw.wb_coeffs[0], raw.wb_coeffs[1], raw.wb_coeffs[2]];

    let meta = RawMetadata {
        black_level,
        white_level,
        bayer_pattern,
        color_temperature: None,
        pre_mul,
        rgb_cam: Default::default(),
        camera_make: raw.make.clone(),
        camera_model: raw.model.clone(),
        bits_per_pixel: raw.bps as u8,
        exif_blob: Vec::new(),
    };

    Ok(RawImage {
        buffer: RawBuffer {
            data,
            width,
            height,
        },
        meta,
    })
}

fn cfa_to_bayer(name: &str) -> BayerPattern {
    match name.to_uppercase().as_str() {
        "RGGB" => BayerPattern::Rggb,
        "GBRG" => BayerPattern::Gbrg,
        "BGGR" => BayerPattern::Bggr,
        "GRBG" => BayerPattern::Grbg,
        _ => BayerPattern::Rggb,
    }
}
