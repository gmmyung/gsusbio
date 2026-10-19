//! CAN frames and device events received over USB.

use crate::can::CanFrame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedFrame {
    pub frame: CanFrame,
    pub channel: u8,
    pub timestamp: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveEvent {
    Frame(ReceivedFrame),
    ErrorFrame {
        channel: u8,
        can_id: u32,
        data: [u8; 8],
        timestamp: Option<u32>,
    },
    Overflow {
        channel: u8,
    },
}
