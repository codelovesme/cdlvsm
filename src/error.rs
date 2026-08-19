//! A single flat error type. The whole CLI's contract is "one clean line to
//! stderr, exit 1" — never a panic/backtrace — so every fallible path returns
//! this and `main` prints it and exits 1.

pub struct CdlvsmError(pub String);

pub type Result<T> = std::result::Result<T, CdlvsmError>;

pub fn fail<T>(msg: impl Into<String>) -> Result<T> {
    Err(CdlvsmError(msg.into()))
}
