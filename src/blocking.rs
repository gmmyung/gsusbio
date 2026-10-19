//! Direct blocking GS-USB socket API.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::can::CanFrame;
use crate::config::{Bitrate, CanMode, Config};
use crate::constants::{CAN_ERR_FLAG, GS_CAN_FLAG_OVERFLOW, GS_USB_RX_ECHO_ID};
use crate::device::GsUsb;
use crate::discovery::{scan_blocking, DiscoveredDevice};
use crate::error::GsUsbError;
use crate::received::{ReceiveEvent, ReceivedFrame};
use crate::socket_error::{Error, Result};

pub struct CanSocket {
    device: GsUsb,
    channel: u8,
    fd_enabled: bool,
    hardware_timestamps: bool,
    timeout: Duration,
    pending: VecDeque<ReceiveEvent>,
}

impl CanSocket {
    pub fn scan() -> Result<Vec<DiscoveredDevice>> {
        scan_blocking()
    }

    pub fn builder(device: DiscoveredDevice) -> CanSocketBuilder {
        CanSocketBuilder {
            device,
            config: Config::default(),
        }
    }

    /// Success confirms a complete USB OUT transfer, not CAN bus acknowledgment.
    pub fn send(&mut self, frame: CanFrame) -> Result<()> {
        if frame.is_fd() && !self.fd_enabled {
            return Err(Error::UnsupportedFeature("CAN FD is not enabled"));
        }
        let raw = frame.to_raw(self.channel, 0)?;
        self.device.send(&raw)?;
        Ok(())
    }

    pub fn recv(&mut self) -> Result<CanFrame> {
        loop {
            match self.recv_event()? {
                ReceiveEvent::Frame(frame) => return Ok(frame.frame),
                ReceiveEvent::ErrorFrame { .. } | ReceiveEvent::Overflow { .. } => {}
            }
        }
    }

    pub fn recv_with_metadata(&mut self) -> Result<ReceivedFrame> {
        loop {
            match self.recv_event()? {
                ReceiveEvent::Frame(frame) => return Ok(frame),
                ReceiveEvent::ErrorFrame { .. } | ReceiveEvent::Overflow { .. } => {}
            }
        }
    }

    pub fn recv_event(&mut self) -> Result<ReceiveEvent> {
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Ok(event);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let raw = match self.device.read(remaining) {
                Ok(raw) => raw,
                Err(GsUsbError::ReadTimeout) => return Err(Error::Timeout),
                Err(error) => return Err(Error::from(error)),
            };
            if raw.channel != self.channel || raw.echo_id != GS_USB_RX_ECHO_ID {
                continue;
            }
            if raw.flags & GS_CAN_FLAG_OVERFLOW != 0 {
                self.pending.push_back(ReceiveEvent::Overflow {
                    channel: self.channel,
                });
            }
            let timestamp = self.hardware_timestamps.then_some(raw.timestamp_us);
            if raw.can_id & CAN_ERR_FLAG != 0 {
                let mut data = [0u8; 8];
                data.copy_from_slice(&raw.data[..8]);
                self.pending.push_back(ReceiveEvent::ErrorFrame {
                    channel: self.channel,
                    can_id: raw.can_id,
                    data,
                    timestamp,
                });
            } else {
                let frame = CanFrame::from_raw(&raw)?;
                self.pending.push_back(ReceiveEvent::Frame(ReceivedFrame {
                    frame,
                    channel: self.channel,
                    timestamp,
                }));
            }
        }
    }

    pub fn close(mut self) -> Result<()> {
        self.device.stop()?;
        Ok(())
    }
}

pub struct CanSocketBuilder {
    device: DiscoveredDevice,
    config: Config,
}

impl CanSocketBuilder {
    pub fn channel(mut self, channel: u8) -> Self {
        self.config.channel = channel;
        self
    }

    pub fn nominal_bitrate(mut self, rate: Bitrate) -> Self {
        self.config.nominal_bitrate = rate;
        self
    }

    pub fn data_bitrate(mut self, rate: Bitrate) -> Self {
        self.config.data_bitrate = Some(rate);
        self
    }

    pub fn fd_enabled(mut self, yes: bool) -> Self {
        self.config.fd_enabled = yes;
        self
    }

    pub fn hardware_timestamps(mut self, yes: bool) -> Self {
        self.config.hardware_timestamps = yes;
        self
    }

    pub fn mode(mut self, mode: CanMode) -> Self {
        self.config.mode = mode;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.transfer_timeout = timeout;
        self
    }

    pub fn connect(self) -> Result<CanSocket> {
        self.config.validate()?;
        let mut device = GsUsb::open_info(self.device.inner)?;
        let info = device.device_info()?;
        if u16::from(self.config.channel) > u16::from(info.icount) {
            return Err(Error::InvalidConfiguration(
                "CAN channel is outside device range",
            ));
        }
        device.select_channel(self.config.channel);
        let mut capability = device.device_capability()?;
        if self.config.fd_enabled {
            capability = device
                .device_capability_extended()?
                .ok_or(Error::UnsupportedFeature("CAN FD data timing constants"))?;
        }
        let flags = self.config.mode_flags(&capability)?;
        let (nominal, data) = self.config.timings(&capability)?;
        device.set_timing(
            nominal.prop_seg,
            nominal.phase_seg1,
            nominal.phase_seg2,
            nominal.sjw,
            nominal.brp,
        )?;
        if let Some(data) = data {
            device.set_data_timing(
                data.prop_seg,
                data.phase_seg1,
                data.phase_seg2,
                data.sjw,
                data.brp,
            )?;
        }
        device.start(flags)?;
        Ok(CanSocket {
            device,
            channel: self.config.channel,
            fd_enabled: self.config.fd_enabled,
            hardware_timestamps: self.config.hardware_timestamps,
            timeout: self.config.transfer_timeout,
            pending: VecDeque::new(),
        })
    }
}
