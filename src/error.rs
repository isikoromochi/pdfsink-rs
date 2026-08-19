use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PDF parse error: {0}")]
    Lopdf(#[from] lopdf::Error),

    #[error("pdf-extract error: {0}")]
    PdfExtract(String),

    #[error("regex error: {0}")]
    Regex(#[from] regex::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("invalid page number: {page_number}")]
    InvalidPage { page_number: usize },

    #[error("invalid bounding box: {0}")]
    InvalidBBox(String),

    #[error("type mismatch: {0}")]
    Type(String),

    #[error("unsupported PDF feature: {0}")]
    Unsupported(String),

    #[error("{0}")]
    Message(String),
}

impl From<pdf_extract::PdfExtractError> for Error {
    fn from(value: pdf_extract::PdfExtractError) -> Self {
        Self::PdfExtract(value.to_string())
    }
}
