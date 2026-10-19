//! Socket configuration independent of USB I/O.

use std::time::Duration;

use crate::constants::{
    GS_CAN_FEATURE_FD, GS_CAN_MODE_FD, GS_CAN_MODE_HW_TIMESTAMP, GS_CAN_MODE_LISTEN_ONLY,
    GS_CAN_MODE_LOOP_BACK, GS_CAN_MODE_ONE_SHOT,
};
use crate::socket_error::{Error, Result};
use crate::structures::{DeviceBitTiming, DeviceCapability};
use crate::timing::calculate_bittiming;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bitrate {
    Kbps10,
    Kbps20,
    Kbps50,
    Kbps100,
    Kbps125,
    Kbps250,
    Kbps500,
    Kbps750,
    Mbps1,
    Mbps2,
    Mbps5,
    Custom(u32),
}

impl Bitrate {
    pub fn bps(self) -> u32 {
        match self {
            Self::Kbps10 => 10_000,
            Self::Kbps20 => 20_000,
            Self::Kbps50 => 50_000,
            Self::Kbps100 => 100_000,
            Self::Kbps125 => 125_000,
            Self::Kbps250 => 250_000,
            Self::Kbps500 => 500_000,
            Self::Kbps750 => 750_000,
            Self::Mbps1 => 1_000_000,
            Self::Mbps2 => 2_000_000,
            Self::Mbps5 => 5_000_000,
            Self::Custom(bps) => bps,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CanMode {
    #[default]
    Normal,
    ListenOnly,
    Loopback,
    OneShot,
}

impl CanMode {
    pub(crate) fn flags(self) -> u32 {
        match self {
            Self::Normal => 0,
            Self::ListenOnly => GS_CAN_MODE_LISTEN_ONLY,
            Self::Loopback => GS_CAN_MODE_LOOP_BACK,
            Self::OneShot => GS_CAN_MODE_ONE_SHOT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverflowPolicy {
    #[default]
    DropNewest,
    DropOldest,
}

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub channel: u8,
    pub nominal_bitrate: Bitrate,
    pub data_bitrate: Option<Bitrate>,
    pub fd_enabled: bool,
    pub hardware_timestamps: bool,
    pub mode: CanMode,
    pub rx_queue_capacity: usize,
    pub tx_queue_capacity: usize,
    pub overflow_policy: OverflowPolicy,
    pub transfer_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            channel: 0,
            nominal_bitrate: Bitrate::Kbps500,
            data_bitrate: None,
            fd_enabled: false,
            hardware_timestamps: false,
            mode: CanMode::Normal,
            rx_queue_capacity: 1024,
            tx_queue_capacity: 256,
            overflow_policy: OverflowPolicy::DropNewest,
            transfer_timeout: Duration::from_secs(2),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        if self.nominal_bitrate.bps() == 0 {
            return Err(Error::InvalidConfiguration(
                "nominal bitrate must be positive",
            ));
        }
        if self.rx_queue_capacity == 0 || self.tx_queue_capacity == 0 {
            return Err(Error::InvalidConfiguration(
                "queue capacities must be positive",
            ));
        }
        if self.transfer_timeout.is_zero() {
            return Err(Error::InvalidConfiguration(
                "transfer timeout must be positive",
            ));
        }
        if self.fd_enabled && self.data_bitrate.is_none() {
            return Err(Error::InvalidConfiguration(
                "CAN FD requires a data bitrate",
            ));
        }
        if let Some(rate) = self.data_bitrate {
            if !self.fd_enabled {
                return Err(Error::InvalidConfiguration(
                    "data bitrate requires CAN FD mode",
                ));
            }
            if rate.bps() == 0 {
                return Err(Error::InvalidConfiguration("data bitrate must be positive"));
            }
        }
        #[cfg(not(feature = "can-fd"))]
        if self.fd_enabled {
            return Err(Error::UnsupportedFeature(
                "CAN FD is disabled at compile time",
            ));
        }
        Ok(())
    }

    pub(crate) fn mode_flags(&self, capability: &DeviceCapability) -> Result<u32> {
        let mut flags = self.mode.flags();
        if self.hardware_timestamps {
            flags |= GS_CAN_MODE_HW_TIMESTAMP;
        }
        if self.fd_enabled {
            flags |= GS_CAN_MODE_FD;
        }
        if flags & !capability.feature != 0 {
            return Err(Error::UnsupportedFeature("requested device mode"));
        }
        Ok(flags)
    }

    pub(crate) fn timings(
        &self,
        capability: &DeviceCapability,
    ) -> Result<(DeviceBitTiming, Option<DeviceBitTiming>)> {
        let nominal_rate = self.nominal_bitrate.bps();
        let nominal_sample_point = if nominal_rate >= 1_000_000 { 750 } else { 875 };
        let nominal = calculate_bittiming(capability, nominal_rate, false, nominal_sample_point)
            .ok_or(Error::UnsupportedFeature(
                "nominal bitrate is outside device timing limits",
            ))?;
        let data = if let Some(rate) = self.data_bitrate {
            if capability.feature & GS_CAN_FEATURE_FD == 0 {
                return Err(Error::UnsupportedFeature("CAN FD"));
            }
            Some(
                calculate_bittiming(capability, rate.bps(), true, 750).ok_or(
                    Error::UnsupportedFeature("data bitrate is outside device timing limits"),
                )?,
            )
        } else {
            None
        };
        Ok((nominal, data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_inconsistent_rates() {
        let mut config = Config {
            fd_enabled: true,
            ..Config::default()
        };
        assert!(config.validate().is_err());
        config.data_bitrate = Some(Bitrate::Mbps2);
        #[cfg(feature = "can-fd")]
        assert!(config.validate().is_ok());
        #[cfg(not(feature = "can-fd"))]
        assert!(config.validate().is_err());
    }
}
