//! Bounded adapter loopback check. Use --fd, --timestamp, --matrix, or --preflight.

use std::time::Duration;

use gsusbio::{Bitrate, CanFrame, CanId, CanMode, CanSocket, Error, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let matrix = std::env::args().any(|arg| arg == "--matrix");
    let fd = matrix || std::env::args().any(|arg| arg == "--fd");
    let timestamp = std::env::args().any(|arg| arg == "--timestamp");
    let preflight = std::env::args().any(|arg| arg == "--preflight");
    let device = CanSocket::scan()
        .await?
        .into_iter()
        .next()
        .ok_or(Error::DeviceNotFound)?;
    if preflight {
        let invalid_rate = CanSocket::builder(device.clone())
            .nominal_bitrate(Bitrate::Custom(123_456_789))
            .connect()
            .await;
        assert!(matches!(invalid_rate, Err(Error::UnsupportedFeature(_))));
        let invalid_channel = CanSocket::builder(device.clone())
            .channel(254)
            .connect()
            .await;
        assert!(matches!(
            invalid_channel,
            Err(Error::InvalidConfiguration(_))
        ));
    }
    let mut builder = CanSocket::builder(device)
        .nominal_bitrate(Bitrate::Kbps500)
        .mode(CanMode::Loopback)
        .hardware_timestamps(timestamp);
    if fd {
        builder = builder.data_bitrate(Bitrate::Mbps2).fd_enabled(true);
    }
    let socket = builder.connect().await?;
    let (tx, mut rx) = socket.split();
    let frames = if matrix {
        let mut frames = vec![
            CanFrame::classic(CanId::standard(0x5a3)?, &[0x55, 0xaa])?,
            CanFrame::classic(CanId::extended(0x18ff_50e5)?, &[1, 2, 3])?,
        ];
        for length in [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64] {
            for id in [CanId::standard(0x123)?, CanId::extended(0x18ff_50e5)?] {
                for brs in [false, true] {
                    frames.push(CanFrame::fd(id, &vec![0x55; length], brs)?);
                }
            }
        }
        frames
    } else if fd {
        vec![CanFrame::fd(CanId::standard(0x5a3)?, &[0x55; 12], true)?]
    } else {
        vec![CanFrame::classic(CanId::standard(0x5a3)?, &[0x55, 0xaa])?]
    };
    let count = frames.len();
    for sent in frames {
        tx.send(sent.clone()).await?;
        let received = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .map_err(|_| Error::Timeout)??
            .ok_or(Error::Disconnected)?;
        assert_eq!(received.frame.id(), sent.id());
        assert_eq!(received.frame.data(), sent.data());
        assert_eq!(received.frame.dlc(), sent.dlc());
        assert_eq!(received.frame.brs(), sent.brs());
        if timestamp {
            assert!(received.timestamp.is_some());
        }
    }
    println!("Loopback checked {count} frames");
    tx.close().await
}
