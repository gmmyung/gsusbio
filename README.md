# gsusbio

Rust sockets for GS-USB Classic CAN and CAN FD adapters on Linux, macOS, and Windows. The USB backend is [nusb](https://docs.rs/nusb/0.2.8/nusb/). The default API uses Tokio; an optional blocking API uses the same backend.

**No libusb installation is required.** nusb uses each operating system's USB interface. USB device permissions or a suitable driver may still be needed; see [USB setup](docs/USB_SETUP.md).

## Quick start

Add the repository to your application:

```toml
[dependencies]
gsusbio = { git = "https://github.com/gmmyung/gsusbio" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use gsusbio::{Bitrate, CanFrame, CanId, CanSocket, Error, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let device = CanSocket::scan().await?
        .into_iter()
        .next()
        .ok_or(Error::DeviceNotFound)?;

    let socket = CanSocket::builder(device)
        .nominal_bitrate(Bitrate::Kbps500)
        .connect()
        .await?;

    let (tx, mut rx) = socket.split();
    tx.send(CanFrame::classic(CanId::standard(0x123)?, &[1, 2, 3])?).await?;

    if let Some(received) = rx.recv().await? {
        println!("{:?}", received.frame);
    }
    tx.close().await
}
```

This example waits for a frame from the CAN bus. For a single adapter with no peer, run the [loopback example](#examples-and-validation). If several adapters are connected, select one by USB ID, bus, address, or serial number.

## Features

| Cargo feature | Provides |
| --- | --- |
| `tokio` (default) | Async socket, split sender and receiver, bounded queues |
| `can-fd` (default) | CAN FD frames and data phase configuration |
| `blocking` | Synchronous socket using nusb directly |
| `default-features = false` | Runtime free Classic CAN frame model |

For a blocking application without Tokio:

```toml
[dependencies]
gsusbio = { git = "https://github.com/gmmyung/gsusbio", default-features = false, features = ["blocking"] }
```

Use `gsusbio::blocking::CanSocket`; see [the blocking example](examples/blocking.rs). Add `can-fd` to the feature list when you need FD frames.

The scanner currently recognizes these USB IDs:

| Adapter family | VID:PID |
| --- | --- |
| GS-USB / candleLight | `1d50:606f` |
| candleLight | `1209:2323` |
| CES CANext FD | `1cd2:606f` |
| ABE CANdebugger FD | `16d0:10b8` |

Capabilities vary by firmware. `connect()` checks channel count, requested modes, FD support, and bit timing against the selected device.

## CAN FD

Enable FD on the channel and set a data bitrate explicitly:

```rust
# use gsusbio::{Bitrate, CanFrame, CanId, CanSocket, DiscoveredDevice, Result};
# async fn example(device: DiscoveredDevice) -> Result<()> {
let socket = CanSocket::builder(device)
    .nominal_bitrate(Bitrate::Kbps500)
    .data_bitrate(Bitrate::Mbps2)
    .fd_enabled(true)
    .connect()
    .await?;
socket.send(CanFrame::fd(CanId::extended(0x18ff_50e5)?, &[0x55; 12], true)?).await?;
socket.close().await
# }
```

The last argument to `CanFrame::fd` enables bit rate switching. Valid FD payload lengths are 0 through 8, then 12, 16, 20, 24, 32, 48, and 64 bytes. `Bitrate::Custom(bps)` requests an exact bitrate when the adapter's timing limits allow it.

## Behavior

- `send()` completes after a full USB OUT transfer. It does not indicate CAN bus acknowledgment. A cancelled send already in the queue may still transmit.
- `recv()` returns data frames. `recv_event()` also exposes CAN error frames and adapter overflow indications. The async receiver is cancellation safe and yields `Ok(None)` after an orderly close.
- `CanSender` can be cloned for concurrent producers. A socket has one `CanReceiver`. Configure bounded RX and TX queues on the builder; RX overflow defaults to `DropNewest`.
- `hardware_timestamps(true)` requests adapter timestamps. `ReceivedFrame::timestamp` is an optional 32 bit microsecond counter that can wrap.
- `close()` processes earlier queued sends, resets the selected CAN channel, and ends the receive stream. The blocking API exposes `recv_with_metadata()` and a configurable timeout.

See the [API guide](docs/API.md) for the frame model, configuration defaults, events, statistics, and error behavior.

## Examples and validation

```sh
cargo run --example loopback -- --matrix --timestamp --preflight
cargo run --example classic
cargo run --example fd
cargo run --features blocking --example blocking
cargo test --all-features --all-targets
cargo test --no-default-features
```

`loopback` requires an adapter that supports loopback and, with `--matrix`, CAN FD. The `classic` and `fd` examples wait for traffic from a peer. Hardware coverage and remaining checks are recorded in [hardware validation](docs/HARDWARE_VALIDATION.md).

## Development

The minimum declared Rust version is 1.85. Run formatting, Clippy, tests, and docs before publishing changes:

```sh
cargo fmt --all -- --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test --all-features --all-targets
cargo test --no-default-features
cargo doc --all-features --no-deps
```
