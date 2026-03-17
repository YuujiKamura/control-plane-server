use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Windows API error: {0}")]
    Windows(#[from] windows::core::Error),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Invalid base64 payload")]
    InvalidBase64,

    #[error("Unknown command")]
    UnknownCommand,

    #[error("Failed to enqueue input")]
    EnqueueFailed,

    #[error("Dispatcher error")]
    DispatcherError,
}

pub type Result<T> = std::result::Result<T, Error>;
