use serde::Serialize;

/// Application error, serialized to the frontend as `{ code, message }`
/// so the UI can localize by `code` and show `message` as detail.
#[derive(Debug, thiserror::Error)]
pub enum SkimError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{message}")]
    Other { code: &'static str, message: String },
}

impl SkimError {
    pub fn other(code: &'static str, message: impl Into<String>) -> Self {
        Self::Other {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Db(_) => "db",
            Self::Join(_) => "internal",
            Self::Io(_) => "io",
            Self::Other { code, .. } => code,
        }
    }
}

#[derive(Serialize)]
struct ErrorPayload<'a> {
    code: &'a str,
    message: String,
}

impl Serialize for SkimError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        ErrorPayload {
            code: self.code(),
            message: self.to_string(),
        }
        .serialize(serializer)
    }
}

pub type Result<T> = std::result::Result<T, SkimError>;
