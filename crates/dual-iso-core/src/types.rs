use serde::{Deserialize, Serialize};

/// EV precision: 65536 steps per stop (same as original C code).
pub const EV_RESOLUTION: i32 = 65536;

// ─── Bayer pattern ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BayerPattern {
    #[default]
    Rggb,
    Gbrg,
    Bggr,
    Grbg,
}

impl BayerPattern {
    /// Return (r_off, g1_off, g2_off, b_off) as (col,row) offsets within 2×2 block
    pub fn offsets(self) -> [(usize, usize); 4] {
        // (col, row) for [R, G1, G2, B]
        match self {
            Self::Rggb => [(0, 0), (1, 0), (0, 1), (1, 1)],
            Self::Gbrg => [(1, 0), (0, 0), (1, 1), (0, 1)],
            Self::Bggr => [(1, 1), (0, 1), (1, 0), (0, 0)],
            Self::Grbg => [(0, 0), (1, 0), (0, 1), (1, 1)], // same layout as RGGB channel-wise
        }
    }

    /// CFA byte pattern [top-left, top-right, bottom-left, bottom-right]
    /// using TIFF/DNG codes: 0=R, 1=G, 2=B
    pub fn cfa_bytes(self) -> [u8; 4] {
        match self {
            Self::Rggb => [0, 1, 1, 2],
            Self::Gbrg => [1, 2, 0, 1],
            Self::Bggr => [2, 1, 1, 0],
            Self::Grbg => [1, 0, 2, 1],
        }
    }
}

// ─── ISO line pattern ───────────────────────────────────────────────────────

/// Describes which rows (by `y % 4`) carry the bright (high-ISO) exposure.
#[derive(Debug, Clone)]
pub struct IsoLinePattern {
    /// `is_bright[y % 4]` – true ⟹ row `y` is high-ISO ("bright")
    pub is_bright: [bool; 4],
    /// Nominal low-ISO value (e.g. 100)
    pub iso_lowlight: u32,
    /// Nominal high-ISO value (e.g. 1600)
    pub iso_highlight: u32,
}

impl IsoLinePattern {
    pub fn bright_fraction(&self) -> f64 {
        self.is_bright.iter().filter(|&&b| b).count() as f64 / 4.0
    }
}

// ─── Raw pixel buffer ───────────────────────────────────────────────────────

/// A 16-bit Bayer pixel buffer (row-major).
#[derive(Debug, Clone)]
pub struct RawBuffer {
    pub data: Vec<u16>,
    pub width: usize,
    pub height: usize,
}

impl RawBuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Self { data: vec![0u16; width * height], width, height }
    }

    #[inline]
    pub fn pixel(&self, x: usize, y: usize) -> u16 {
        self.data[y * self.width + x]
    }

    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, v: u16) {
        self.data[y * self.width + x] = v;
    }

    /// Safely read a pixel, clamping out-of-bounds coordinates.
    #[inline]
    pub fn pixel_clamped(&self, x: i64, y: i64) -> u16 {
        let x = x.clamp(0, self.width as i64 - 1) as usize;
        let y = y.clamp(0, self.height as i64 - 1) as usize;
        self.data[y * self.width + x]
    }
}

// ─── Raw image metadata ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct RawMetadata {
    pub black_level: u16,
    pub white_level: u16,
    pub bayer_pattern: BayerPattern,
    /// Colour temperature in Kelvin (from EXIF, if available).
    pub color_temperature: Option<f64>,
    /// Per-channel pre-multipliers [R, G, B].
    pub pre_mul: [f32; 3],
    /// Camera-to-sRGB matrix, row-major 3×4 (4th column = unused).
    pub rgb_cam: [[f32; 4]; 3],
    pub camera_make: String,
    pub camera_model: String,
    pub bits_per_pixel: u8,
    /// Original EXIF blob (embedded verbatim in output DNG).
    pub exif_blob: Vec<u8>,
}

impl Default for RawMetadata {
    fn default() -> Self {
        Self {
            black_level: 2048,
            white_level: 15000,
            bayer_pattern: BayerPattern::Rggb,
            color_temperature: Some(5500.0),
            pre_mul: [1.0, 1.0, 1.0],
            rgb_cam: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            camera_make: String::new(),
            camera_model: String::new(),
            bits_per_pixel: 14,
            exif_blob: Vec::new(),
        }
    }
}

/// A full raw image: pixel data + metadata.
#[derive(Debug, Clone)]
pub struct RawImage {
    pub buffer: RawBuffer,
    pub meta: RawMetadata,
}

// ─── Processing configuration ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum InterpolationMethod {
    /// AMaZE + edge-directed interpolation (high quality, slower).
    #[default]
    AmazeEdge,
    /// Average of nearest 2–3 same-colour Bayer pixels (fast).
    Mean23,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ChromaSmoothSize {
    /// 2×2 neighbourhood (default).
    #[default]
    TwoByTwo,
    ThreeByThree,
    FiveByFive,
    None,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum WhiteBalance {
    /// Maximise gray pixels (default).
    #[default]
    GrayMax,
    /// Median of R−G and B−G differences.
    GrayMedian,
    /// Use EXIF white balance.
    Exif,
    /// Custom RGB multipliers.
    Custom(f32, f32, f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BadPixelFix {
    #[default]
    Normal,
    Aggressive,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Compression {
    #[default]
    None,
    Lossless,
    Lossy,
}

/// All processing knobs — mirrors the cr2hdr command-line options.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessConfig {
    pub interp_method: InterpolationMethod,
    pub chroma_smooth: ChromaSmoothSize,
    pub bad_pixels: BadPixelFix,
    pub mark_bad_pixels_black: bool,
    pub white_balance: WhiteBalance,
    pub use_fullres: bool,
    pub use_alias_map: bool,
    pub use_stripe_fix: bool,
    /// Soft-film curve EV lift (0 = disabled).
    pub soft_film_ev: f32,
    pub compression: Compression,
    pub same_levels: bool,
    pub skip_existing: bool,
    pub embed_original: bool,
    pub embed_original_copy: bool,
    // Debug / diagnostic flags
    pub debug_blend: bool,
    pub debug_black: bool,
    pub debug_amaze: bool,
    pub debug_edge: bool,
    pub debug_alias: bool,
    pub debug_bad_pixels: bool,
    pub debug_wb: bool,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            interp_method: InterpolationMethod::AmazeEdge,
            chroma_smooth: ChromaSmoothSize::TwoByTwo,
            bad_pixels: BadPixelFix::Normal,
            mark_bad_pixels_black: false,
            white_balance: WhiteBalance::GrayMax,
            use_fullres: true,
            use_alias_map: true,
            use_stripe_fix: true,
            soft_film_ev: 0.0,
            compression: Compression::None,
            same_levels: false,
            skip_existing: false,
            embed_original: false,
            embed_original_copy: false,
            debug_blend: false,
            debug_black: false,
            debug_amaze: false,
            debug_edge: false,
            debug_alias: false,
            debug_bad_pixels: false,
            debug_wb: false,
        }
    }
}

impl ProcessConfig {
    /// Equivalent to `--fast`: disable most post-processing steps.
    pub fn fast() -> Self {
        Self {
            interp_method: InterpolationMethod::Mean23,
            chroma_smooth: ChromaSmoothSize::None,
            bad_pixels: BadPixelFix::Disabled,
            use_fullres: false,
            use_alias_map: false,
            use_stripe_fix: false,
            ..Default::default()
        }
    }
}
