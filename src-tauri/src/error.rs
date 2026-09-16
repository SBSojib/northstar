use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Secure storage error: {0}")]
    Keyring(#[from] keyring::Error),
    #[error("Encryption failed")]
    Encryption,
    #[error("Invalid encrypted data")]
    InvalidEncryptedData,
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SSH error: {0}")]
    Ssh(#[from] ssh2::Error),
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Item not found")]
    NotFound,
    #[error("Session not found")]
    SessionNotFound,
    #[error("Internal error: {0}")]
    Internal(String),
}

impl serde::Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
