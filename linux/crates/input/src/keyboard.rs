//! The virtual USB keyboard as a session [`Driver`].
//!
//! `key()` only queues the event. A writer thread sends queued events at USB
//! polling boundaries (1000 Hz by default), grouping them into reports the
//! way a real keyboard does, so event timing and grouping look like hardware.

use std::collections::{HashSet, VecDeque};
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use humanize_core::{Driver, DriverError, KeyCode};

use crate::frame::{KeyEvent, take_frame};
use crate::hid;
use crate::uinput::{Setup, UinputDevice};

/// How the virtual keyboard presents itself to the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceIdentity {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
    /// Physical path string (`phys`); real USB keyboards have one, uinput
    /// devices default to none.
    pub phys: Option<String>,
    /// USB polling rate: 125, 250, 500 or 1000 Hz are typical.
    pub polling_hz: u32,
}

impl Default for DeviceIdentity {
    /// A generic USB keyboard, as many low-cost boards identify themselves.
    fn default() -> Self {
        Self {
            name: "USB Keyboard".into(),
            vendor: 0x1c4f,
            product: 0x0002,
            version: 0x0110,
            phys: None,
            polling_hz: 1000,
        }
    }
}

/// Time for the compositor to notice a new input device before it is used.
const SETTLE: Duration = Duration::from_millis(500);

pub struct UinputKeyboard {
    tx: Option<Sender<KeyEvent>>,
    failure: Arc<Mutex<Option<String>>>,
    writer: Option<JoinHandle<()>>,
    sysname: String,
}

impl UinputKeyboard {
    /// Creates the device and waits briefly so the compositor picks it up.
    pub fn create(identity: &DeviceIdentity) -> io::Result<Self> {
        let device = UinputDevice::create(&Setup {
            name: &identity.name,
            vendor: identity.vendor,
            product: identity.product,
            version: identity.version,
            phys: identity.phys.as_deref(),
        })?;
        let sysname = device.sysname().to_owned();
        let period = Duration::from_nanos(1_000_000_000 / u64::from(identity.polling_hz.clamp(125, 8000)));
        let (tx, rx) = mpsc::channel();
        let failure = Arc::new(Mutex::new(None));
        let writer_failure = Arc::clone(&failure);
        let writer = thread::Builder::new()
            .name("uinput-writer".into())
            .spawn(move || write_loop(device, rx, period, &writer_failure))?;
        thread::sleep(SETTLE);
        Ok(Self { tx: Some(tx), failure, writer: Some(writer), sysname })
    }

    /// Kernel name of the device (`inputN`), for finding it in sysfs.
    pub fn sysname(&self) -> &str {
        &self.sysname
    }
}

impl Driver for UinputKeyboard {
    fn key(&mut self, key: KeyCode, down: bool) -> Result<(), DriverError> {
        if let Some(message) = self.failure.lock().unwrap_or_else(PoisonError::into_inner).clone() {
            return Err(DriverError(message));
        }
        if hid::scan_code(key).is_none() {
            return Err(DriverError(format!("the virtual keyboard has no key {}", key.0)));
        }
        self.tx
            .as_ref()
            .and_then(|tx| tx.send(KeyEvent { key, down }).ok())
            .ok_or_else(|| DriverError("the virtual keyboard has stopped".into()))
    }
}

impl Drop for UinputKeyboard {
    fn drop(&mut self) {
        // Closing the channel tells the writer to lift held keys and exit.
        self.tx.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn write_loop(
    mut device: UinputDevice,
    rx: Receiver<KeyEvent>,
    period: Duration,
    failure: &Mutex<Option<String>>,
) {
    let epoch = Instant::now();
    // u128 nanoseconds: no overflow however long the device lives.
    let period_ns = period.as_nanos().max(1);
    let next_tick = |now: Instant| {
        let next = ((now - epoch).as_nanos() / period_ns + 1) * period_ns;
        epoch + Duration::from_nanos(u64::try_from(next).unwrap_or(u64::MAX))
    };
    let mut pending: VecDeque<KeyEvent> = VecDeque::new();
    let mut held: HashSet<KeyCode> = HashSet::new();

    let send = |device: &mut UinputDevice, frame: Vec<KeyEvent>, held: &mut HashSet<KeyCode>| {
        for e in &frame {
            if e.down {
                held.insert(e.key);
            } else {
                held.remove(&e.key);
            }
        }
        device.write_frame(&frame)
    };

    // Idle: block until something arrives (or the driver is dropped).
    'outer: while let Ok(event) = rx.recv() {
        pending.push_back(event);
        while !pending.is_empty() {
            let tick = next_tick(Instant::now());
            thread::sleep(tick.saturating_duration_since(Instant::now()));
            pending.extend(rx.try_iter());
            let frame = take_frame(&mut pending);
            if let Err(e) = send(&mut device, frame, &mut held) {
                *failure.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(format!("uinput write failed: {e}"));
                break 'outer;
            }
        }
    }

    // Never leave a key down when the device goes away.
    let mut release: VecDeque<KeyEvent> = held.iter().map(|&key| KeyEvent { key, down: false }).collect();
    while !release.is_empty() {
        let frame = take_frame(&mut release);
        let _ = device.write_frame(&frame);
    }
}
