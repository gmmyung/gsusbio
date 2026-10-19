//! GS-USB CAN frame implementation
//!
//! This module provides the `GsUsbFrame` struct for representing CAN frames
//! in the GS-USB protocol, including support for both classic CAN and CAN FD.

use crate::constants::{
    CANFD_DLC_TO_LEN, CANFD_MAX_DLEN, CAN_EFF_FLAG, CAN_EFF_MASK, CAN_ERR_FLAG, CAN_MAX_DLEN,
    CAN_RTR_FLAG, GS_CAN_FLAG_BRS, GS_CAN_FLAG_FD, GS_USB_ECHO_ID, GS_USB_FRAME_SIZE,
    GS_USB_FRAME_SIZE_FD, GS_USB_FRAME_SIZE_FD_HW_TIMESTAMP, GS_USB_FRAME_SIZE_HW_TIMESTAMP,
    GS_USB_RX_ECHO_ID,
};
use crate::error::{GsUsbError, Result};

/// Convert DLC to data length
pub fn dlc_to_len(dlc: u8, fd: bool) -> usize {
    if fd {
        if (dlc as usize) < CANFD_DLC_TO_LEN.len() {
            CANFD_DLC_TO_LEN[dlc as usize]
        } else {
            CANFD_MAX_DLEN
        }
    } else {
        (dlc as usize).min(CAN_MAX_DLEN)
    }
}

/// Convert data length to DLC
pub fn len_to_dlc(length: usize, fd: bool) -> u8 {
    if fd {
        for (dlc, &dlen) in CANFD_DLC_TO_LEN.iter().enumerate() {
            if dlen >= length {
                return dlc as u8;
            }
        }
        15 // Max DLC for CAN FD
    } else {
        length.min(CAN_MAX_DLEN) as u8
    }
}

/// GS-USB CAN frame
///
/// Represents a CAN frame in the GS-USB protocol format.
/// Supports both classic CAN (8 bytes max) and CAN FD (64 bytes max).
#[derive(Clone)]
pub struct GsUsbFrame {
    /// Echo ID (0 for TX, 0xFFFFFFFF for RX)
    pub echo_id: u32,
    /// CAN identifier (with flags like CAN_EFF_FLAG if needed)
    pub can_id: u32,
    /// Data length code
    pub can_dlc: u8,
    /// CAN channel
    pub channel: u8,
    /// Frame flags (FD, BRS, etc.)
    pub flags: u8,
    /// Reserved byte
    pub reserved: u8,
    /// Frame data (up to 64 bytes for CAN FD)
    pub data: [u8; CANFD_MAX_DLEN],
    /// Hardware timestamp in microseconds
    pub timestamp_us: u32,
}

impl Default for GsUsbFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl GsUsbFrame {
    /// Create a new empty CAN frame
    pub fn new() -> Self {
        Self {
            echo_id: GS_USB_ECHO_ID,
            can_id: 0,
            can_dlc: 0,
            channel: 0,
            flags: 0,
            reserved: 0,
            data: [0u8; CANFD_MAX_DLEN],
            timestamp_us: 0,
        }
    }

    /// Create a new CAN frame with the specified ID and data
    ///
    /// # Arguments
    /// * `can_id` - CAN identifier (with flags like CAN_EFF_FLAG if needed)
    /// * `data` - Frame data (up to 8 bytes for classic CAN, 64 for CAN FD)
    pub fn with_data(can_id: u32, data: &[u8]) -> Self {
        let mut frame = Self::new();
        frame.can_id = can_id;
        frame.set_data(data, false);
        frame
    }

    /// Create a new CAN FD frame with the specified ID and data
    ///
    /// # Arguments
    /// * `can_id` - CAN identifier (with flags like CAN_EFF_FLAG if needed)
    /// * `data` - Frame data (up to 64 bytes)
    /// * `brs` - Enable bit rate switch (transmit data at higher rate)
    pub fn with_fd_data(can_id: u32, data: &[u8], brs: bool) -> Self {
        let mut frame = Self::new();
        frame.can_id = can_id;
        frame.flags |= GS_CAN_FLAG_FD;
        if brs {
            frame.flags |= GS_CAN_FLAG_BRS;
        }
        frame.set_data(data, true);
        frame
    }

    /// Set frame data
    fn set_data(&mut self, data: &[u8], fd: bool) {
        let max_len = if fd { CANFD_MAX_DLEN } else { CAN_MAX_DLEN };
        let data_len = data.len().min(max_len);

        // Clear data array and copy new data
        self.data = [0u8; CANFD_MAX_DLEN];
        self.data[..data_len].copy_from_slice(&data[..data_len]);
        self.can_dlc = len_to_dlc(data.len(), fd);
    }

    /// Get the arbitration ID (without flags)
    pub fn arbitration_id(&self) -> u32 {
        self.can_id & CAN_EFF_MASK
    }

    /// Check if this is an extended ID frame (29-bit)
    pub fn is_extended_id(&self) -> bool {
        (self.can_id & CAN_EFF_FLAG) != 0
    }

    /// Check if this is a remote transmission request
    pub fn is_remote_frame(&self) -> bool {
        (self.can_id & CAN_RTR_FLAG) != 0
    }

    /// Check if this is an error frame
    pub fn is_error_frame(&self) -> bool {
        (self.can_id & CAN_ERR_FLAG) != 0
    }

    /// Check if this is a CAN FD frame
    pub fn is_fd(&self) -> bool {
        (self.flags & GS_CAN_FLAG_FD) != 0
    }

    /// Check if bit rate switch is enabled
    pub fn is_brs(&self) -> bool {
        (self.flags & GS_CAN_FLAG_BRS) != 0
    }

    /// Check if this is an echo frame (TX confirmation from device)
    pub fn is_echo_frame(&self) -> bool {
        self.echo_id != GS_USB_RX_ECHO_ID
    }

    /// Check if this is a received frame (from CAN bus)
    pub fn is_rx_frame(&self) -> bool {
        self.echo_id == GS_USB_RX_ECHO_ID
    }

    /// Get timestamp in seconds
    pub fn timestamp(&self) -> f64 {
        self.timestamp_us as f64 / 1_000_000.0
    }

    /// Get actual data length based on DLC and frame type
    pub fn data_length(&self) -> usize {
        dlc_to_len(self.can_dlc, self.is_fd())
    }

    /// Get frame data as a slice
    pub fn data(&self) -> &[u8] {
        &self.data[..self.data_length()]
    }

    /// Get frame size in bytes
    pub fn frame_size(hw_timestamp: bool, fd_mode: bool) -> usize {
        match (fd_mode, hw_timestamp) {
            (true, true) => GS_USB_FRAME_SIZE_FD_HW_TIMESTAMP,
            (true, false) => GS_USB_FRAME_SIZE_FD,
            (false, true) => GS_USB_FRAME_SIZE_HW_TIMESTAMP,
            (false, false) => GS_USB_FRAME_SIZE,
        }
    }

    /// Pack frame into bytes for transmission
    ///
    /// # Arguments
    /// * `hw_timestamp` - Include timestamp field
    /// * `fd_mode` - Use CAN FD frame format (64-byte data)
    pub fn pack(&self, hw_timestamp: bool, fd_mode: bool) -> Vec<u8> {
        let data_len = if fd_mode { 64 } else { 8 };
        let size = Self::frame_size(hw_timestamp, fd_mode);
        let mut buf = Vec::with_capacity(size);

        // Header: echo_id (4) + can_id (4) + can_dlc (1) + channel (1) + flags (1) + reserved (1)
        buf.extend_from_slice(&self.echo_id.to_le_bytes());
        buf.extend_from_slice(&self.can_id.to_le_bytes());
        buf.push(self.can_dlc);
        buf.push(self.channel);
        buf.push(self.flags);
        buf.push(self.reserved);

        // Data
        buf.extend_from_slice(&self.data[..data_len]);

        // Optional timestamp
        if hw_timestamp {
            buf.extend_from_slice(&self.timestamp_us.to_le_bytes());
        }

        buf
    }

    /// Encode a frame for host-to-device transfer. Timestamps are generated
    /// by the adapter, so outgoing frames never contain a timestamp field.
    pub(crate) fn pack_for_tx(&self, fd_mode: bool) -> Result<Vec<u8>> {
        if self.is_fd() && !fd_mode {
            return Err(GsUsbError::FdNotSupported);
        }
        if self.is_brs() && !self.is_fd() {
            return Err(GsUsbError::InvalidFrame("BRS requires a CAN FD frame"));
        }
        if self.is_fd() && self.can_dlc > 15 {
            return Err(GsUsbError::InvalidFrame("CAN FD DLC exceeds 15"));
        }
        if !self.is_fd() && self.can_dlc > 8 {
            return Err(GsUsbError::InvalidFrame("classic CAN DLC exceeds 8"));
        }
        Ok(self.pack(false, fd_mode))
    }

    /// Unpack received bytes into this frame
    ///
    /// # Arguments
    /// * `data` - Raw bytes received from device
    /// * `hw_timestamp` - Data includes timestamp field
    /// * `fd_mode` - CAN FD frame format (64-byte data)
    ///
    /// Returns an error if the frame is shorter than its header, declared
    /// payload, or timestamp field.
    pub fn unpack_from(&mut self, data: &[u8], hw_timestamp: bool, fd_mode: bool) -> Result<()> {
        if data.len() < 12 {
            return Err(GsUsbError::InvalidResponse {
                expected: 12,
                actual: data.len(),
            });
        }

        let wire_fd = data[10] & GS_CAN_FLAG_FD != 0;
        if wire_fd != fd_mode {
            return Err(GsUsbError::InvalidFrame(
                "FD flag does not match frame format",
            ));
        }
        if fd_mode && data[8] > 15 {
            return Err(GsUsbError::InvalidFrame("CAN FD DLC exceeds 15"));
        }

        let data_len = if fd_mode { 64 } else { 8 };
        let echo_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let can_id = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let required_data =
            if echo_id != GS_USB_RX_ECHO_ID || (!fd_mode && can_id & CAN_RTR_FLAG != 0) {
                0
            } else {
                dlc_to_len(data[8], fd_mode)
            };
        // Timestamps follow the full 8- or 64-byte data field, even for
        // short payloads and TX echo frames.
        let expected = if hw_timestamp {
            12 + data_len + 4
        } else {
            12 + required_data
        };
        if data.len() < expected {
            return Err(GsUsbError::InvalidResponse {
                expected,
                actual: data.len(),
            });
        }

        // Header
        self.echo_id = echo_id;
        self.can_id = can_id;
        self.can_dlc = data[8];
        self.channel = data[9];
        self.flags = data[10];
        self.reserved = data[11];

        // Data
        self.data = [0u8; CANFD_MAX_DLEN];
        let copy_len = data_len.min(data.len() - 12);
        self.data[..copy_len].copy_from_slice(&data[12..12 + copy_len]);

        // Timestamp
        if hw_timestamp {
            let ts_offset = 12 + data_len;
            self.timestamp_us = u32::from_le_bytes([
                data[ts_offset],
                data[ts_offset + 1],
                data[ts_offset + 2],
                data[ts_offset + 3],
            ]);
        } else {
            self.timestamp_us = 0;
        }
        Ok(())
    }

    /// Create a new frame from received bytes
    ///
    /// # Arguments
    /// * `data` - Raw bytes received from device
    /// * `hw_timestamp` - Data includes timestamp field
    /// * `fd_mode` - CAN FD frame format (64-byte data)
    ///
    /// Returns an error for malformed or truncated frames.
    pub fn from_bytes(data: &[u8], hw_timestamp: bool, fd_mode: bool) -> Result<Self> {
        let mut frame = Self::new();
        frame.unpack_from(data, hw_timestamp, fd_mode)?;
        Ok(frame)
    }
}

impl std::fmt::Display for GsUsbFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let fd_indicator = if self.is_fd() { " FD" } else { "" };
        let brs_indicator = if self.is_brs() { " BRS" } else { "" };

        let data_str = if self.is_remote_frame() {
            "remote request".to_string()
        } else {
            self.data()
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<_>>()
                .join(" ")
        };

        write!(
            f,
            "{:>8X}{}{}   [{}]  {}",
            self.arbitration_id(),
            fd_indicator,
            brs_indicator,
            self.data_length(),
            data_str
        )
    }
}

impl std::fmt::Debug for GsUsbFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GsUsbFrame")
            .field("echo_id", &format_args!("0x{:08X}", self.echo_id))
            .field("can_id", &format_args!("0x{:08X}", self.can_id))
            .field("can_dlc", &self.can_dlc)
            .field("channel", &self.channel)
            .field("flags", &format_args!("0x{:02X}", self.flags))
            .field("data_length", &self.data_length())
            .field("is_fd", &self.is_fd())
            .field("is_echo", &self.is_echo_frame())
            .field("timestamp_us", &self.timestamp_us)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dlc_to_len_classic() {
        assert_eq!(dlc_to_len(0, false), 0);
        assert_eq!(dlc_to_len(8, false), 8);
        assert_eq!(dlc_to_len(15, false), 8); // Clamped to 8
    }

    #[test]
    fn test_dlc_to_len_fd() {
        assert_eq!(dlc_to_len(0, true), 0);
        assert_eq!(dlc_to_len(8, true), 8);
        assert_eq!(dlc_to_len(9, true), 12);
        assert_eq!(dlc_to_len(15, true), 64);
    }

    #[test]
    fn test_len_to_dlc_classic() {
        assert_eq!(len_to_dlc(0, false), 0);
        assert_eq!(len_to_dlc(8, false), 8);
        assert_eq!(len_to_dlc(64, false), 8); // Clamped to 8
    }

    #[test]
    fn test_len_to_dlc_fd() {
        assert_eq!(len_to_dlc(0, true), 0);
        assert_eq!(len_to_dlc(8, true), 8);
        assert_eq!(len_to_dlc(12, true), 9);
        assert_eq!(len_to_dlc(64, true), 15);
    }

    #[test]
    fn test_frame_creation() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let frame = GsUsbFrame::with_data(0x7FF, &data);

        assert_eq!(frame.arbitration_id(), 0x7FF);
        assert!(!frame.is_extended_id());
        assert!(!frame.is_fd());
        assert_eq!(frame.data_length(), 8);
        assert_eq!(frame.data(), &data);
    }

    #[test]
    fn test_fd_frame_creation() {
        let data: Vec<u8> = (0..64).collect();
        let frame = GsUsbFrame::with_fd_data(0x123 | CAN_EFF_FLAG, &data, true);

        assert_eq!(frame.arbitration_id(), 0x123);
        assert!(frame.is_extended_id());
        assert!(frame.is_fd());
        assert!(frame.is_brs());
        assert_eq!(frame.data_length(), 64);
    }

    #[test]
    fn test_pack_unpack_classic() {
        let data = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let frame = GsUsbFrame::with_data(0x7FF, &data);

        let packed = frame.pack(false, false);
        assert_eq!(packed.len(), GS_USB_FRAME_SIZE);

        let unpacked = GsUsbFrame::from_bytes(&packed, false, false).unwrap();
        assert_eq!(unpacked.can_id, frame.can_id);
        assert_eq!(unpacked.data(), frame.data());
    }

    #[test]
    fn test_pack_unpack_fd() {
        let data: Vec<u8> = (0..64).collect();
        let frame = GsUsbFrame::with_fd_data(0x123, &data, true);

        let packed = frame.pack(true, true);
        assert_eq!(packed.len(), GS_USB_FRAME_SIZE_FD_HW_TIMESTAMP);

        let unpacked = GsUsbFrame::from_bytes(&packed, true, true).unwrap();
        assert_eq!(unpacked.can_id, frame.can_id);
        assert_eq!(unpacked.flags, frame.flags);
        assert_eq!(unpacked.data(), frame.data());
    }

    #[test]
    fn rejects_short_and_truncated_rx_frames() {
        assert!(matches!(
            GsUsbFrame::from_bytes(&[0; 11], false, false),
            Err(GsUsbError::InvalidResponse {
                expected: 12,
                actual: 11
            })
        ));

        let mut classic = GsUsbFrame::with_data(0x123, &[1, 2, 3, 4]);
        classic.echo_id = GS_USB_RX_ECHO_ID;
        let bytes = classic.pack(false, false);
        assert!(matches!(
            GsUsbFrame::from_bytes(&bytes[..14], false, false),
            Err(GsUsbError::InvalidResponse {
                expected: 16,
                actual: 14
            })
        ));

        let mut fd = GsUsbFrame::with_fd_data(0x123, &[1; 64], false);
        fd.echo_id = GS_USB_RX_ECHO_ID;
        let bytes = fd.pack(true, true);
        assert!(matches!(
            GsUsbFrame::from_bytes(&bytes[..76], true, true),
            Err(GsUsbError::InvalidResponse {
                expected: 80,
                actual: 76
            })
        ));
    }

    #[test]
    fn tx_frames_omit_timestamps_and_reject_fd_in_classic_mode() {
        let classic = GsUsbFrame::with_data(0x123, &[1, 2, 3]);
        assert_eq!(classic.pack_for_tx(false).unwrap().len(), GS_USB_FRAME_SIZE);

        let fd = GsUsbFrame::with_fd_data(0x123, &[7; 64], true);
        assert_eq!(fd.pack_for_tx(true).unwrap().len(), GS_USB_FRAME_SIZE_FD);
        assert!(matches!(
            fd.pack_for_tx(false),
            Err(GsUsbError::FdNotSupported)
        ));

        let mut invalid = classic;
        invalid.can_dlc = 9;
        assert!(matches!(
            invalid.pack_for_tx(false),
            Err(GsUsbError::InvalidFrame("classic CAN DLC exceeds 8"))
        ));
    }

    #[test]
    fn accepts_short_echo_and_remote_frames() {
        let echo = GsUsbFrame::with_data(0x123, &[1, 2, 3]);
        let packed = echo.pack(false, false);
        let unpacked = GsUsbFrame::from_bytes(&packed[..12], false, false).unwrap();
        assert!(unpacked.is_echo_frame());
        assert_eq!(unpacked.can_dlc, 3);

        let mut remote = GsUsbFrame::with_data(0x123 | CAN_RTR_FLAG, &[]);
        remote.echo_id = GS_USB_RX_ECHO_ID;
        remote.can_dlc = 8;
        let packed = remote.pack(false, false);
        let unpacked = GsUsbFrame::from_bytes(&packed[..12], false, false).unwrap();
        assert!(unpacked.is_remote_frame());
        assert_eq!(unpacked.can_dlc, 8);
    }

    #[test]
    fn reads_classic_hardware_timestamp_after_full_data_field() {
        let mut frame = GsUsbFrame::with_data(0x123, &[0x42]);
        frame.echo_id = GS_USB_RX_ECHO_ID;
        frame.timestamp_us = 123_456;
        let packed = frame.pack(true, false);
        let unpacked = GsUsbFrame::from_bytes(&packed, true, false).unwrap();
        assert_eq!(unpacked.data(), &[0x42]);
        assert_eq!(unpacked.timestamp_us, 123_456);
    }
}
