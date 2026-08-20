use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Msg(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("image: {0}")]
    Image(#[from] image::ImageError),
}

impl Error {
    pub fn msg(m: impl Into<String>) -> Self {
        Self::Msg(m.into())
    }

    pub fn is_wayland_connect(&self) -> bool {
        match self {
            Self::Msg(m) => {
                let l = m.to_ascii_lowercase();
                l.contains("wayland") || l.contains("xdg_runtime") || l.contains("connect")
            }
            Self::Io(e) => e.kind() == std::io::ErrorKind::ConnectionRefused
                || e.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
