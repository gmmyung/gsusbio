//! USB adapter discovery and selection.

#[cfg(feature = "blocking")]
use nusb::MaybeFuture;

use crate::socket_error::Result;

#[derive(Debug, Clone)]
pub struct DiscoveredDevice {
    pub(crate) inner: nusb::DeviceInfo,
}

impl DiscoveredDevice {
    pub fn vendor_id(&self) -> u16 {
        self.inner.vendor_id()
    }

    pub fn product_id(&self) -> u16 {
        self.inner.product_id()
    }

    pub fn bus_id(&self) -> &str {
        self.inner.bus_id()
    }

    pub fn address(&self) -> u8 {
        self.inner.device_address()
    }

    #[cfg(feature = "tokio")]
    pub async fn serial_number(&self) -> Result<Option<String>> {
        Ok(self.inner.get_serial_number_string().await?)
    }

    #[cfg(feature = "blocking")]
    pub fn serial_number_blocking(&self) -> Result<Option<String>> {
        Ok(self.inner.get_serial_number_string().wait()?)
    }
}

fn collect(infos: impl IntoIterator<Item = nusb::DeviceInfo>) -> Vec<DiscoveredDevice> {
    infos
        .into_iter()
        .filter(|info| is_gs_usb_device(info.vendor_id(), info.product_id()))
        .map(|inner| DiscoveredDevice { inner })
        .collect()
}

fn is_gs_usb_device(vendor_id: u16, product_id: u16) -> bool {
    matches!(
        (vendor_id, product_id),
        (0x1d50, 0x606f) | (0x1209, 0x2323) | (0x1cd2, 0x606f) | (0x16d0, 0x10b8)
    )
}

#[cfg(feature = "tokio")]
pub(crate) async fn scan_async() -> Result<Vec<DiscoveredDevice>> {
    Ok(collect(nusb::list_devices().await?))
}

#[cfg(feature = "blocking")]
pub(crate) fn scan_blocking() -> Result<Vec<DiscoveredDevice>> {
    Ok(collect(nusb::list_devices().wait()?))
}
