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

    let mut out: Vec<u8> = Vec::with_capacity(8 + 2 + 18 * 12 + 4 + 256 + (w * h * 2) as usize);

    let offsets = build_blobs(image);
    let tag_start = IFD_START + ifd_bytes(18);
    let strip_off = tag_start + blob_total(&offsets);
    let strip_bytes = w * h * 2;

    write_tiff_header(&mut out);
    out.extend_from_slice(&18u16.to_le_bytes());
    write_ifd_entries(
        &mut out,
        image,
        w,
        h,
        &offsets,
        tag_start,
        strip_off,
        strip_bytes,
        false,
    );
    out.extend_from_slice(&0u32.to_le_bytes());

    write_blobs(&mut out, &offsets);

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

    let offsets = build_blobs(image);
    let tag_start = IFD_START + ifd_bytes(19); // 19 entries (adds Predictor tag)
    let strip_off = tag_start + blob_total(&offsets);

    let mut out: Vec<u8> = Vec::with_capacity(8 + 2 + 19 * 12 + 4 + 256 + compressed.len());

    write_tiff_header(&mut out);
    out.extend_from_slice(&19u16.to_le_bytes());
    write_ifd_entries(
        &mut out,
        image,
        w as u32,
        h as u32,
        &offsets,
        tag_start,
        strip_off,
        strip_bytes_compressed,
        true,
    );
    out.extend_from_slice(&0u32.to_le_bytes());

    write_blobs(&mut out, &offsets);

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

/// Collect the variable-length tag blobs and their offsets from `tag_start`.
fn build_blobs(image: &RawImage) -> BlobOffsets {
    let cfa_bytes = image.meta.bayer_pattern.cfa_bytes().to_vec();
    let dng_ver: Vec<u8> = vec![1, 4, 0, 0];
    let black = image.meta.black_level as u32;
    let white = image.meta.white_level as u32;
    let _ = white; // used below in entry()

    let mut make_bytes = image.meta.camera_make.as_bytes().to_vec();
    make_bytes.push(0);
    let mut model_bytes = image.meta.camera_model.as_bytes().to_vec();
    model_bytes.push(0);
    let black_rational: Vec<u8> = [black, 1u32].iter().flat_map(|v| v.to_le_bytes()).collect();

    let blobs = vec![make_bytes, model_bytes, cfa_bytes, dng_ver, black_rational];
    BlobOffsets { blobs }
}

struct BlobOffsets {
    blobs: Vec<Vec<u8>>,
}

fn blob_total(bo: &BlobOffsets) -> u32 {
    bo.blobs.iter().map(|b| padded(b.len())).sum()
}

fn compute_offsets(bo: &BlobOffsets, tag_start: u32) -> [u32; 5] {
    let mut off = tag_start;
    let mut offsets = [0u32; 5];
    for (i, b) in bo.blobs.iter().enumerate() {
        offsets[i] = off;
        off += padded(b.len());
    }
    offsets
}

fn write_tiff_header(out: &mut Vec<u8>) {
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&IFD_START.to_le_bytes());
}

#[allow(clippy::too_many_arguments)]
fn write_ifd_entries(
    out: &mut Vec<u8>,
    image: &RawImage,
    w: u32,
    h: u32,
    bo: &BlobOffsets,
    tag_start: u32,
    strip_off: u32,
    strip_bytes: u32,
    deflate: bool,
) {
    let offsets = compute_offsets(bo, tag_start);
    let make_off = offsets[0];
    let model_off = offsets[1];
    let cfa_off = offsets[2];
    let dng_ver_off = offsets[3];
    let black_rat_off = offsets[4];

    let white = image.meta.white_level as u32;
    let make_len = bo.blobs[0].len() as u32;
    let model_len = bo.blobs[1].len() as u32;

    let mut entry = |tag: u16, typ: u16, count: u32, val: u32| {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&val.to_le_bytes());
    };

    entry(254, 4, 1, 0); // NewSubfileType
    entry(256, 4, 1, w); // ImageWidth
    entry(257, 4, 1, h); // ImageLength
    entry(258, 3, 1, 16); // BitsPerSample
    entry(259, 3, 1, if deflate { 8 } else { 1 }); // Compression
    entry(262, 3, 1, 32803); // PhotometricInterp = CFA
    entry(271, 2, make_len, make_off); // Make
    entry(272, 2, model_len, model_off); // Model
    entry(273, 4, 1, strip_off); // StripOffsets
    entry(277, 3, 1, 1); // SamplesPerPixel
    entry(278, 4, 1, h); // RowsPerStrip
    entry(279, 4, 1, strip_bytes); // StripByteCounts
    entry(284, 3, 1, 1); // PlanarConfiguration
    if deflate {
        entry(317, 3, 1, 2); // Predictor = HorizontalDifferencing
    }
    entry(33421, 3, 2, pack_shorts(2, 2)); // CFARepeatPatternDim
    entry(33422, 1, 4, cfa_off); // CFAPattern
    entry(50706, 1, 4, dng_ver_off); // DNGVersion
    entry(50714, 5, 1, black_rat_off); // BlackLevel (RATIONAL)
    entry(50717, 4, 1, white); // WhiteLevel
}

fn write_blobs(out: &mut Vec<u8>, bo: &BlobOffsets) {
    for b in &bo.blobs {
        out.extend_from_slice(b);
        if b.len() % 2 == 1 {
            out.push(0);
        }
    }
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
