use std::fmt;

#[derive(Debug, Clone)]
pub struct PreviewError(pub String);

impl PreviewError {
    pub fn new(msg: impl Into<String>) -> Self {
        PreviewError(msg.into())
    }
}

impl fmt::Display for PreviewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for PreviewError {}

pub type Result<T> = std::result::Result<T, PreviewError>;
