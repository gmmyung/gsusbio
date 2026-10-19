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
        .connect()
        .await?;
    let (tx, mut rx) = socket.split();
    tx.send(CanFrame::classic(CanId::standard(0x123)?, &[1, 2, 3])?)
        .await?;
    if let Some(received) = rx.recv().await? {
        println!("{:?}", received);
    }
    tx.close().await
}
