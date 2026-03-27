use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use rayon::prelude::*;

use dual_iso_core::{
    BadPixelFix, ChromaSmoothSize, Compression, InterpolationMethod, ProcessConfig, WhiteBalance,
    dng_output::write_dng, raw_io::read_raw,
};

// ─── CLI definition ──────────────────────────────────────────────────────────

#[derive(Debug, Parser)]
#[command(
    name = "cr2hdr",
    version,
    about = "Post-process dual-ISO CR2/DNG files into clean 16-bit HDR DNG images.\n\
             Drag-and-drop usage: drop CR2/DNG files onto cr2hdr to convert with defaults.",
    long_about = None,
)]
struct Cli {
    /// Input CR2 or DNG files (one or more).
    #[arg(required = true, value_name = "FILE")]
    files: Vec<PathBuf>,

    // ── Shortcuts ──────────────────────────────────────────────────────────
    /// Disable most post-processing steps (fast, but lower quality).
    /// Equivalent to --mean23 --no-cs --no-fullres --no-alias-map --no-stripe-fix --no-bad-pix.
    #[arg(long, conflicts_with = "interp")]
    fast: bool,

    // ── Interpolation ──────────────────────────────────────────────────────
    /// Interpolation method.
    #[arg(long = "interp", value_enum, default_value = "amaze-edge")]
    interp: InterpArg,

    // ── Chroma smoothing ───────────────────────────────────────────────────
    /// Chroma smoothing filter size.
    #[arg(long = "cs", value_enum, default_value = "2x2")]
    chroma_smooth: CsArg,

    // ── Bad pixels ─────────────────────────────────────────────────────────
    /// Bad pixel fix aggressiveness.
    #[arg(long = "bad-pix", value_enum, default_value = "normal")]
    bad_pix: BadPixArg,

    /// Mark bad pixels as black (for troubleshooting).
    #[arg(long)]
    black_bad_pix: bool,

    // ── White balance ──────────────────────────────────────────────────────
    /// White balance mode.
    #[arg(long = "wb", value_enum, default_value = "graymax")]
    wb: WbArg,

    /// Custom RGB white-balance multipliers (used when --wb=custom).
    #[arg(long, num_args = 3, value_names = ["R", "G", "B"])]
    wb_custom: Option<Vec<f32>>,

    // ── Post-processing toggles ────────────────────────────────────────────
    /// Disable full-resolution blending.
    #[arg(long)]
    no_fullres: bool,

    /// Disable alias map (alias fix in deep shadows).
    #[arg(long)]
    no_alias_map: bool,

    /// Disable horizontal stripe fix.
    #[arg(long)]
    no_stripe_fix: bool,

    // ── Highlight/shadow ───────────────────────────────────────────────────
    /// Apply a soft-film curve to compress highlights and raise shadows by X EV.
    #[arg(long, value_name = "EV")]
    soft_film: Option<f32>,

    // ── Output ────────────────────────────────────────────────────────────
    /// DNG compression.
    #[arg(long = "compress", value_enum, default_value = "none")]
    compression: CompressArg,

    /// Skip conversion if the output DNG already exists.
    #[arg(long)]
    skip_existing: bool,

    /// Embed (move) the original CR2 inside the output DNG.
    #[arg(long)]
    embed_original: bool,

    /// Same as --embed-original but keeps the original file.
    #[arg(long)]
    embed_original_copy: bool,

    /// Equalise output white levels across all frames (flicker prevention).
    #[arg(long)]
    same_levels: bool,

    // ── Debug ─────────────────────────────────────────────────────────────
    /// Save intermediate blend images.
    #[arg(long)]
    debug_blend: bool,

    /// Save intermediate black-subtraction images.
    #[arg(long)]
    debug_black: bool,

    /// Save AMaZE input/output images.
    #[arg(long)]
    debug_amaze: bool,

    /// Save edge-direction debug images.
    #[arg(long)]
    debug_edge: bool,

    /// Save alias-map debug images.
    #[arg(long)]
    debug_alias: bool,

    /// Mark bad pixels black in output (debug).
    #[arg(long)]
    debug_bad_pix: bool,

    /// Show white-balance vectorscope.
    #[arg(long)]
    debug_wb: bool,

    // ── Verbosity ─────────────────────────────────────────────────────────
    /// Verbose output (repeat for more detail: -v, -vv, -vvv).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

// ─── Argument enums ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum InterpArg {
    #[value(name = "amaze-edge")]
    AmazeEdge,
    #[value(name = "mean23")]
    Mean23,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CsArg {
    #[value(name = "2x2")]  TwoByTwo,
    #[value(name = "3x3")]  ThreeByThree,
    #[value(name = "5x5")]  FiveByFive,
    #[value(name = "none")] None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BadPixArg {
    Normal, Aggressive, Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum WbArg {
    Graymax, Graymed, Exif, Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CompressArg {
    None, Lossless, Lossy,
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up tracing.
    let level = match cli.verbose {
        0 => tracing::Level::WARN,
        1 => tracing::Level::INFO,
        2 => tracing::Level::DEBUG,
        _ => tracing::Level::TRACE,
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(false)
        .init();

    let config = build_config(&cli);

    // Process files in parallel (rayon).
    let results: Vec<Result<PathBuf>> = cli.files
        .par_iter()
        .map(|path| process_file(path, &config))
        .collect();

    let mut had_error = false;
    for r in results {
        match r {
            Ok(out) => println!("  -> {}", out.display()),
            Err(e) => { eprintln!("Error: {e:#}"); had_error = true; }
        }
    }

    if had_error { std::process::exit(1); }
    Ok(())
}

// ─── Per-file processing ─────────────────────────────────────────────────────

fn process_file(input: &Path, config: &ProcessConfig) -> Result<PathBuf> {
    let out_path = derive_output_path(input)?;

    if config.skip_existing && out_path.exists() {
        tracing::info!("{} already exists, skipping", out_path.display());
        return Ok(out_path);
    }

    tracing::info!("reading {}", input.display());
    let raw = read_raw(input)
        .with_context(|| format!("reading {}", input.display()))?;

    tracing::info!("processing");
    let processed = dual_iso_core::process(raw, config)
        .with_context(|| format!("processing {}", input.display()))?;

    tracing::info!("writing {}", out_path.display());
    write_dng(&out_path, &processed, config)
        .with_context(|| format!("writing {}", out_path.display()))?;

    Ok(out_path)
}

fn derive_output_path(input: &Path) -> Result<PathBuf> {
    let stem = input.file_stem()
        .and_then(|s| s.to_str())
        .context("input file has no stem")?;
    let dir = input.parent().unwrap_or(Path::new("."));
    Ok(dir.join(format!("{stem}.DNG")))
}

// ─── Config builder ──────────────────────────────────────────────────────────

fn build_config(cli: &Cli) -> ProcessConfig {
    if cli.fast {
        return ProcessConfig::fast();
    }

    let interp_method = match cli.interp {
        InterpArg::AmazeEdge => InterpolationMethod::AmazeEdge,
        InterpArg::Mean23    => InterpolationMethod::Mean23,
    };

    let chroma_smooth = match cli.chroma_smooth {
        CsArg::TwoByTwo       => ChromaSmoothSize::TwoByTwo,
        CsArg::ThreeByThree   => ChromaSmoothSize::ThreeByThree,
        CsArg::FiveByFive     => ChromaSmoothSize::FiveByFive,
        CsArg::None           => ChromaSmoothSize::None,
    };

    let bad_pixels = match cli.bad_pix {
        BadPixArg::Normal     => BadPixelFix::Normal,
        BadPixArg::Aggressive => BadPixelFix::Aggressive,
        BadPixArg::Disabled   => BadPixelFix::Disabled,
    };

    let white_balance = match cli.wb {
        WbArg::Graymax => WhiteBalance::GrayMax,
        WbArg::Graymed => WhiteBalance::GrayMedian,
        WbArg::Exif    => WhiteBalance::Exif,
        WbArg::Custom  => {
            let v = cli.wb_custom.as_deref().unwrap_or(&[1.0, 1.0, 1.0]);
            WhiteBalance::Custom(
                *v.first().unwrap_or(&1.0),
                *v.get(1).unwrap_or(&1.0),
                *v.get(2).unwrap_or(&1.0),
            )
        }
    };

    let compression = match cli.compression {
        CompressArg::None     => Compression::None,
        CompressArg::Lossless => Compression::Lossless,
        CompressArg::Lossy    => Compression::Lossy,
    };

    ProcessConfig {
        interp_method,
        chroma_smooth,
        bad_pixels,
        mark_bad_pixels_black: cli.black_bad_pix || cli.debug_bad_pix,
        white_balance,
        use_fullres:   !cli.no_fullres,
        use_alias_map: !cli.no_alias_map,
        use_stripe_fix: !cli.no_stripe_fix,
        soft_film_ev:  cli.soft_film.unwrap_or(0.0),
        compression,
        same_levels:   cli.same_levels,
        skip_existing: cli.skip_existing,
        embed_original: cli.embed_original,
        embed_original_copy: cli.embed_original_copy,
        debug_blend:  cli.debug_blend,
        debug_black:  cli.debug_black,
        debug_amaze:  cli.debug_amaze,
        debug_edge:   cli.debug_edge,
        debug_alias:  cli.debug_alias,
        debug_bad_pixels: cli.debug_bad_pix,
        debug_wb:     cli.debug_wb,
    }
}
