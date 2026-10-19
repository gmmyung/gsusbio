//! GS-USB device implementation
//!
//! This module provides the `GsUsb` struct for interfacing with GS-USB compatible
//! CAN adapters, including candleLight, CANable, and similar devices.

use std::time::Duration;

use nusb::transfer::{
    Buffer, Bulk, ControlIn, ControlOut, ControlType, In, Out, Recipient, TransferError,
};
use nusb::{Device, DeviceInfo as UsbDeviceInfo, Endpoint, Interface, MaybeFuture};

use crate::constants::*;
use crate::error::{GsUsbError, Result};
use crate::frame::GsUsbFrame;
use crate::structures::{DeviceBitTiming, DeviceCapability, DeviceInfo, DeviceMode, DeviceState};
use crate::timing::calculate_bittiming;

/// GS-USB device handle
///
/// Provides methods for interacting with GS-USB compatible CAN adapters.
///
pub struct GsUsb {
    /// USB device handle
    handle: Device,
    usb_info: UsbDeviceInfo,
    interface: Option<Interface>,
    endpoint_in: Option<Endpoint<Bulk, In>>,
    endpoint_out: Option<Endpoint<Bulk, Out>>,
    host_format_attempted: bool,
    /// Cached device capability
    capability: Option<DeviceCapability>,
    /// Current device flags
    device_flags: u32,
    /// Whether FD mode is enabled
    fd_mode: bool,
    /// Whether the device has been started
    started: bool,
    /// USB bus number
    bus: u8,
    /// USB device address
    address: u8,
    /// Selected CAN channel
    channel: u16,
    /// Device serial number (cached)
    serial_number: Option<String>,
    /// Last nominal (arbitration) phase bit timing that was set
    last_timing: Option<DeviceBitTiming>,
    /// Last data phase (CAN FD) bit timing that was set
    last_data_timing: Option<DeviceBitTiming>,
}

impl GsUsb {
    /// Create a new GsUsb from a USB device handle
    fn new(handle: Device, usb_info: UsbDeviceInfo) -> Self {
        let bus = Self::bus_number(&usb_info);
        let address = usb_info.device_address();
        Self {
            handle,
            usb_info,
            interface: None,
            endpoint_in: None,
            endpoint_out: None,
            host_format_attempted: false,
            capability: None,
            device_flags: 0,
            fd_mode: false,
            started: false,
            bus,
            address,
            channel: 0,
            serial_number: None,
            last_timing: None,
            last_data_timing: None,
        }
    }

    pub(crate) fn open_info(usb_info: UsbDeviceInfo) -> Result<Self> {
        let handle = usb_info.open().wait()?;
        Ok(Self::new(handle, usb_info))
    }

    pub(crate) fn select_channel(&mut self, channel: u8) {
        self.channel = u16::from(channel);
    }

    /// Start the GS-USB device
    ///
    /// # Arguments
    /// * `flags` - Mode flags (combination of GS_CAN_MODE_* constants)
    ///
    pub fn start(&mut self, flags: u32) -> Result<()> {
        if self.started {
            return Err(GsUsbError::AlreadyStarted);
        }

        // Get capability to check supported features
        let capability = self.device_capability()?;
        let driver_flags = GS_CAN_MODE_LISTEN_ONLY
            | GS_CAN_MODE_LOOP_BACK
            | GS_CAN_MODE_TRIPLE_SAMPLE
            | GS_CAN_MODE_ONE_SHOT
            | GS_CAN_MODE_HW_TIMESTAMP
            | GS_CAN_MODE_IDENTIFY
            | GS_CAN_MODE_USER_ID
            | GS_CAN_MODE_PAD_PKTS_TO_MAX_PKT_SIZE
            | GS_CAN_MODE_FD
            | GS_CAN_MODE_BERR_REPORTING;
        let unsupported = flags & !(capability.feature & driver_flags);
        if unsupported != 0 {
            return Err(GsUsbError::UnsupportedModeFlags { unsupported });
        }

        let mode = DeviceMode::new(GS_CAN_MODE_START, flags);
        self.control_out(GS_USB_BREQ_MODE, self.channel, &mode.pack())?;

        self.device_flags = flags;
        self.fd_mode = flags & GS_CAN_MODE_FD != 0;
        self.started = true;
        Ok(())
    }

    /// Stop the GS-USB device
    pub fn stop(&mut self) -> Result<()> {
        if !self.started {
            return Ok(());
        }
        let mode = DeviceMode::new(GS_CAN_MODE_RESET, 0);
        self.control_out(GS_USB_BREQ_MODE, self.channel, &mode.pack())?;
        self.started = false;
        self.device_flags = 0;
        self.fd_mode = false;
        Ok(())
    }

    /// Set the CAN bitrate
    ///
    /// Configures an exact nominal bitrate using the device's timing limits.
    /// Prefers an 87.5% sample point below 1 Mbps and 75% at 1 Mbps or above.
    ///
    /// # Arguments
    /// * `bitrate` - Bitrate in bits per second (e.g., 250000 for 250 kbps)
    ///
    /// # Common target bitrates (subject to device timing limits)
    /// - 10000 (10 kbps)
    /// - 20000 (20 kbps)
    /// - 50000 (50 kbps)
    /// - 100000 (100 kbps)
    /// - 125000 (125 kbps)
    /// - 250000 (250 kbps)
    /// - 500000 (500 kbps)
    /// - 800000 (800 kbps)
    /// - 1000000 (1 Mbps)
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        let capability = self.device_capability()?;
        let sample_point = if bitrate >= 1_000_000 { 750 } else { 875 };
        match calculate_bittiming(&capability, bitrate, false, sample_point) {
            Some(timing) => self.set_timing(
                timing.prop_seg,
                timing.phase_seg1,
                timing.phase_seg2,
                timing.sjw,
                timing.brp,
            ),
            None => Err(GsUsbError::UnsupportedBitrate {
                bitrate,
                clock_hz: capability.fclk_can,
            }),
        }
    }

    /// Set raw CAN bit timing parameters
    ///
    /// # Arguments
    /// * `prop_seg` - Propagation segment (typically 1)
    /// * `phase_seg1` - Phase segment 1 (1-15)
    /// * `phase_seg2` - Phase segment 2 (1-8)
    /// * `sjw` - Synchronization jump width (1-4)
    /// * `brp` - Baud rate prescaler (1-1024)
    pub fn set_timing(
        &mut self,
        prop_seg: u32,
        phase_seg1: u32,
        phase_seg2: u32,
        sjw: u32,
        brp: u32,
    ) -> Result<()> {
        let timing = DeviceBitTiming::new(prop_seg, phase_seg1, phase_seg2, sjw, brp);
        self.control_out(GS_USB_BREQ_BITTIMING, self.channel, &timing.pack())?;
        self.last_timing = Some(timing);
        Ok(())
    }

    /// Set CAN FD data phase bit timing parameters
    pub fn set_data_timing(
        &mut self,
        prop_seg: u32,
        phase_seg1: u32,
        phase_seg2: u32,
        sjw: u32,
        brp: u32,
    ) -> Result<()> {
        let timing = DeviceBitTiming::new(prop_seg, phase_seg1, phase_seg2, sjw, brp);
        self.control_out(GS_USB_BREQ_DATA_BITTIMING, self.channel, &timing.pack())?;
        self.last_data_timing = Some(timing);
        Ok(())
    }

    /// Get the last nominal (arbitration) phase timing that was set via `set_timing`/`set_bitrate*`
    pub fn last_timing(&self) -> Option<DeviceBitTiming> {
        self.last_timing
    }

    /// Get the last data phase (CAN FD) timing that was set via `set_data_timing`/`set_data_bitrate*`
    pub fn last_data_timing(&self) -> Option<DeviceBitTiming> {
        self.last_data_timing
    }

    /// Set CAN FD data phase bitrate
    ///
    /// Configures an exact data bitrate with a preferred 75% sample point.
    ///
    /// # Arguments
    /// * `bitrate` - Data phase bitrate in bits per second
    pub fn set_data_bitrate(&mut self, bitrate: u32) -> Result<()> {
        let capability = self.device_capability()?;

        // Check if device supports CAN FD
        if (capability.feature & GS_CAN_FEATURE_FD) == 0 {
            return Err(GsUsbError::FdNotSupported);
        }

        let capability = self.device_capability_extended()?.unwrap_or(capability);
        match calculate_bittiming(&capability, bitrate, true, 750) {
            Some(timing) => self.set_data_timing(
                timing.prop_seg,
                timing.phase_seg1,
                timing.phase_seg2,
                timing.sjw,
                timing.brp,
            ),
            None => Err(GsUsbError::UnsupportedDataBitrate {
                bitrate,
                clock_hz: capability.fclk_can,
            }),
        }
    }

    /// Send a CAN frame
    ///
    /// # Arguments
    /// * `frame` - The CAN frame to send
    pub fn send(&mut self, frame: &GsUsbFrame) -> Result<()> {
        if !self.started {
            return Err(GsUsbError::NotStarted);
        }
        let data = frame.pack_for_tx(self.fd_mode)?;
        let expected_len = data.len();
        if self.endpoint_out.is_none() {
            let endpoint = self
                .ensure_interface()?
                .endpoint::<Bulk, Out>(GS_USB_ENDPOINT_OUT)?;
            self.endpoint_out = Some(endpoint);
        }
        let completion = self
            .endpoint_out
            .as_mut()
            .unwrap()
            .transfer_blocking(data.into(), Duration::from_millis(1000));
        match completion.status {
            Ok(()) if completion.actual_len == expected_len => Ok(()),
            Ok(()) => Err(GsUsbError::IncompleteWrite {
                expected: expected_len,
                actual: completion.actual_len,
            }),
            Err(TransferError::Cancelled) => Err(GsUsbError::WriteTimeout),
            Err(e) => Err(GsUsbError::BulkTransfer(e)),
        }
    }

    /// Read a CAN frame
    ///
    /// # Arguments
    /// * `timeout` - Read timeout duration
    ///
    /// # Returns
    /// The received CAN frame, or an error if timeout or other failure
    pub fn read(&mut self, timeout: Duration) -> Result<GsUsbFrame> {
        if !self.started {
            return Err(GsUsbError::NotStarted);
        }
        let hw_timestamps = (self.device_flags & GS_CAN_MODE_HW_TIMESTAMP) != 0;
        let max_size = GsUsbFrame::frame_size(hw_timestamps, self.fd_mode);
        if self.endpoint_in.is_none() {
            let endpoint = self
                .ensure_interface()?
                .endpoint::<Bulk, In>(GS_USB_ENDPOINT_IN)?;
            self.endpoint_in = Some(endpoint);
        }
        let endpoint = self.endpoint_in.as_mut().unwrap();
        // nusb requires IN requests to be a multiple of the endpoint packet
        // size. A single GS-USB frame still ends with a short USB packet.
        let packet_size = endpoint.max_packet_size();
        if packet_size == 0 {
            return Err(GsUsbError::InvalidFrame(
                "USB endpoint has zero packet size",
            ));
        }
        let request_size = max_size.div_ceil(packet_size) * packet_size;
        let completion = endpoint.transfer_blocking(Buffer::new(request_size), timeout);
        match completion.status {
            Ok(()) => {}
            Err(TransferError::Cancelled) => return Err(GsUsbError::ReadTimeout),
            Err(e) => return Err(GsUsbError::BulkTransfer(e)),
        }
        let buf = completion.buffer;
        let len = completion.actual_len;

        // Determine if this is an FD frame by checking the flags byte (offset 10)
        let is_fd_frame = if len >= 11 {
            (buf[10] & GS_CAN_FLAG_FD) != 0
        } else {
            false
        };

        GsUsbFrame::from_bytes(&buf[..len], hw_timestamps, is_fd_frame)
    }

    /// Get the numeric USB bus number. On non-Linux platforms, use `bus_id`
    /// because the OS may not expose a numeric bus number.
    pub fn bus(&self) -> u8 {
        self.bus
    }

    /// Get the OS-provided USB bus identifier.
    pub fn bus_id(&self) -> &str {
        self.usb_info.bus_id()
    }

    /// Get the USB device address
    pub fn address(&self) -> u8 {
        self.address
    }

    /// Get the device serial number
    pub fn serial_number(&mut self) -> Result<String> {
        if let Some(ref sn) = self.serial_number {
            return Ok(sn.clone());
        }

        let sn = self
            .usb_info
            .get_serial_number_string()
            .wait()?
            .unwrap_or_default();
        self.serial_number = Some(sn.clone());
        Ok(sn)
    }

    /// Get device information (channel count, firmware/hardware version)
    pub fn device_info(&mut self) -> Result<DeviceInfo> {
        let data = self.control_in(GS_USB_BREQ_DEVICE_CONFIG, 0, 12)?;
        Ok(DeviceInfo::unpack(&data))
    }

    /// Get device capability (bit timing constraints, feature flags)
    pub fn device_capability(&mut self) -> Result<DeviceCapability> {
        if let Some(ref cap) = self.capability {
            return Ok(*cap);
        }

        let data = self.control_in(GS_USB_BREQ_BT_CONST, 0, 40)?;
        let cap = DeviceCapability::unpack(&data);
        self.capability = Some(cap);
        Ok(cap)
    }

    /// Get extended device capability (includes CAN FD timing constraints)
    ///
    /// Returns `None` if device doesn't support BT_CONST_EXT
    pub fn device_capability_extended(&mut self) -> Result<Option<DeviceCapability>> {
        let cap = self.device_capability()?;

        // Check if device supports extended capability
        if (cap.feature & GS_CAN_FEATURE_BT_CONST_EXT) == 0 {
            return Ok(None);
        }

        // If we already have extended capability cached, return it
        if let Some(ref cap) = self.capability {
            if cap.has_fd_timing() {
                return Ok(Some(*cap));
            }
        }

        // Fetch extended capability and replace the basic one
        let data = self.control_in(GS_USB_BREQ_BT_CONST_EXT, 0, 72)?;
        let cap = DeviceCapability::unpack_extended(&data);
        self.capability = Some(cap);
        Ok(Some(cap))
    }

    /// Check if device supports CAN FD
    pub fn supports_fd(&mut self) -> Result<bool> {
        let cap = self.device_capability()?;
        Ok((cap.feature & GS_CAN_FEATURE_FD) != 0)
    }

    /// Check if device supports GET_STATE request
    pub fn supports_get_state(&mut self) -> Result<bool> {
        let cap = self.device_capability()?;
        Ok((cap.feature & GS_CAN_FEATURE_GET_STATE) != 0)
    }

    /// Get CAN bus state and error counters
    ///
    /// # Arguments
    /// * `channel` - CAN channel number (default 0)
    pub fn get_state(&mut self, channel: u16) -> Result<DeviceState> {
        if !self.supports_get_state()? {
            return Err(GsUsbError::GetStateNotSupported);
        }

        let data = self.control_in(GS_USB_BREQ_GET_STATE, channel, 12)?;
        Ok(DeviceState::unpack(&data))
    }

    /// Send HOST_FORMAT request (legacy requirement)
    ///
    /// This sets the byte order for the device. Most modern devices
    /// don't require this, but it's included for compatibility.
    pub fn send_host_format(&mut self) -> Result<()> {
        let host_format: [u8; 4] = 0x0000_BEEFu32.to_le_bytes();
        let interface = self.ensure_interface()?;
        interface
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request: GS_USB_BREQ_HOST_FORMAT,
                    value: 0,
                    index: 0,
                    data: &host_format,
                },
                Duration::from_millis(1000),
            )
            .wait()
            .map_err(GsUsbError::ControlTransfer)?;
        self.host_format_attempted = true;
        Ok(())
    }

    fn ensure_interface(&mut self) -> Result<&Interface> {
        if self.interface.is_none() {
            let interface = self
                .handle
                .detach_and_claim_interface(0)
                .wait()
                .map_err(GsUsbError::ClaimInterface)?;
            self.interface = Some(interface);
        }
        Ok(self.interface.as_ref().unwrap())
    }

    fn ensure_host_format(&mut self) -> Result<()> {
        if self.host_format_attempted {
            return Ok(());
        }
        match self.send_host_format() {
            Ok(()) => Ok(()),
            // candleLight and some compatible firmware omit this legacy request.
            Err(GsUsbError::ControlTransfer(TransferError::Stall)) => {
                self.host_format_attempted = true;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Perform a control OUT transfer
    fn control_out(&mut self, request: u8, value: u16, data: &[u8]) -> Result<()> {
        self.ensure_host_format()?;
        self.ensure_interface()?
            .control_out(
                ControlOut {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request,
                    value,
                    index: 0,
                    data,
                },
                Duration::from_millis(1000),
            )
            .wait()
            .map_err(GsUsbError::ControlTransfer)?;
        Ok(())
    }

    /// Perform a control IN transfer
    fn control_in(&mut self, request: u8, value: u16, length: usize) -> Result<Vec<u8>> {
        self.ensure_host_format()?;
        let buf = self
            .ensure_interface()?
            .control_in(
                ControlIn {
                    control_type: ControlType::Vendor,
                    recipient: Recipient::Interface,
                    request,
                    value,
                    index: 0,
                    length: length as u16,
                },
                Duration::from_millis(1000),
            )
            .wait()
            .map_err(GsUsbError::ControlTransfer)?;

        if buf.len() != length {
            return Err(GsUsbError::InvalidResponse {
                expected: length,
                actual: buf.len(),
            });
        }

        Ok(buf)
    }

    /// Check if a USB device is a GS-USB device
    pub(crate) fn is_gs_usb_device(vendor_id: u16, product_id: u16) -> bool {
        matches!(
            (vendor_id, product_id),
            (GS_USB_ID_VENDOR, GS_USB_ID_PRODUCT)
                | (GS_USB_CANDLELIGHT_VENDOR_ID, GS_USB_CANDLELIGHT_PRODUCT_ID)
                | (
                    GS_USB_CES_CANEXT_FD_VENDOR_ID,
                    GS_USB_CES_CANEXT_FD_PRODUCT_ID
                )
                | (
                    GS_USB_ABE_CANDEBUGGER_FD_VENDOR_ID,
                    GS_USB_ABE_CANDEBUGGER_FD_PRODUCT_ID
                )
        )
    }

    fn bus_number(info: &UsbDeviceInfo) -> u8 {
        #[cfg(target_os = "linux")]
        {
            info.busnum()
        }
        #[cfg(not(target_os = "linux"))]
        {
            info.bus_id().parse().unwrap_or(0)
        }
    }

    /// Scan for GS-USB devices
    ///
    /// Returns a list of all connected GS-USB compatible devices.
    pub fn scan() -> Result<Vec<GsUsb>> {
        let mut devices = Vec::new();

        for info in nusb::list_devices().wait()? {
            if Self::is_gs_usb_device(info.vendor_id(), info.product_id()) {
                let handle = match info.open().wait() {
                    Ok(handle) => handle,
                    Err(_) => continue,
                };
                devices.push(GsUsb::new(handle, info));
            }
        }

        Ok(devices)
    }

    /// Find a specific GS-USB device by numeric bus and address.
    /// `find_by_bus_id` is portable across platforms.
    pub fn find(bus: u8, address: u8) -> Result<Option<GsUsb>> {
        for info in nusb::list_devices().wait()? {
            if Self::bus_number(&info) != bus || info.device_address() != address {
                continue;
            }
            if Self::is_gs_usb_device(info.vendor_id(), info.product_id()) {
                let handle = info.open().wait()?;
                return Ok(Some(GsUsb::new(handle, info)));
            }
        }

        Ok(None)
    }

    /// Find a specific GS-USB device by OS bus identifier and address.
    pub fn find_by_bus_id(bus_id: &str, address: u8) -> Result<Option<GsUsb>> {
        for info in nusb::list_devices().wait()? {
            if info.bus_id() != bus_id || info.device_address() != address {
                continue;
            }
            if Self::is_gs_usb_device(info.vendor_id(), info.product_id()) {
                let handle = info.open().wait()?;
                return Ok(Some(GsUsb::new(handle, info)));
            }
        }
        Ok(None)
    }
}

impl std::fmt::Display for GsUsb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GS-USB {:04x}:{:04x} (bus {}, addr {})",
            self.usb_info.vendor_id(),
            self.usb_info.product_id(),
            self.bus_id(),
            self.address
        )
    }
}

impl std::fmt::Debug for GsUsb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GsUsb")
            .field("bus", &self.bus)
            .field("address", &self.address)
            .field("started", &self.started)
            .field("fd_mode", &self.fd_mode)
            .field("device_flags", &format_args!("0x{:08x}", self.device_flags))
            .finish()
    }
}

impl Drop for GsUsb {
    fn drop(&mut self) {
        if self.started {
            let _ = self.stop();
        }
    }
}
