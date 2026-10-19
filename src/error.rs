//! Error types for the GS-USB library
//!
//! This module defines the error types used throughout the library
//! for handling USB communication and protocol errors.

use thiserror::Error;

/// Result type alias for GS-USB operations
pub type Result<T> = std::result::Result<T, GsUsbError>;

/// Error types for GS-USB operations
#[derive(Error, Debug)]
pub enum GsUsbError {
    /// USB device or interface error from nusb
    #[error("USB error: {0}")]
    Usb(#[from] nusb::Error),

    /// No GS-USB device found
    #[error("No GS-USB device found")]
    DeviceNotFound,

    /// Device is not open
    #[error("Device is not open")]
    DeviceNotOpen,

    /// Failed to claim interface
    #[error("Failed to claim USB interface: {0}")]
    ClaimInterface(nusb::Error),

    /// Unsupported bitrate for the device clock
    #[error("Unsupported bitrate {bitrate} for clock {clock_hz} Hz")]
    UnsupportedBitrate { bitrate: u32, clock_hz: u32 },

    /// Unsupported data bitrate for CAN FD
    #[error("Unsupported data bitrate {bitrate} for clock {clock_hz} Hz")]
    UnsupportedDataBitrate { bitrate: u32, clock_hz: u32 },

    /// Device does not support CAN FD
    #[error("Device does not support CAN FD")]
    FdNotSupported,

    /// Device does not support the requested feature
    #[error("Device does not support feature: {0}")]
    FeatureNotSupported(&'static str),

    /// Timeout during read operation
    #[error("Read timeout")]
    ReadTimeout,

    /// Timeout during write operation
    #[error("Write timeout")]
    WriteTimeout,

    /// Invalid response from device
    #[error("Invalid response from device: expected {expected} bytes, got {actual}")]
    InvalidResponse { expected: usize, actual: usize },

    /// Control transfer failed
    #[error("Control transfer failed: {0}")]
    ControlTransfer(nusb::transfer::TransferError),

    /// Bulk transfer failed
    #[error("Bulk transfer failed: {0}")]
    BulkTransfer(nusb::transfer::TransferError),

    /// A USB write completed without transferring the whole frame
    #[error("Incomplete USB write: expected {expected} bytes, wrote {actual}")]
    IncompleteWrite { expected: usize, actual: usize },

    /// Invalid CAN frame or malformed wire data
    #[error("Invalid CAN frame: {0}")]
    InvalidFrame(&'static str),

    /// Requested mode flags cannot be enabled
    #[error("Unsupported mode flags: 0x{unsupported:08x}")]
    UnsupportedModeFlags { unsupported: u32 },

    /// Device is already started
    #[error("Device is already started")]
    AlreadyStarted,

    /// Device is not started
    #[error("Device is not started")]
    NotStarted,

    /// Invalid channel number
    #[error("Invalid channel number: {channel} (device has {max_channels} channels)")]
    InvalidChannel { channel: u8, max_channels: u8 },

    /// GET_STATE feature not supported
    #[error("Device does not support GET_STATE feature")]
    GetStateNotSupported,
}

impl GsUsbError {
    /// Check if this error is a timeout error
    pub fn is_timeout(&self) -> bool {
        matches!(
            self,
            GsUsbError::ReadTimeout
                | GsUsbError::WriteTimeout
                | GsUsbError::ControlTransfer(nusb::transfer::TransferError::Cancelled)
        )
    }

    /// Check if this error is a USB error
    pub fn is_usb_error(&self) -> bool {
        matches!(
            self,
            GsUsbError::Usb(_)
                | GsUsbError::ClaimInterface(_)
                | GsUsbError::ControlTransfer(_)
                | GsUsbError::BulkTransfer(_)
        )
    }
}
