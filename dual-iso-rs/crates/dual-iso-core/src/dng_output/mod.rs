use std::io::{BufWriter, Write};
use std::path::Path;
use std::fs::File;

use crate::error::DualIsoError;
use crate::types::{ProcessConfig, RawImage};

/// Write a processed `RawImage` as a minimal 16-bit DNG file.
///
/// DNG is a profile of the TIFF format.  We construct the TIFF IFD by hand
/// using little-endian byte order, which keeps the implementation
/// dependency-free while remaining readable by any DNG-compatible software.
pub fn write_dng(path: &Path, image: &RawImage, _config: &ProcessConfig) -> Result<(), DualIsoError> {
    let w = image.buffer.width as u32;
    let h = image.buffer.height as u32;

    let mut out: Vec<u8> = Vec::with_capacity(
        8 + 2 + 18 * 12 + 4 + 256 + (w * h * 2) as usize,
    );

    // ── Content buffers for tag value blobs ──────────────────────────────────
    let cfa_bytes   = image.meta.bayer_pattern.cfa_bytes();
    let dng_ver: [u8; 4] = [1, 4, 0, 0];
    let black = image.meta.black_level as u32;
    let white = image.meta.white_level as u32;

    let mut make_bytes  = image.meta.camera_make.as_bytes().to_vec(); make_bytes.push(0);
    let mut model_bytes = image.meta.camera_model.as_bytes().to_vec(); model_bytes.push(0);
    // Black level as RATIONAL (num/denom as two u32 LE)
    let black_rational: Vec<u8> = [black, 1].iter().flat_map(|v| v.to_le_bytes()).collect();

    // ── Compute blob offsets ─────────────────────────────────────────────────
    // Layout: 8 (header) + 2 + 18*12 + 4 (IFD) = 8 + 2 + 216 + 4 = 230 bytes before blobs.
    const N_ENTRIES: u16 = 18;
    const IFD_START: u32 = 8;
    const IFD_BYTES: u32 = 2 + N_ENTRIES as u32 * 12 + 4;
    let mut tag_start: u32 = IFD_START + IFD_BYTES;

    fn padded(n: usize) -> u32 { n as u32 + (n % 2) as u32 }

    let make_off       = tag_start; tag_start += padded(make_bytes.len());
    let model_off      = tag_start; tag_start += padded(model_bytes.len());
    let cfa_off        = tag_start; tag_start += padded(cfa_bytes.len());
    let dng_ver_off    = tag_start; tag_start += padded(dng_ver.len());
    let black_rat_off  = tag_start; tag_start += padded(black_rational.len());
    // strip data starts right after blobs
    let strip_off: u32 = tag_start;
    let strip_bytes: u32 = w * h * 2;

    // ── TIFF header ──────────────────────────────────────────────────────────
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&IFD_START.to_le_bytes());

    // ── IFD ──────────────────────────────────────────────────────────────────
    out.extend_from_slice(&N_ENTRIES.to_le_bytes());

    let mut entry = |tag: u16, typ: u16, count: u32, val: u32| {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&val.to_le_bytes());
    };

    entry(254, 4, 1, 0);                                      // NewSubfileType
    entry(256, 4, 1, w);                                      // ImageWidth
    entry(257, 4, 1, h);                                      // ImageLength
    entry(258, 3, 1, 16);                                     // BitsPerSample
    entry(259, 3, 1, 1);                                      // Compression = none
    entry(262, 3, 1, 32803);                                  // PhotometricInterp = CFA
    entry(271, 2, make_bytes.len() as u32, make_off);         // Make
    entry(272, 2, model_bytes.len() as u32, model_off);       // Model
    entry(273, 4, 1, strip_off);                              // StripOffsets
    entry(277, 3, 1, 1);                                      // SamplesPerPixel
    entry(278, 4, 1, h);                                      // RowsPerStrip
    entry(279, 4, 1, strip_bytes);                            // StripByteCounts
    entry(284, 3, 1, 1);                                      // PlanarConfiguration
    entry(33421, 3, 2, pack_shorts(2, 2));                    // CFARepeatPatternDim
    entry(33422, 1, 4, cfa_off);                              // CFAPattern
    entry(50706, 1, 4, dng_ver_off);                          // DNGVersion
    entry(50714, 5, 1, black_rat_off);                        // BlackLevel (RATIONAL)
    entry(50717, 4, 1, white);                                // WhiteLevel

    out.extend_from_slice(&0u32.to_le_bytes()); // next IFD = 0

    // ── Tag value blobs ───────────────────────────────────────────────────────
    let write_blob = |out: &mut Vec<u8>, data: &[u8]| {
        out.extend_from_slice(data);
        if data.len() % 2 == 1 { out.push(0); }
    };
    write_blob(&mut out, &make_bytes);
    write_blob(&mut out, &model_bytes);
    write_blob(&mut out, &cfa_bytes);
    write_blob(&mut out, &dng_ver);
    write_blob(&mut out, &black_rational);

    // ── Image data ────────────────────────────────────────────────────────────
    for &pixel in &image.buffer.data {
        out.extend_from_slice(&pixel.to_le_bytes());
    }

    // Write all at once.
    let file = File::create(path).map_err(DualIsoError::Io)?;
    let mut writer = BufWriter::new(file);
    writer.write_all(&out).map_err(DualIsoError::Io)?;
    writer.flush().map_err(DualIsoError::Io)?;

    Ok(())
}

/// Pack two u16 into a single u32 LE value field (used for CFARepeatPatternDim).
#[inline]
fn pack_shorts(a: u16, b: u16) -> u32 {
    (a as u32) | ((b as u32) << 16)
}

