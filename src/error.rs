use thiserror::Error;

#[derive(Error, Debug)]
pub enum ArtisanError {
    #[error("{0}")]
    Generic(String),
    #[error("No such file: {0}")]
    NoSuchFile(String),
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Drive not found: {0}")]
    DriveNotFound(String),
    #[error("Drive is read-only")]
    ReadOnlyDrive,
    #[error("Drive too small: need {need} bytes, have {have} bytes")]
    DriveTooSmall { need: u64, have: u64 },
    #[error("Not enough space on drive")]
    NoSpace,
    #[error("Drive was unplugged during write")]
    DriveUnplugged,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image format not supported: {0}")]
    UnsupportedFormat(String),
    #[error("Flash cancelled")]
    Cancelled,
    #[error("Writer process died: {0}")]
    WriterDied(String),
    #[error("Unsupported protocol: {0}")]
    UnsupportedProtocol(String),
}

impl From<&str> for ArtisanError {
    fn from(s: &str) -> Self {
        ArtisanError::Generic(s.to_string())
    }
}

impl From<String> for ArtisanError {
    fn from(s: String) -> Self {
        ArtisanError::Generic(s)
    }
}
