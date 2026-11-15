use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use flate2::Compression as ZlibCompression;
use flate2::write::ZlibEncoder;

use crate::error::DualIsoError;
use crate::types::{Compression, ProcessConfig, RawImage};

/// Write a processed `RawImage` as a minimal 16-bit DNG file.
///
/// Supports uncompressed (Compression=1) and lossless Deflate (Compression=8)
/// with horizontal-differencing predictor.
pub fn write_dng(
    path: &Path,
    image: &RawImage,
    config: &ProcessConfig,
) -> Result<(), DualIsoError> {
    let use_deflate = config.compression == Compression::Lossless;
    if use_deflate {
        write_dng_deflate(path, image)
    } else {
        write_dng_uncompressed(path, image)
    }
}

// ─── Uncompressed DNG ────────────────────────────────────────────────────────

fn write_dng_uncompressed(path: &Path, image: &RawImage) -> Result<(), DualIsoError> {
    let w = image.buffer.width as u32;
    let h = image.buffer.height as u32;
    let strip_bytes = w * h * 2;
    let layout = IfdLayout::build(image, false);

    let mut out: Vec<u8> = Vec::with_capacity((layout.strip_off + strip_bytes) as usize);
    write_tiff_header(&mut out);
    layout.write_ifd(&mut out, image, w, h, strip_bytes, false);
    layout.write_blobs(&mut out);

    for &pixel in &image.buffer.data {
        out.extend_from_slice(&pixel.to_le_bytes());
    }

    flush_to_file(path, &out)
}

// ─── Deflate-compressed DNG ──────────────────────────────────────────────────

fn write_dng_deflate(path: &Path, image: &RawImage) -> Result<(), DualIsoError> {
    let w = image.buffer.width;
    let h = image.buffer.height;

    // Apply horizontal-differencing predictor (TIFF Predictor=2, 16-bit samples).
    let predicted = predict_horizontal(image);

    // Compress with zlib (Compression=8).
    let compressed = {
        let mut enc =
            ZlibEncoder::new(Vec::with_capacity(predicted.len()), ZlibCompression::best());
        enc.write_all(&predicted).map_err(DualIsoError::Io)?;
        enc.finish().map_err(DualIsoError::Io)?
    };

    let strip_bytes_compressed = compressed.len() as u32;
    let layout = IfdLayout::build(image, true);

    let mut out: Vec<u8> = Vec::with_capacity(layout.strip_off as usize + compressed.len());
    write_tiff_header(&mut out);
    layout.write_ifd(
        &mut out,
        image,
        w as u32,
        h as u32,
        strip_bytes_compressed,
        true,
    );
    layout.write_blobs(&mut out);
    out.extend_from_slice(&compressed);

    flush_to_file(path, &out)
}

// ─── Horizontal-differencing predictor ───────────────────────────────────────

/// Apply TIFF Predictor=2 (horizontal differencing) to 16-bit LE rows.
/// Returns raw bytes (LE u16 pairs) suitable for compression.
fn predict_horizontal(image: &RawImage) -> Vec<u8> {
    let w = image.buffer.width;
    let h = image.buffer.height;
    let mut out = Vec::with_capacity(w * h * 2);

    for row in 0..h {
        let mut prev: u16 = 0;
        for col in 0..w {
            let px = image.buffer.pixel(col, row);
            let diff = px.wrapping_sub(prev);
            out.extend_from_slice(&diff.to_le_bytes());
            prev = px;
        }
    }
    out
}

// ─── Shared IFD helpers ───────────────────────────────────────────────────────

const IFD_START: u32 = 8;

fn ifd_bytes(n_entries: u32) -> u32 {
    2 + n_entries * 12 + 4
}

fn padded(n: usize) -> u32 {
    n as u32 + (n % 2) as u32
}

/// Pack up to 4 bytes into a 4-byte TIFF IFD Value/Offset field (inline storage).
#[inline]
fn pack_inline_bytes(bytes: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    let n = bytes.len().min(4);
    buf[..n].copy_from_slice(&bytes[..n]);
    u32::from_le_bytes(buf)
}

fn cstr_bytes(s: &str, fallback: &str) -> Vec<u8> {
    let text = if s.trim().is_empty() { fallback } else { s };
    let mut bytes = text.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

struct IfdLayout {
    n_entries: u16,
    make_len: u32,
    make_val: u32,
    model_len: u32,
    model_val: u32,
    unique_len: u32,
    unique_val: u32,
    black_rat_off: u32,
    color_mat_off: u32,
    strip_off: u32,
    blobs: Vec<Vec<u8>>,
}

impl IfdLayout {
    fn build(image: &RawImage, deflate: bool) -> Self {
        let n_entries: u16 = if deflate { 26 } else { 25 };
        let mut cursor = IFD_START + ifd_bytes(n_entries as u32);
        let mut blobs = Vec::new();

        let mut alloc_or_inline = |bytes: Vec<u8>| -> (u32, u32) {
            let len = bytes.len() as u32;
            if len <= 4 {
                (len, pack_inline_bytes(&bytes))
            } else {
                let off = cursor;
                cursor += padded(bytes.len());
                blobs.push(bytes);
                (len, off)
            }
        };

        let make_bytes = cstr_bytes(&image.meta.camera_make, "Canon");
        let model_bytes = cstr_bytes(&image.meta.camera_model, "EOS");
        let unique_str = format!(
            "{} {}",
            image.meta.camera_make.trim(),
            image.meta.camera_model.trim()
        )
        .trim()
        .to_string();
        let unique_bytes = cstr_bytes(&unique_str, "Dual-ISO Camera");

        let (make_len, make_val) = alloc_or_inline(make_bytes);
        let (model_len, model_val) = alloc_or_inline(model_bytes);
        let (unique_len, unique_val) = alloc_or_inline(unique_bytes);

        let black = image.meta.black_level as u32;
        let black_rational: Vec<u8> = [black, 1u32].iter().flat_map(|v| v.to_le_bytes()).collect();
        let black_rat_off = cursor;
        cursor += padded(black_rational.len());
        blobs.push(black_rational);

        // Identity 3x3 SRATIONAL ColorMatrix1 (9 * 8 = 72 bytes)
        let color_matrix: Vec<u8> = [1i32, 1, 0, 1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 1, 0, 1, 1, 1]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let color_mat_off = cursor;
        cursor += padded(color_matrix.len());
        blobs.push(color_matrix);

        Self {
            n_entries,
            make_len,
            make_val,
            model_len,
            model_val,
            unique_len,
            unique_val,
            black_rat_off,
            color_mat_off,
            strip_off: cursor,
            blobs,
        }
    }

    fn write_ifd(
        &self,
        out: &mut Vec<u8>,
        image: &RawImage,
        w: u32,
        h: u32,
        strip_bytes: u32,
        deflate: bool,
    ) {
        let white = image.meta.white_level as u32;
        let cfa_inline = pack_inline_bytes(&image.meta.bayer_pattern.cfa_bytes());
        let dng_ver_inline = pack_inline_bytes(&[1, 4, 0, 0]);
        let dng_back_inline = pack_inline_bytes(&[1, 1, 0, 0]);
        let cfa_plane_color_inline = pack_inline_bytes(&[0, 1, 2]);

        out.extend_from_slice(&self.n_entries.to_le_bytes());

        let mut entry = |tag: u16, typ: u16, count: u32, val: u32| {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&typ.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&val.to_le_bytes());
        };

        // Tags MUST be in strictly ascending numerical order (TIFF 6.0 spec).
        entry(254, 4, 1, 0); // NewSubfileType
        entry(256, 4, 1, w); // ImageWidth
        entry(257, 4, 1, h); // ImageLength
        entry(258, 3, 1, 16); // BitsPerSample
        entry(259, 3, 1, if deflate { 8 } else { 1 }); // Compression
        entry(262, 3, 1, 32803); // PhotometricInterpretation = CFA
        entry(271, 2, self.make_len, self.make_val); // Make
        entry(272, 2, self.model_len, self.model_val); // Model
        entry(273, 4, 1, self.strip_off); // StripOffsets
        entry(274, 3, 1, 1); // Orientation = TopLeft
        entry(277, 3, 1, 1); // SamplesPerPixel
        entry(278, 4, 1, h); // RowsPerStrip
        entry(279, 4, 1, strip_bytes); // StripByteCounts
        entry(284, 3, 1, 1); // PlanarConfiguration = Chunky
        if deflate {
            entry(317, 3, 1, 2); // Predictor = HorizontalDifferencing
        }
        entry(33421, 3, 2, pack_shorts(2, 2)); // CFARepeatPatternDim
        entry(33422, 1, 4, cfa_inline); // CFAPattern (4 bytes -> inline!)
        entry(50706, 1, 4, dng_ver_inline); // DNGVersion (4 bytes -> inline!)
        entry(50707, 1, 4, dng_back_inline); // DNGBackwardVersion (4 bytes -> inline!)
        entry(50708, 2, self.unique_len, self.unique_val); // UniqueCameraModel
        entry(50710, 1, 3, cfa_plane_color_inline); // CFAPlaneColor (3 bytes -> inline!)
        entry(50711, 3, 1, 1); // CFALayout = Rectangular
        entry(50714, 5, 1, self.black_rat_off); // BlackLevel (RATIONAL)
        entry(50717, 4, 1, white); // WhiteLevel
        entry(50721, 10, 9, self.color_mat_off); // ColorMatrix1 (SRATIONAL x 9)
        entry(50778, 3, 1, 21); // CalibrationIlluminant1 = D65

        out.extend_from_slice(&0u32.to_le_bytes()); // Next IFD offset = 0
    }

    fn write_blobs(&self, out: &mut Vec<u8>) {
        for b in &self.blobs {
            out.extend_from_slice(b);
            if b.len() % 2 == 1 {
                out.push(0);
            }
        }
    }
}

fn write_tiff_header(out: &mut Vec<u8>) {
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&IFD_START.to_le_bytes());
}

fn flush_to_file(path: &Path, data: &[u8]) -> Result<(), DualIsoError> {
    let file = File::create(path).map_err(DualIsoError::Io)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(data).map_err(DualIsoError::Io)?;
    writer.flush().map_err(DualIsoError::Io)?;
    Ok(())
}

/// Pack two u16 into a single u32 LE value field (used for CFARepeatPatternDim).
#[inline]
fn pack_shorts(a: u16, b: u16) -> u32 {
    (a as u32) | ((b as u32) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BayerPattern, RawBuffer, RawMetadata};

    fn sample_image(make: &str, model: &str) -> RawImage {
        let mut buf = RawBuffer::new(4, 4);
        for (i, px) in buf.data.iter_mut().enumerate() {
            *px = 2048 + (i as u16) * 100;
        }
        RawImage {
            buffer: buf,
            meta: RawMetadata {
                camera_make: make.to_string(),
                camera_model: model.to_string(),
                black_level: 2048,
                white_level: 15000,
                bayer_pattern: BayerPattern::Rggb,
                ..Default::default()
            },
        }
    }

    fn verify_ifd(data: &[u8]) {
        assert_eq!(&data[0..2], b"II");
        assert_eq!(u16::from_le_bytes([data[2], data[3]]), 42);
        let ifd_off = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes([data[ifd_off], data[ifd_off + 1]]) as usize;

        let mut prev_tag = 0u16;
        let mut dng_ver = None;
        let mut cfa_pat = None;
        let mut strip_off = 0usize;
        let mut strip_len = 0usize;

        for i in 0..count {
            let off = ifd_off + 2 + i * 12;
            let tag = u16::from_le_bytes([data[off], data[off + 1]]);
            let typ = u16::from_le_bytes([data[off + 2], data[off + 3]]);
            let cnt = u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap());
            let val_bytes = &data[off + 8..off + 12];
            let val = u32::from_le_bytes(val_bytes.try_into().unwrap());

            assert!(
                tag > prev_tag,
                "IFD tags must be strictly ascending: {prev_tag} -> {tag}"
            );
            prev_tag = tag;

            let elem_size = match typ {
                1 | 2 => 1,
                3 => 2,
                4 => 4,
                5 | 10 => 8,
                _ => panic!("unexpected TIFF type {typ}"),
            };
            if elem_size * cnt as usize > 4 {
                assert!(
                    (val as usize) + elem_size * cnt as usize <= data.len(),
                    "out-of-bounds offset for tag {tag}"
                );
            }

            match tag {
                273 => strip_off = val as usize,
                279 => strip_len = val as usize,
                33422 => cfa_pat = Some(val_bytes.to_vec()),
                50706 => dng_ver = Some(val_bytes.to_vec()),
                _ => {}
            }
        }

        assert_eq!(dng_ver.unwrap(), vec![1, 4, 0, 0]);
        assert_eq!(cfa_pat.unwrap(), vec![0, 1, 1, 2]);
        assert_eq!(strip_off + strip_len, data.len());
    }

    #[test]
    fn writes_valid_uncompressed_and_deflate_dng() {
        let dir = std::env::temp_dir();
        for (idx, (comp, make, model)) in [
            (Compression::None, "Canon", "Canon EOS 5D Mark II"),
            (Compression::None, "A", "B"),
            (Compression::Lossless, "Canon", "Canon EOS 5D Mark II"),
        ]
        .into_iter()
        .enumerate()
        {
            let path = dir.join(format!("ml_dual_iso_test_{idx}.dng"));
            let img = sample_image(make, model);
            let cfg = ProcessConfig {
                compression: comp,
                ..Default::default()
            };
            write_dng(&path, &img, &cfg).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            verify_ifd(&bytes);
            let _ = std::fs::remove_file(&path);
        }
    }
}
