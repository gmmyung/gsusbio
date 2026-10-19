use gsusbio::{Bitrate, CanFrame, CanId, CanSocket, Error, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let device = CanSocket::scan()
        .await?
        .into_iter()
        .next()
        .ok_or(Error::DeviceNotFound)?;
    let socket = CanSocket::builder(device)
        .nominal_bitrate(Bitrate::Kbps500)
        .data_bitrate(Bitrate::Mbps2)
        .fd_enabled(true)
        .connect()
        .await?;
    let (tx, mut rx) = socket.split();
    tx.send(CanFrame::fd(CanId::standard(0x123)?, &[0x55; 12], true)?)
        .await?;
    if let Some(received) = rx.recv().await? {
        println!("{:?}", received);
    }
    tx.close().await
}
