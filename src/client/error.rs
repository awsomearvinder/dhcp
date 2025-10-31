use std::{error::Error, fmt::Display};

#[derive(Debug)]
pub enum DhcpError {
    EncodeError(dhcproto::error::EncodeError),
}

impl From<dhcproto::error::EncodeError> for DhcpError {
    fn from(value: dhcproto::error::EncodeError) -> Self {
        Self::EncodeError(value)
    }
}

impl Error for DhcpError {}

impl Display for DhcpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DhcpError::EncodeError(encode_error) => write!(f, "{}", encode_error),
        }
    }
}
