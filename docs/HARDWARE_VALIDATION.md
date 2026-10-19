# Hardware validation

The repeatable single adapter check is `examples/loopback.rs`. It starts a channel in adapter loopback mode, sends frames, compares received identifiers, data, DLC, and BRS flags, then closes the channel.

```sh
cargo run --example loopback -- --matrix --timestamp --preflight
```

| Flag | Check |
| --- | --- |
| `--fd` | One CAN FD frame. |
| `--matrix` | Two Classic CAN frames plus every FD DLC from 0 through 15 with standard and extended IDs and BRS on and off. Implies `--fd`. |
| `--timestamp` | Request and assert adapter hardware timestamps. |
| `--preflight` | Reject an impossible bitrate and invalid channel before successfully connecting. |

On 2026-10-19, a connected `1d50:606f` GS-USB adapter passed the async `--matrix --timestamp --preflight` check: 66 frames total, including mixed Classic/FD traffic. The blocking Classic CAN example also passed loopback with timestamps:

```sh
cargo run --features blocking --example blocking
```

Software checks passed on the development Linux host: all feature tests, no default feature tests, Clippy with warnings denied, and package creation. This records one adapter and one host; it is not a claim of cross platform hardware validation.

Before treating a platform or adapter family as hardware verified, test a real CAN peer at matching bitrates, disconnected device behavior, and a multi channel adapter. Repeat the platform's USB permission and driver setup from [USB setup](USB_SETUP.md).
