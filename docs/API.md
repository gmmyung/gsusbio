# API guide

`gsusbio` provides one configured CAN channel per socket. The async API is at the crate root with the `tokio` feature. The synchronous API is in `gsusbio::blocking` with the `blocking` feature. Both use nusb for USB transfers.

## Find and connect

`CanSocket::scan()` returns supported USB devices. Inspect `vendor_id()`, `product_id()`, `bus_id()`, and `address()` to choose an adapter. `serial_number().await` is available with Tokio; `serial_number_blocking()` is available with the blocking feature.

Pass the selected `DiscoveredDevice` to `CanSocket::builder(device)`. The defaults are channel `0`, `Bitrate::Kbps500`, `CanMode::Normal`, and FD and hardware timestamps off. `connect()` validates the configuration and adapter capability before starting the channel.

| Builder method | Effect |
| --- | --- |
| `channel(u8)` | Select a channel advertised by the adapter. |
| `nominal_bitrate(Bitrate)` | Set the CAN arbitration bitrate. |
| `data_bitrate(Bitrate)` | Set the CAN FD data bitrate; requires `fd_enabled(true)`. |
| `fd_enabled(bool)` | Enable CAN FD on a capable adapter; requires a data bitrate. |
| `mode(CanMode)` | Select `Normal`, `ListenOnly`, `Loopback`, or `OneShot`. |
| `hardware_timestamps(bool)` | Request device timestamps when supported. |

Async builders also expose `rx_queue_capacity(usize)` (default 1024), `tx_queue_capacity(usize)` (default 256), `overflow_policy(OverflowPolicy)` (default `DropNewest`), and `command_timeout(Duration)` (default 2 seconds). Capacities and timeout must be positive. Blocking builders expose `timeout(Duration)` (default 2 seconds) for receive operations.

`Bitrate` has named rates from `Kbps10` to `Mbps5`, plus `Custom(u32)`. The timing calculator requests an exact rate from the adapter's advertised limits. Unsupported requests return an error during `connect()`.

## Frames

Create an identifier with `CanId::standard(u16)` for 11 bit IDs or `CanId::extended(u32)` for 29 bit IDs. Both constructors check the range.

| Constructor | Payload or DLC | Notes |
| --- | --- | --- |
| `CanFrame::classic(id, data)` | 0–8 bytes | Classic CAN data frame. |
| `CanFrame::remote(id, dlc)` | DLC 0–8 | Classic CAN remote request. |
| `CanFrame::fd(id, data, brs)` | 0–8, 12, 16, 20, 24, 32, 48, or 64 bytes | Requires the `can-fd` feature and FD channel mode. |

Inspect frames with `id()`, `data()`, `dlc()`, `is_remote()`, `is_fd()`, `brs()`, and `esi()`. ESI is present only on received FD frames. The constructors reject invalid lengths and identifiers.

## Send and receive

`CanSocket::send(&self, frame)` and `CanSender::send(&self, frame)` finish when the USB OUT transfer completes. Success does not mean another CAN node acknowledged the frame. A cancelled async call can leave an already queued frame in flight. `split()` returns a cloneable sender and an exclusive receiver for concurrent tasks.

`CanSocket::recv(&mut self)` and `CanReceiver::recv(&mut self)` yield `Result<Option<ReceivedFrame>>`. `None` means the socket closed normally; a USB failure returns an error. `recv()` skips device events. Use `recv_event()` to receive all events in order:

| `ReceiveEvent` | Contents |
| --- | --- |
| `Frame(ReceivedFrame)` | Validated CAN frame, channel, optional timestamp. |
| `ErrorFrame` | Raw CAN error identifier, eight data bytes, channel, optional timestamp. |
| `Overflow` | Adapter overflow indication and channel. |

`ReceivedFrame::timestamp` is the device's 32 bit microsecond counter when requested and supported. It may wrap and is not a host wall clock time.

Async RX events go into a bounded queue. When full, `DropNewest` discards the incoming event; `DropOldest` removes the oldest queued event. `recv()` is cancellation safe. The blocking API has `send(&mut self, frame)`, `recv(&mut self) -> Result<CanFrame>`, `recv_with_metadata(&mut self) -> Result<ReceivedFrame>`, and `recv_event(&mut self) -> Result<ReceiveEvent>`. A blocking receive timeout returns `Error::Timeout`.

## Closing and statistics

Call `socket.close().await` or, after `split()`, `tx.close().await` to process earlier queued sends and reset the CAN channel. Blocking sockets use `socket.close()`. Handle an error from `close()` if the device has disconnected.

Async `stats()` returns a snapshot:

| Field | Meaning |
| --- | --- |
| `rx_frames` | Valid data frames received from USB. |
| `rx_dropped` | Events discarded because the RX queue was full. |
| `malformed_frames` | Invalid USB frame payloads or CAN frame data. |
| `tx_writes` | Complete USB OUT transfers. |
| `tx_echoes` | Matching adapter TX echoes. |
| `timeouts` | Timed out USB writes. |
| `usb_errors` | USB read or write failures. |

`Error` distinguishes invalid frames and configuration, unsupported features, timeouts, disconnection, incomplete writes, protocol errors, and underlying nusb errors. Match variants when your application needs different recovery behavior.
