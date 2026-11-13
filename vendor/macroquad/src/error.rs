#[derive(Debug)]
pub enum Error {
    FontError(&'static str),
    FileError {
        kind: miniquad::fs::Error,
        path: String,
    },
    ShaderError(miniquad::ShaderError),
    /// Last Route: the reason as text (it held the `image` crate's error; see `png_lite.rs`).
    ImageError(String),
    UnknownError(&'static str),
}

impl From<&'static str> for Error {
    fn from(s: &'static str) -> Self {
        Error::UnknownError(s)
    }
}

impl From<miniquad::ShaderError> for Error {
    fn from(s: miniquad::ShaderError) -> Self {
        Error::ShaderError(s)
    }
}


impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {self:?}")
    }
}

impl std::error::Error for Error {}
