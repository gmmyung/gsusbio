//! Cross-platform GS-USB Classic CAN and CAN FD sockets backed by nusb.
//!
//! The default `tokio` feature provides `CanSocket` with a cloneable sender
//! and an exclusive receiver. Enable `blocking` for `blocking::CanSocket`.
//! Both APIs use nusb.
//!
//! Discover a supported adapter, configure a channel, and connect:
//!
//! ```no_run
//! # #[cfg(feature = "tokio")]
//! # {
//! use gsusbio::{Bitrate, CanFrame, CanId, CanSocket, Error, Result};
//!
//! # async fn example() -> Result<()> {
//! let device = CanSocket::scan().await?.into_iter().next().ok_or(Error::DeviceNotFound)?;
//! let socket = CanSocket::builder(device)
//!     .nominal_bitrate(Bitrate::Kbps500)
//!     .connect()
//!     .await?;
//! let (tx, mut rx) = socket.split();
//! tx.send(CanFrame::classic(CanId::standard(0x123)?, &[1, 2, 3])?).await?;
//! if let Some(received) = rx.recv().await? {
//!     println!("{:?}", received.frame);
//! }
//! # Ok(())
//! # }
//! # }
//! ```
//!
//! `send()` completes after a USB write, without waiting for CAN bus
//! acknowledgment. `recv_event()` exposes adapter overflow and CAN error
//! frames in addition to data frames. See the repository's
//! [API guide](https://github.com/gmmyung/gsusbio/blob/master/docs/API.md)
//! and [USB setup](https://github.com/gmmyung/gsusbio/blob/master/docs/USB_SETUP.md)
//! for configuration and device access details.

mod can;
#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod constants;
#[cfg(feature = "blocking")]
#[allow(dead_code)]
mod device;
#[cfg(any(feature = "tokio", feature = "blocking"))]
mod discovery;
#[allow(dead_code)]
mod error;
#[allow(dead_code)]
mod frame;
mod received;
mod socket_error;
#[allow(dead_code)]
mod structures;
#[allow(dead_code)]
mod timing;

#[cfg(feature = "tokio")]
mod socket;
#[cfg(feature = "tokio")]
mod usb_async;

#[cfg(feature = "blocking")]
pub mod blocking;

pub use can::{CanFrame, CanId};
pub use config::{Bitrate, CanMode, OverflowPolicy};
#[cfg(any(feature = "tokio", feature = "blocking"))]
pub use discovery::DiscoveredDevice;
pub use received::{ReceiveEvent, ReceivedFrame};
pub use socket_error::{Error, Result};

#[cfg(feature = "tokio")]
pub use socket::{CanReceiver, CanSender, CanSocket, CanSocketBuilder, Stats};
