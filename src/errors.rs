use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Download,
    Parse,
    Render,
    Other,
}

#[derive(Error, Debug, Clone)]
pub enum PreviewError {
    #[error("download error: {0}")]
    Download(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("render error: {0}")]
    Render(String),
    #[error("{0}")]
    Other(String),
}

impl PreviewError {
    pub fn kind(&self) -> ErrorKind {
        match self {
            PreviewError::Download(_) => ErrorKind::Download,
            PreviewError::Parse(_) => ErrorKind::Parse,
            PreviewError::Render(_) => ErrorKind::Render,
            PreviewError::Other(_) => ErrorKind::Other,
        }
    }

    pub fn new(msg: impl Into<String>) -> Self {
        PreviewError::Other(msg.into())
    }

    pub fn download(msg: impl Into<String>) -> Self {
        PreviewError::Download(msg.into())
    }

    pub fn parse(msg: impl Into<String>) -> Self {
        PreviewError::Parse(msg.into())
    }

    pub fn render(msg: impl Into<String>) -> Self {
        PreviewError::Render(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, PreviewError>;
