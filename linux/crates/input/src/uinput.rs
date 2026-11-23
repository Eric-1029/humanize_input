//! Minimal raw uinput keyboard device.
//!
//! Done with plain ioctls instead of a wrapper crate so the device can
//! declare everything a real keyboard declares: keys, `MSC_SCAN`, the three
//! lock LEDs and `EV_REP`.

use std::ffi::CString;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use crate::frame::KeyEvent;
use crate::hid;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_MSC: u16 = 0x04;
const EV_LED: u16 = 0x11;
const EV_REP: u16 = 0x14;
const SYN_REPORT: u16 = 0;
const MSC_SCAN: u16 = 0x04;
const LED_NUML: u16 = 0;
const LED_CAPSL: u16 = 1;
const LED_SCROLLL: u16 = 2;
pub(crate) const BUS_USB: u16 = 0x03;

const UINPUT_MAX_NAME_SIZE: usize = 80;

#[repr(C)]
struct InputId {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
}

#[repr(C)]
struct UinputSetup {
    id: InputId,
    name: [u8; UINPUT_MAX_NAME_SIZE],
    ff_effects_max: u32,
}

#[repr(C)]
struct RawEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

const fn ioc(dir: u64, nr: u64, size: usize) -> u64 {
    (dir << 30) | ((size as u64) << 16) | ((b'U' as u64) << 8) | nr
}
const IOC_NONE: u64 = 0;
const IOC_WRITE: u64 = 1;
const IOC_READ: u64 = 2;
const UI_DEV_CREATE: u64 = ioc(IOC_NONE, 1, 0);
const UI_DEV_DESTROY: u64 = ioc(IOC_NONE, 2, 0);
const UI_DEV_SETUP: u64 = ioc(IOC_WRITE, 3, size_of::<UinputSetup>());
const UI_SET_EVBIT: u64 = ioc(IOC_WRITE, 100, size_of::<libc::c_int>());
const UI_SET_KEYBIT: u64 = ioc(IOC_WRITE, 101, size_of::<libc::c_int>());
const UI_SET_MSCBIT: u64 = ioc(IOC_WRITE, 104, size_of::<libc::c_int>());
const UI_SET_LEDBIT: u64 = ioc(IOC_WRITE, 105, size_of::<libc::c_int>());
const UI_SET_PHYS: u64 = ioc(IOC_WRITE, 108, size_of::<*const libc::c_char>());
const SYSNAME_LEN: usize = 64;
const UI_GET_SYSNAME: u64 = ioc(IOC_READ, 44, SYSNAME_LEN);

/// Identity values for the virtual device.
pub(crate) struct Setup<'a> {
    pub name: &'a str,
    pub vendor: u16,
    pub product: u16,
    pub version: u16,
    pub phys: Option<&'a str>,
}

pub(crate) struct UinputDevice {
    file: File,
    sysname: String,
}

fn check(result: libc::c_int, what: &str) -> io::Result<()> {
    if result < 0 {
        let err = io::Error::last_os_error();
        return Err(io::Error::new(err.kind(), format!("{what}: {err}")));
    }
    Ok(())
}

impl UinputDevice {
    pub(crate) fn create(setup: &Setup<'_>) -> io::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open("/dev/uinput")
            .map_err(|e| {
                let hint = if e.kind() == io::ErrorKind::PermissionDenied {
                    " (no write access to /dev/uinput; see linux/README.md for the udev rule)"
                } else {
                    ""
                };
                io::Error::new(e.kind(), format!("opening /dev/uinput: {e}{hint}"))
            })?;
        let fd = file.as_raw_fd();
        // SAFETY: every ioctl below gets a valid fd and an argument of the
        // type the kernel expects for that request number.
        unsafe {
            let set = |request: u64, value: u16, what: &str| {
                check(libc::ioctl(fd, request as _, libc::c_int::from(value)), what)
            };
            for ev in [EV_KEY, EV_MSC, EV_LED, EV_REP] {
                set(UI_SET_EVBIT, ev, "UI_SET_EVBIT")?;
            }
            for key in hid::advertised_keys() {
                set(UI_SET_KEYBIT, key, "UI_SET_KEYBIT")?;
            }
            set(UI_SET_MSCBIT, MSC_SCAN, "UI_SET_MSCBIT")?;
            for led in [LED_NUML, LED_CAPSL, LED_SCROLLL] {
                set(UI_SET_LEDBIT, led, "UI_SET_LEDBIT")?;
            }
            if let Some(phys) = setup.phys {
                let phys = CString::new(phys).map_err(|_| io::Error::other("phys contains NUL"))?;
                check(libc::ioctl(fd, UI_SET_PHYS as _, phys.as_ptr()), "UI_SET_PHYS")?;
            }

            let mut raw = UinputSetup {
                id: InputId {
                    bustype: BUS_USB,
                    vendor: setup.vendor,
                    product: setup.product,
                    version: setup.version,
                },
                name: [0; UINPUT_MAX_NAME_SIZE],
                ff_effects_max: 0,
            };
            let name = setup.name.as_bytes();
            let len = name.len().min(UINPUT_MAX_NAME_SIZE - 1);
            raw.name[..len].copy_from_slice(&name[..len]);
            check(libc::ioctl(fd, UI_DEV_SETUP as _, &raw as *const UinputSetup), "UI_DEV_SETUP")?;
            check(libc::ioctl(fd, UI_DEV_CREATE as _), "UI_DEV_CREATE")?;
        }

        let mut buf = [0u8; SYSNAME_LEN];
        // SAFETY: the buffer is SYSNAME_LEN bytes, matching the request size.
        let sysname = unsafe {
            if libc::ioctl(fd, UI_GET_SYSNAME as _, buf.as_mut_ptr()) >= 0 {
                let end = buf.iter().position(|&b| b == 0).unwrap_or(SYSNAME_LEN);
                String::from_utf8_lossy(&buf[..end]).into_owned()
            } else {
                String::new()
            }
        };
        Ok(Self { file, sysname })
    }

    /// Kernel name of the device, e.g. `input42`; empty if unknown.
    pub(crate) fn sysname(&self) -> &str {
        &self.sysname
    }

    /// Writes one report: `MSC_SCAN` + `EV_KEY` per event, then `SYN_REPORT`.
    pub(crate) fn write_frame(&mut self, frame: &[KeyEvent]) -> io::Result<()> {
        if frame.is_empty() {
            return Ok(());
        }
        let mut events = Vec::with_capacity(frame.len() * 2 + 1);
        for e in frame {
            let scan = hid::scan_code(e.key).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, format!("no scan code for {:?}", e.key))
            })?;
            events.push(raw_event(EV_MSC, MSC_SCAN, scan));
            events.push(raw_event(EV_KEY, e.key.0, i32::from(e.down)));
        }
        events.push(raw_event(EV_SYN, SYN_REPORT, 0));

        // SAFETY: RawEvent is repr(C) plain data; viewing it as bytes is sound.
        let bytes = unsafe {
            std::slice::from_raw_parts(events.as_ptr().cast::<u8>(), events.len() * size_of::<RawEvent>())
        };
        self.file.write_all(bytes)
    }
}

fn raw_event(kind: u16, code: u16, value: i32) -> RawEvent {
    // The kernel timestamps events itself; the time field is ignored.
    RawEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind, code, value }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        // SAFETY: valid fd; UI_DEV_DESTROY takes no argument.
        unsafe {
            libc::ioctl(self.file.as_raw_fd(), UI_DEV_DESTROY as _);
        }
    }
}
