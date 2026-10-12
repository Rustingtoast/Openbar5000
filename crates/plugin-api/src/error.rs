use std::fmt;

#[derive(Debug)]
pub enum DecodeError {
    UnsupportedFormat,
    IoError(String),
    CorruptStream(String),
    EndOfStream,
    SeekUnsupported,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat => write!(f, "unsupported audio format"),
            Self::IoError(msg) => write!(f, "io error: {msg}"),
            Self::CorruptStream(msg) => write!(f, "corrupt stream: {msg}"),
            Self::EndOfStream => write!(f, "end of stream"),
            Self::SeekUnsupported => write!(f, "seeking is not supported by this decoder"),
        }
    }
}

impl std::error::Error for DecodeError {}

#[derive(Debug)]
pub enum OutputError {
    DeviceUnavailable(String),
    UnsupportedConfig { sample_rate: u32, channels: u16 },
    StreamError(String),
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceUnavailable(msg) => write!(f, "output device unavailable: {msg}"),
            Self::UnsupportedConfig {
                sample_rate,
                channels,
            } => {
                write!(
                    f,
                    "unsupported output config: {sample_rate}Hz x{channels}ch"
                )
            }
            Self::StreamError(msg) => write!(f, "output stream error: {msg}"),
        }
    }
}

impl std::error::Error for OutputError {}
