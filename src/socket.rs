//! Async socket API backed by nusb.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nusb::transfer::Buffer;
use tokio::sync::{mpsc, oneshot, Notify};

use crate::can::CanFrame;
use crate::config::{Bitrate, CanMode, Config, OverflowPolicy};
use crate::constants::{CAN_ERR_FLAG, GS_CAN_FLAG_OVERFLOW, GS_USB_RX_ECHO_ID};
use crate::discovery::{scan_async, DiscoveredDevice};
use crate::frame::GsUsbFrame;
use crate::received::{ReceiveEvent, ReceivedFrame};
use crate::socket_error::{Error, Result};
use crate::usb_async::AsyncUsb;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub rx_frames: u64,
    pub rx_dropped: u64,
    pub malformed_frames: u64,
    pub tx_writes: u64,
    pub tx_echoes: u64,
    pub timeouts: u64,
    pub usb_errors: u64,
}

#[derive(Default)]
struct Counters {
    rx_frames: AtomicU64,
    rx_dropped: AtomicU64,
    malformed_frames: AtomicU64,
    tx_writes: AtomicU64,
    tx_echoes: AtomicU64,
    timeouts: AtomicU64,
    usb_errors: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> Stats {
        Stats {
            rx_frames: self.rx_frames.load(Ordering::Relaxed),
            rx_dropped: self.rx_dropped.load(Ordering::Relaxed),
            malformed_frames: self.malformed_frames.load(Ordering::Relaxed),
            tx_writes: self.tx_writes.load(Ordering::Relaxed),
            tx_echoes: self.tx_echoes.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            usb_errors: self.usb_errors.load(Ordering::Relaxed),
        }
    }
}

struct RxState {
    events: VecDeque<ReceiveEvent>,
    capacity: usize,
    policy: OverflowPolicy,
    closed: bool,
    failed: bool,
}

struct RxQueue {
    state: Mutex<RxState>,
    notify: Notify,
    actor_notify: Notify,
    receiver_alive: AtomicBool,
    counters: Arc<Counters>,
}

impl RxQueue {
    fn new(config: &Config, counters: Arc<Counters>) -> Self {
        Self {
            state: Mutex::new(RxState {
                events: VecDeque::with_capacity(config.rx_queue_capacity),
                capacity: config.rx_queue_capacity,
                policy: config.overflow_policy,
                closed: false,
                failed: false,
            }),
            notify: Notify::new(),
            actor_notify: Notify::new(),
            receiver_alive: AtomicBool::new(true),
            counters,
        }
    }

    fn push(&self, event: ReceiveEvent) {
        let mut state = self.state.lock().expect("RX queue lock poisoned");
        if state.closed || !self.receiver_alive.load(Ordering::Relaxed) {
            return;
        }
        if state.events.len() == state.capacity {
            self.counters.rx_dropped.fetch_add(1, Ordering::Relaxed);
            match state.policy {
                OverflowPolicy::DropNewest => return,
                OverflowPolicy::DropOldest => {
                    state.events.pop_front();
                }
            }
        }
        state.events.push_back(event);
        drop(state);
        self.notify.notify_one();
    }

    fn finish(&self, failed: bool) {
        let mut state = self.state.lock().expect("RX queue lock poisoned");
        state.closed = true;
        state.failed = failed;
        drop(state);
        self.notify.notify_waiters();
    }

    async fn pop(&self) -> Result<Option<ReceiveEvent>> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.state.lock().expect("RX queue lock poisoned");
                if let Some(event) = state.events.pop_front() {
                    return Ok(Some(event));
                }
                if state.closed {
                    return if state.failed {
                        Err(Error::Disconnected)
                    } else {
                        Ok(None)
                    };
                }
            }
            notified.await;
        }
    }
}

enum Command {
    Send(CanFrame, oneshot::Sender<Result<()>>),
    Close(oneshot::Sender<Result<()>>),
}

/// Cloneable handle for queued CAN transmissions.
#[derive(Clone)]
pub struct CanSender {
    commands: mpsc::Sender<Command>,
    fd_enabled: bool,
    counters: Arc<Counters>,
}

impl CanSender {
    /// Success confirms a complete USB OUT transfer. It does not confirm CAN bus acknowledgment.
    /// A cancelled call may still transmit if it was already queued.
    pub async fn send(&self, frame: CanFrame) -> Result<()> {
        if frame.is_fd() && !self.fd_enabled {
            return Err(Error::UnsupportedFeature("CAN FD is not enabled"));
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .send(Command::Send(frame, reply_tx))
            .await
            .map_err(|_| Error::QueueClosed)?;
        reply_rx.await.map_err(|_| Error::Disconnected)?
    }

    pub fn stats(&self) -> Stats {
        self.counters.snapshot()
    }

    pub async fn close(&self) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.commands
            .send(Command::Close(reply_tx))
            .await
            .map_err(|_| Error::QueueClosed)?;
        reply_rx.await.map_err(|_| Error::Disconnected)?
    }
}

/// Exclusive consumer of received frames and device events.
pub struct CanReceiver {
    queue: Arc<RxQueue>,
}

impl CanReceiver {
    /// Cancellation safe: queued data is removed only when this call returns.
    pub async fn recv(&mut self) -> Result<Option<ReceivedFrame>> {
        loop {
            match self.recv_event().await? {
                Some(ReceiveEvent::Frame(frame)) => return Ok(Some(frame)),
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }

    pub async fn recv_event(&mut self) -> Result<Option<ReceiveEvent>> {
        self.queue.pop().await
    }

    pub fn stats(&self) -> Stats {
        self.queue.counters.snapshot()
    }
}

impl Drop for CanReceiver {
    fn drop(&mut self) {
        self.queue.receiver_alive.store(false, Ordering::Relaxed);
        self.queue.actor_notify.notify_one();
    }
}

/// Configured asynchronous GS-USB CAN channel.
pub struct CanSocket {
    sender: CanSender,
    receiver: CanReceiver,
}

impl CanSocket {
    pub async fn scan() -> Result<Vec<DiscoveredDevice>> {
        scan_async().await
    }

    pub fn builder(device: DiscoveredDevice) -> CanSocketBuilder {
        CanSocketBuilder {
            device,
            config: Config::default(),
        }
    }

    pub fn split(self) -> (CanSender, CanReceiver) {
        (self.sender, self.receiver)
    }

    pub async fn send(&self, frame: CanFrame) -> Result<()> {
        self.sender.send(frame).await
    }

    pub async fn recv(&mut self) -> Result<Option<ReceivedFrame>> {
        self.receiver.recv().await
    }

    pub async fn recv_event(&mut self) -> Result<Option<ReceiveEvent>> {
        self.receiver.recv_event().await
    }

    pub fn stats(&self) -> Stats {
        self.sender.stats()
    }

    pub async fn close(self) -> Result<()> {
        self.sender.close().await
    }
}

pub struct CanSocketBuilder {
    device: DiscoveredDevice,
    config: Config,
}

impl CanSocketBuilder {
    pub fn channel(mut self, channel: u8) -> Self {
        self.config.channel = channel;
        self
    }

    pub fn nominal_bitrate(mut self, rate: Bitrate) -> Self {
        self.config.nominal_bitrate = rate;
        self
    }

    pub fn data_bitrate(mut self, rate: Bitrate) -> Self {
        self.config.data_bitrate = Some(rate);
        self
    }

    pub fn fd_enabled(mut self, yes: bool) -> Self {
        self.config.fd_enabled = yes;
        self
    }

    pub fn hardware_timestamps(mut self, yes: bool) -> Self {
        self.config.hardware_timestamps = yes;
        self
    }

    pub fn mode(mut self, mode: CanMode) -> Self {
        self.config.mode = mode;
        self
    }

    pub fn rx_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.rx_queue_capacity = capacity;
        self
    }

    pub fn tx_queue_capacity(mut self, capacity: usize) -> Self {
        self.config.tx_queue_capacity = capacity;
        self
    }

    pub fn overflow_policy(mut self, policy: OverflowPolicy) -> Self {
        self.config.overflow_policy = policy;
        self
    }

    pub fn command_timeout(mut self, timeout: Duration) -> Self {
        self.config.transfer_timeout = timeout;
        self
    }

    pub async fn connect(self) -> Result<CanSocket> {
        let usb = AsyncUsb::connect(self.device, &self.config).await?;
        let packet_size = usb.endpoint_in.max_packet_size();
        if packet_size == 0 {
            let _ = usb.stop().await;
            return Err(Error::Protocol("USB IN endpoint has zero packet size"));
        }
        let counters = Arc::new(Counters::default());
        let queue = Arc::new(RxQueue::new(&self.config, counters.clone()));
        let (commands, command_rx) = mpsc::channel(self.config.tx_queue_capacity);
        let sender = CanSender {
            commands,
            fd_enabled: self.config.fd_enabled,
            counters: counters.clone(),
        };
        let receiver = CanReceiver {
            queue: queue.clone(),
        };
        tokio::spawn(run_actor(
            usb,
            command_rx,
            queue,
            counters,
            self.config.transfer_timeout,
        ));
        Ok(CanSocket { sender, receiver })
    }
}

struct PendingWrite {
    reply: oneshot::Sender<Result<()>>,
    expected_len: usize,
    echo_id: u32,
    deadline: tokio::time::Instant,
    timed_out: bool,
}

fn write_result(
    status: std::result::Result<(), nusb::transfer::TransferError>,
    actual: usize,
    expected: usize,
    timed_out: bool,
) -> Result<()> {
    if timed_out {
        return Err(Error::Timeout);
    }
    status.map_err(Error::from_transfer)?;
    if actual != expected {
        return Err(Error::IncompleteWrite { expected, actual });
    }
    Ok(())
}

fn handle_input(
    bytes: &[u8],
    channel: u8,
    hw_timestamp: bool,
    queue: &RxQueue,
    counters: &Counters,
    pending_echoes: &mut VecDeque<u32>,
) {
    let is_fd = bytes
        .get(10)
        .is_some_and(|flags| flags & crate::constants::GS_CAN_FLAG_FD != 0);
    let raw = match GsUsbFrame::from_bytes(bytes, hw_timestamp, is_fd) {
        Ok(raw) => raw,
        Err(_) => {
            counters.malformed_frames.fetch_add(1, Ordering::Relaxed);
            return;
        }
    };
    if raw.channel != channel {
        return;
    }
    if raw.echo_id != GS_USB_RX_ECHO_ID {
        if let Some(index) = pending_echoes.iter().position(|id| *id == raw.echo_id) {
            pending_echoes.remove(index);
            counters.tx_echoes.fetch_add(1, Ordering::Relaxed);
        }
        return;
    }
    if raw.flags & GS_CAN_FLAG_OVERFLOW != 0 {
        queue.push(ReceiveEvent::Overflow { channel });
    }
    let timestamp = hw_timestamp.then_some(raw.timestamp_us);
    if raw.can_id & CAN_ERR_FLAG != 0 {
        let mut data = [0u8; 8];
        data.copy_from_slice(&raw.data[..8]);
        queue.push(ReceiveEvent::ErrorFrame {
            channel,
            can_id: raw.can_id,
            data,
            timestamp,
        });
        return;
    }
    match CanFrame::from_raw(&raw) {
        Ok(frame) => {
            counters.rx_frames.fetch_add(1, Ordering::Relaxed);
            queue.push(ReceiveEvent::Frame(ReceivedFrame {
                frame,
                channel,
                timestamp,
            }));
        }
        Err(_) => {
            counters.malformed_frames.fetch_add(1, Ordering::Relaxed);
        }
    }
}

async fn run_actor(
    mut usb: AsyncUsb,
    mut commands: mpsc::Receiver<Command>,
    queue: Arc<RxQueue>,
    counters: Arc<Counters>,
    timeout: Duration,
) {
    let packet_size = usb.endpoint_in.max_packet_size();
    let frame_size = GsUsbFrame::frame_size(usb.hardware_timestamps, usb.fd_enabled);
    let request_size = frame_size.div_ceil(packet_size) * packet_size;
    for _ in 0..4 {
        usb.endpoint_in.submit(Buffer::new(request_size));
    }
    let mut pending_echoes = VecDeque::new();
    let mut next_echo_id = 0u32;
    let mut pending_write: Option<PendingWrite> = None;
    let mut commands_closed = false;
    let mut receiver_closed = false;
    let mut failed = false;

    loop {
        let deadline = pending_write
            .as_ref()
            .map(|write| write.deadline)
            .unwrap_or_else(tokio::time::Instant::now);
        tokio::select! {
            command = commands.recv(), if !commands_closed && pending_write.is_none() => {
                match command {
                    Some(Command::Send(frame, reply)) => {
                        let result = frame
                            .to_raw(usb.channel, next_echo_id)
                            .and_then(|raw| raw.pack_for_tx(usb.fd_enabled).map_err(Error::from));
                        match result {
                            Ok(bytes) => {
                                let expected_len = bytes.len();
                                usb.endpoint_out.submit(bytes.into());
                                pending_write = Some(PendingWrite {
                                    reply,
                                    expected_len,
                                    echo_id: next_echo_id,
                                    deadline: tokio::time::Instant::now() + timeout,
                                    timed_out: false,
                                });
                                pending_echoes.push_back(next_echo_id);
                                if pending_echoes.len() > 256 {
                                    pending_echoes.pop_front();
                                }
                                next_echo_id = next_echo_id.wrapping_add(1);
                                if next_echo_id == GS_USB_RX_ECHO_ID {
                                    next_echo_id = 0;
                                }
                            }
                            Err(error) => { let _ = reply.send(Err(error)); }
                        }
                    }
                    Some(Command::Close(reply)) => {
                        let result = usb.stop().await;
                        let failed = result.is_err();
                        let _ = reply.send(result);
                        queue.finish(failed);
                        return;
                    }
                    None => commands_closed = true,
                }
            }
            completion = usb.endpoint_out.next_complete(), if pending_write.is_some() => {
                let write = pending_write.take().expect("OUT completion needs a pending write");
                let result = write_result(completion.status, completion.actual_len, write.expected_len, write.timed_out);
                if result.is_ok() {
                    counters.tx_writes.fetch_add(1, Ordering::Relaxed);
                } else {
                    counters.usb_errors.fetch_add(1, Ordering::Relaxed);
                    if let Some(index) = pending_echoes.iter().position(|id| *id == write.echo_id) {
                        pending_echoes.remove(index);
                    }
                    if matches!(result, Err(Error::Timeout)) {
                        counters.timeouts.fetch_add(1, Ordering::Relaxed);
                    }
                }
                let _ = write.reply.send(result);
            }
            _ = tokio::time::sleep_until(deadline), if pending_write.as_ref().is_some_and(|write| !write.timed_out) => {
                usb.endpoint_out.cancel_all();
                pending_write.as_mut().expect("timeout needs a pending write").timed_out = true;
            }
            completion = usb.endpoint_in.next_complete() => {
                match completion.status {
                    Ok(()) => {
                        let len = completion.actual_len;
                        handle_input(&completion.buffer[..len], usb.channel, usb.hardware_timestamps, &queue, &counters, &mut pending_echoes);
                        usb.endpoint_in.submit(completion.buffer);
                    }
                    Err(_) => {
                        counters.usb_errors.fetch_add(1, Ordering::Relaxed);
                        failed = true;
                        break;
                    }
                }
            }
            _ = queue.actor_notify.notified(), if !receiver_closed => {
                receiver_closed = !queue.receiver_alive.load(Ordering::Relaxed);
            }
        }
        if commands_closed && receiver_closed && pending_write.is_none() {
            break;
        }
    }
    let _ = usb.stop().await;
    queue.finish(failed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::can::CanId;

    #[tokio::test]
    async fn rx_overflow_policy_and_cancellation() {
        let config = Config {
            rx_queue_capacity: 1,
            overflow_policy: OverflowPolicy::DropOldest,
            ..Config::default()
        };
        let counters = Arc::new(Counters::default());
        let queue = RxQueue::new(&config, counters.clone());
        let first = CanFrame::classic(CanId::standard(1).unwrap(), &[1]).unwrap();
        let second = CanFrame::classic(CanId::standard(2).unwrap(), &[2]).unwrap();
        queue.push(ReceiveEvent::Frame(ReceivedFrame {
            frame: first,
            channel: 0,
            timestamp: None,
        }));
        queue.push(ReceiveEvent::Frame(ReceivedFrame {
            frame: second.clone(),
            channel: 0,
            timestamp: None,
        }));
        assert_eq!(counters.snapshot().rx_dropped, 1);
        let popped = queue.pop().await.unwrap().unwrap();
        assert!(matches!(popped, ReceiveEvent::Frame(v) if v.frame == second));
        {
            let pending = queue.pop();
            tokio::pin!(pending);
            assert!(tokio::time::timeout(Duration::from_millis(1), &mut pending)
                .await
                .is_err());
        }
        queue.push(ReceiveEvent::Frame(ReceivedFrame {
            frame: second.clone(),
            channel: 0,
            timestamp: None,
        }));
        assert!(
            matches!(queue.pop().await.unwrap(), Some(ReceiveEvent::Frame(v)) if v.frame == second)
        );
        queue.finish(false);
        assert!(queue.pop().await.unwrap().is_none());
    }

    #[test]
    fn routes_echo_and_data_separately() {
        let config = Config::default();
        let counters = Arc::new(Counters::default());
        let queue = RxQueue::new(&config, counters.clone());
        let mut echoes = VecDeque::from([7]);
        let frame = CanFrame::classic(CanId::standard(0x123).unwrap(), &[1, 2]).unwrap();
        let echo = frame.to_raw(0, 7).unwrap();
        handle_input(
            &echo.pack(false, false),
            0,
            false,
            &queue,
            &counters,
            &mut echoes,
        );
        assert_eq!(counters.snapshot().tx_echoes, 1);
        let mut incoming = frame.to_raw(0, GS_USB_RX_ECHO_ID).unwrap();
        incoming.echo_id = GS_USB_RX_ECHO_ID;
        handle_input(
            &incoming.pack(false, false),
            0,
            false,
            &queue,
            &counters,
            &mut echoes,
        );
        assert_eq!(counters.snapshot().rx_frames, 1);
    }

    #[test]
    fn rejects_short_writes_and_reports_timeouts() {
        assert!(matches!(
            write_result(Ok(()), 19, 20, false),
            Err(Error::IncompleteWrite {
                expected: 20,
                actual: 19
            })
        ));
        assert!(matches!(
            write_result(Ok(()), 20, 20, true),
            Err(Error::Timeout)
        ));
        assert!(matches!(
            write_result(
                Err(nusb::transfer::TransferError::Disconnected),
                0,
                20,
                false
            ),
            Err(Error::Disconnected)
        ));
    }

    #[tokio::test]
    async fn routes_overflow_error_and_malformed_input() {
        let config = Config::default();
        let counters = Arc::new(Counters::default());
        let queue = RxQueue::new(&config, counters.clone());
        let mut echoes = VecDeque::new();
        handle_input(&[0; 11], 0, false, &queue, &counters, &mut echoes);
        assert_eq!(counters.snapshot().malformed_frames, 1);

        let mut raw = CanFrame::classic(CanId::standard(0x123).unwrap(), &[1; 8])
            .unwrap()
            .to_raw(0, GS_USB_RX_ECHO_ID)
            .unwrap();
        raw.flags |= GS_CAN_FLAG_OVERFLOW;
        raw.can_id |= CAN_ERR_FLAG;
        handle_input(
            &raw.pack(false, false),
            0,
            false,
            &queue,
            &counters,
            &mut echoes,
        );
        assert!(matches!(
            queue.pop().await.unwrap(),
            Some(ReceiveEvent::Overflow { channel: 0 })
        ));
        assert!(matches!(
            queue.pop().await.unwrap(),
            Some(ReceiveEvent::ErrorFrame { channel: 0, .. })
        ));
    }

    #[tokio::test]
    async fn disconnected_receiver_reports_error() {
        let counters = Arc::new(Counters::default());
        let queue = RxQueue::new(&Config::default(), counters);
        queue.finish(true);
        assert!(matches!(queue.pop().await, Err(Error::Disconnected)));
    }
}
