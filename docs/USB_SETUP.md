# USB setup

`gsusbio` uses [nusb 0.2.8](https://docs.rs/nusb/0.2.8/nusb/) for both async and blocking operations. nusb is a Rust USB library and does not depend on libusb. There is no libusb package or native libusb development header to install for this crate.

## Linux

nusb uses the kernel's usbfs interface. Your user needs write access to the adapter's `/dev/bus/usb/...` node. On a system with systemd logind, a narrow udev rule for a `1d50:606f` adapter is:

```udev
SUBSYSTEM=="usb", ATTR{idVendor}=="1d50", ATTR{idProduct}=="606f", TAG+="uaccess"
```

Place it in `/etc/udev/rules.d/70-gsusbio.rules`, reload rules with `sudo udevadm control --reload-rules`, then reconnect the adapter. Adjust the IDs for your hardware. On systems without logind, configure a suitable device group and use `MODE="0660", GROUP="your-usb-group"` in the rule instead.

The Linux `gs_usb` kernel driver may own the interface. The socket calls nusb's `detach_and_claim_interface(0)` when connecting; the kernel network interface will no longer handle that USB interface while the socket has it claimed. Close the socket to reset the selected CAN channel.

## Windows

nusb uses WinUSB. Associate the adapter's USB interface with WinUSB, using firmware descriptors where available or a driver installation tool such as Zadig. The socket cannot open an interface still bound to an incompatible driver. See [nusb's Windows setup](https://docs.rs/nusb/0.2.8/nusb/#windows).

## macOS

nusb uses IOKit. USB access normally needs no special permission setup. A device interface already claimed by a kernel driver may be unavailable. See [nusb's macOS notes](https://docs.rs/nusb/0.2.8/nusb/#macos).

## Diagnose discovery and connection

`CanSocket::scan()` lists only the USB IDs in the [README](../README.md#features). An empty list means no supported device was discovered; it does not mean that libusb is missing. If discovery succeeds but `connect()` fails, check device permissions, driver binding, and whether another process has claimed the interface. An unsupported bitrate, mode, or FD request is reported separately by `connect()`.
