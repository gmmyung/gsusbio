//! nusb setup and low-level operations for the async socket.

use std::time::Duration;

use nusb::transfer::{Bulk, ControlIn, ControlOut, ControlType, In, Out, Recipient, TransferError};
use nusb::{Device, Endpoint, Interface};

use crate::config::Config;
use crate::constants::*;
use crate::discovery::DiscoveredDevice;
use crate::socket_error::{Error, Result};
use crate::structures::{DeviceCapability, DeviceInfo, DeviceMode};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct AsyncUsb {
    // Keep the device alive for the lifetime of the claimed interface.
    pub _device: Device,
    pub interface: Interface,
    pub endpoint_in: Endpoint<Bulk, In>,
    pub endpoint_out: Endpoint<Bulk, Out>,
    pub channel: u8,
    pub fd_enabled: bool,
    pub hardware_timestamps: bool,
}

impl AsyncUsb {
    pub async fn connect(selected: DiscoveredDevice, config: &Config) -> Result<Self> {
        config.validate()?;
        let device = selected.inner.open().await?;
        let interface = device.detach_and_claim_interface(0).await?;

        // A STALL here means firmware does not implement the legacy request.
        let host_format = 0x0000_BEEFu32.to_le_bytes();
        match control_out(&interface, GS_USB_BREQ_HOST_FORMAT, 0, &host_format).await {
            Ok(()) | Err(Error::Transfer(TransferError::Stall)) => {}
            Err(error) => return Err(error),
        }

        let info = control_in(&interface, GS_USB_BREQ_DEVICE_CONFIG, 0, 12).await?;
        let info = DeviceInfo::unpack(&info);
        if u16::from(config.channel) > u16::from(info.icount) {
            return Err(Error::InvalidConfiguration(
                "CAN channel is outside device range",
            ));
        }

        let capability =
            control_in(&interface, GS_USB_BREQ_BT_CONST, config.channel.into(), 40).await?;
        let mut capability = DeviceCapability::unpack(&capability);
        if config.fd_enabled {
            if capability.feature & GS_CAN_FEATURE_FD == 0 {
                return Err(Error::UnsupportedFeature("CAN FD"));
            }
            if capability.feature & GS_CAN_FEATURE_BT_CONST_EXT == 0 {
                return Err(Error::UnsupportedFeature("CAN FD data timing constants"));
            }
            let extended = control_in(
                &interface,
                GS_USB_BREQ_BT_CONST_EXT,
                config.channel.into(),
                72,
            )
            .await?;
            capability = DeviceCapability::unpack_extended(&extended);
        }
        let flags = config.mode_flags(&capability)?;
        let (nominal, data) = config.timings(&capability)?;

        // Open endpoints before changing the CAN channel state.
        let endpoint_in = interface.endpoint::<Bulk, In>(GS_USB_ENDPOINT_IN)?;
        let endpoint_out = interface.endpoint::<Bulk, Out>(GS_USB_ENDPOINT_OUT)?;

        control_out(
            &interface,
            GS_USB_BREQ_BITTIMING,
            config.channel.into(),
            &nominal.pack(),
        )
        .await?;
        if let Some(data) = data {
            control_out(
                &interface,
                GS_USB_BREQ_DATA_BITTIMING,
                config.channel.into(),
                &data.pack(),
            )
            .await?;
        }
        let mode = DeviceMode::new(GS_CAN_MODE_START, flags).pack();
        if let Err(error) =
            control_out(&interface, GS_USB_BREQ_MODE, config.channel.into(), &mode).await
        {
            let reset = DeviceMode::new(GS_CAN_MODE_RESET, 0).pack();
            let _ = control_out(&interface, GS_USB_BREQ_MODE, config.channel.into(), &reset).await;
            return Err(error);
        }

        Ok(Self {
            _device: device,
            interface,
            endpoint_in,
            endpoint_out,
            channel: config.channel,
            fd_enabled: config.fd_enabled,
            hardware_timestamps: config.hardware_timestamps,
        })
    }

    pub async fn stop(&self) -> Result<()> {
        let mode = DeviceMode::new(GS_CAN_MODE_RESET, 0).pack();
        control_out(
            &self.interface,
            GS_USB_BREQ_MODE,
            self.channel.into(),
            &mode,
        )
        .await
    }
}

async fn control_out(interface: &Interface, request: u8, value: u16, data: &[u8]) -> Result<()> {
    interface
        .control_out(
            ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Interface,
                request,
                value,
                index: 0,
                data,
            },
            CONTROL_TIMEOUT,
        )
        .await
        .map_err(Error::from_transfer)
}

async fn control_in(
    interface: &Interface,
    request: u8,
    value: u16,
    length: usize,
) -> Result<Vec<u8>> {
    let bytes = interface
        .control_in(
            ControlIn {
                control_type: ControlType::Vendor,
                recipient: Recipient::Interface,
                request,
                value,
                index: 0,
                length: length as u16,
            },
            CONTROL_TIMEOUT,
        )
        .await
        .map_err(Error::from_transfer)?;
    if bytes.len() != length {
        return Err(Error::Protocol("USB control response has the wrong length"));
    }
    Ok(bytes)
}
