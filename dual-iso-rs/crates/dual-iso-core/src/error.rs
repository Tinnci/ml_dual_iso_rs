use thiserror::Error;

#[derive(Debug, Error)]
pub enum DualIsoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Unsupported raw format: {0}")]
    UnsupportedFormat(String),

    #[error("Failed to decode raw file: {0}")]
    DecodeError(String),

    #[error("Pipeline error: {0}")]
    PipelineError(String),

    #[error("DNG output error: {0}")]
    DngOutputError(String),

    #[error("Not a dual-ISO image: cannot detect dual-ISO line pattern")]
    NotDualIso,

    #[error("Image too small for processing (need at least 8 rows)")]
    ImageTooSmall,
}
