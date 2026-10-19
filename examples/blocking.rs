use std::time::Duration;

use gsusbio::{blocking::CanSocket, Bitrate, CanFrame, CanId, CanMode, Error, Result};

fn main() -> Result<()> {
    let device = CanSocket::scan()?
        .into_iter()
        .next()
        .ok_or(Error::DeviceNotFound)?;
    let mut socket = CanSocket::builder(device)
        .nominal_bitrate(Bitrate::Kbps500)
        .mode(CanMode::Loopback)
        .hardware_timestamps(true)
        .timeout(Duration::from_secs(2))
        .connect()?;
    let sent = CanFrame::classic(CanId::standard(0x123)?, &[1, 2, 3])?;
    socket.send(sent.clone())?;
    let received = socket.recv_with_metadata()?;
    if received.frame != sent || received.timestamp.is_none() {
        return Err(Error::Protocol(
            "loopback frame did not match transmitted frame",
        ));
    }
    println!("Loopback received: {received:?}");
    socket.close()
}
