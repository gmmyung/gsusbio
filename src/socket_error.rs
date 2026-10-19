//! Errors returned by the socket API.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("USB error: {0}")]
    Usb(#[from] nusb::Error),
    #[error("USB transfer error: {0}")]
    Transfer(#[from] nusb::transfer::TransferError),
    #[error("GS-USB operation failed: {0}")]
    DeviceOperation(String),
    #[error("no GS-USB device found")]
    DeviceNotFound,
    #[error("device disconnected")]
    Disconnected,
    #[error("operation timed out")]
    Timeout,
    #[error("unsupported feature: {0}")]
    UnsupportedFeature(&'static str),
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("invalid CAN frame: {0}")]
    InvalidFrame(&'static str),
    #[error("invalid GS-USB message: {0}")]
    Protocol(&'static str),
    #[error("I/O queue closed")]
    QueueClosed,
    #[error("incomplete USB write: expected {expected} bytes, wrote {actual}")]
    IncompleteWrite { expected: usize, actual: usize },
}

impl Error {
    pub(crate) fn from_transfer(value: nusb::transfer::TransferError) -> Self {
        match value {
            nusb::transfer::TransferError::Disconnected => Self::Disconnected,
            other => Self::Transfer(other),
        }
    }
}

impl From<crate::error::GsUsbError> for Error {
    fn from(value: crate::error::GsUsbError) -> Self {
        use crate::error::GsUsbError;
        match value {
            GsUsbError::Usb(error) | GsUsbError::ClaimInterface(error) => Self::Usb(error),
            GsUsbError::ControlTransfer(error) | GsUsbError::BulkTransfer(error) => {
                Self::from_transfer(error)
            }
            GsUsbError::ReadTimeout | GsUsbError::WriteTimeout => Self::Timeout,
            GsUsbError::IncompleteWrite { expected, actual } => {
                Self::IncompleteWrite { expected, actual }
            }
            GsUsbError::InvalidFrame(message) => Self::InvalidFrame(message),
            GsUsbError::FdNotSupported => Self::UnsupportedFeature("CAN FD"),
            other => Self::DeviceOperation(other.to_string()),
        }
    }
}
