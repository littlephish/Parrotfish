pub mod command;
pub mod fragment;
pub mod init;
pub mod packet;
pub mod voice;
pub mod window;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    TooShort(&'static str),
    InvalidInit(String),
    Decompress(String),
    TooLarge(&'static str),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::TooShort(what) => write!(f, "{what} is too short"),
            ProtocolError::InvalidInit(s) => write!(f, "invalid init packet: {s}"),
            ProtocolError::Decompress(s) => write!(f, "decompression failed: {s}"),
            ProtocolError::TooLarge(what) => write!(f, "{what} exceeds the size limit"),
        }
    }
}

impl std::error::Error for ProtocolError {}
