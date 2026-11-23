//! Creates a real uinput device and checks how the kernel sees it.
//!
//! No key events are sent, so nothing is typed anywhere. Ignored by default
//! because it needs write access to /dev/uinput and briefly adds a keyboard
//! to the desktop:
//!
//! ```sh
//! cargo test -p humanize-input --test device -- --ignored
//! ```

use std::path::PathBuf;

use humanize_core::{Driver, KeyCode};
use humanize_input::{DeviceIdentity, UinputKeyboard};

fn sysfs(keyboard: &UinputKeyboard) -> PathBuf {
    PathBuf::from("/sys/devices/virtual/input").join(keyboard.sysname())
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).trim().to_owned()
}

#[test]
#[ignore = "creates a real input device"]
fn device_looks_like_a_usb_keyboard() {
    let identity = DeviceIdentity { name: "humanize test keyboard".into(), ..DeviceIdentity::default() };
    let keyboard = UinputKeyboard::create(&identity).expect("create uinput device");
    let dir = sysfs(&keyboard);
    assert!(keyboard.sysname().starts_with("input"), "sysname {:?}", keyboard.sysname());

    assert_eq!(read(dir.join("name")), "humanize test keyboard");
    assert_eq!(read(dir.join("id/bustype")), "0003"); // BUS_USB
    assert_eq!(read(dir.join("id/vendor")), "1c4f");
    assert_eq!(read(dir.join("id/product")), "0002");
    assert_eq!(read(dir.join("id/version")), "0110");

    // EV_SYN | EV_KEY | EV_MSC | EV_LED | EV_REP = 0x120013
    assert_eq!(read(dir.join("capabilities/ev")), "120013");
    assert_eq!(read(dir.join("capabilities/msc")), "10"); // MSC_SCAN
    assert_eq!(read(dir.join("capabilities/led")), "7"); // Num, Caps, Scroll

    // The device shows up as an event node and the LED class devices exist.
    let has_event_node = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .any(|e| e.file_name().to_string_lossy().starts_with("event"));
    assert!(has_event_node);
    let caps_led = PathBuf::from("/sys/class/leds").join(format!("{}::capslock", keyboard.sysname()));
    assert!(caps_led.exists(), "{} missing", caps_led.display());
}

#[test]
#[ignore = "creates a real input device"]
fn rejects_keys_without_a_scan_code() {
    let mut keyboard = UinputKeyboard::create(&DeviceIdentity::default()).unwrap();
    assert!(keyboard.key(KeyCode(200), true).is_err());
    // A valid key is queued fine; releasing it right away types nothing
    // visible beyond one key, so use a modifier: no character at all.
    assert!(keyboard.key(KeyCode::LEFT_SHIFT, true).is_ok());
    assert!(keyboard.key(KeyCode::LEFT_SHIFT, false).is_ok());
}
