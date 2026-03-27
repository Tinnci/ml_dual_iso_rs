use std::path::Path;

use crate::error::DualIsoError;
use crate::types::{BayerPattern, ExifInfo, RawBuffer, RawImage, RawMetadata};

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

    // Extract EXIF via decoder metadata API.
    let exif = extract_exif(path);

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
        exif,
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

/// Extract EXIF fields using rawler's decoder metadata API.
/// Returns a default `ExifInfo` on any failure (non-fatal).
fn extract_exif(path: &Path) -> ExifInfo {
    use rawler::decoders::RawDecodeParams;
    use rawler::rawsource::RawSource;

    let Ok(source) = RawSource::new(path) else {
        return ExifInfo::default();
    };
    let Ok(decoder) = rawler::get_decoder(&source) else {
        return ExifInfo::default();
    };
    let params = RawDecodeParams::default();
    let Ok(meta) = decoder.raw_metadata(&source, &params) else {
        return ExifInfo::default();
    };
    let e = &meta.exif;

    // Shutter speed: prefer exposure_time rational (e.g. "1/500 s")
    let exposure_time = e.exposure_time.as_ref().map(|r| {
        if r.d == 1 {
            format!("{} s", r.n)
        } else {
            format!("{}/{} s", r.n, r.d)
        }
    });

    // f-number: e.g. "f/2.8"
    let fnumber = e.fnumber.as_ref().map(|r| {
        let v = r.n as f64 / r.d.max(1) as f64;
        format!("f/{v:.1}")
    });

    // ISO: prefer iso_speed over iso_speed_ratings
    let iso = e
        .iso_speed
        .or_else(|| e.iso_speed_ratings.map(|v| v as u32));

    // Focal length: e.g. "50 mm"
    let focal_length = e.focal_length.as_ref().map(|r| {
        let v = r.n as f64 / r.d.max(1) as f64;
        format!("{v:.0} mm")
    });

    ExifInfo {
        exposure_time,
        fnumber,
        iso,
        focal_length,
        date_time_original: e.date_time_original.clone(),
        lens_model: e.lens_model.clone(),
    }
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
