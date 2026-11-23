//! evdev key code → USB HID usage (keyboard page 0x07).
//!
//! A real USB keyboard handled by `hid-generic` reports `MSC_SCAN` with
//! `0x7_0000 | usage` before every key event; we send the same.

use humanize_core::KeyCode;

/// HID keyboard usage page, shifted into the high half as the kernel reports it.
const KEYBOARD_PAGE: i32 = 0x7_0000;

/// (evdev code, HID usage) for every key the virtual keyboard can send.
#[rustfmt::skip]
const TABLE: &[(u16, u8)] = &[
    // Letters a–z.
    (30, 0x04), (48, 0x05), (46, 0x06), (32, 0x07), (18, 0x08), (33, 0x09), (34, 0x0a),
    (35, 0x0b), (23, 0x0c), (36, 0x0d), (37, 0x0e), (38, 0x0f), (50, 0x10), (49, 0x11),
    (24, 0x12), (25, 0x13), (16, 0x14), (19, 0x15), (31, 0x16), (20, 0x17), (22, 0x18),
    (47, 0x19), (17, 0x1a), (45, 0x1b), (21, 0x1c), (44, 0x1d),
    // Digits 1–9, 0.
    (2, 0x1e), (3, 0x1f), (4, 0x20), (5, 0x21), (6, 0x22), (7, 0x23), (8, 0x24), (9, 0x25),
    (10, 0x26), (11, 0x27),
    // Enter, Esc, Backspace, Tab, Space.
    (28, 0x28), (1, 0x29), (14, 0x2a), (15, 0x2b), (57, 0x2c),
    // - = [ ] \ ; ' ` , . /
    (12, 0x2d), (13, 0x2e), (26, 0x2f), (27, 0x30), (43, 0x31), (39, 0x33), (40, 0x34),
    (41, 0x35), (51, 0x36), (52, 0x37), (53, 0x38),
    // Caps Lock, ISO key left of Z.
    (58, 0x39), (86, 0x64),
    // Modifiers: LCtrl LShift LAlt LMeta RCtrl RShift RAlt RMeta.
    (29, 0xe0), (42, 0xe1), (56, 0xe2), (125, 0xe3), (97, 0xe4), (54, 0xe5), (100, 0xe6),
    (126, 0xe7),
];

/// The `MSC_SCAN` value for `key`, or `None` if the keyboard cannot send it.
pub fn scan_code(key: KeyCode) -> Option<i32> {
    TABLE.iter().find(|&&(code, _)| code == key.0).map(|&(_, usage)| KEYBOARD_PAGE | i32::from(usage))
}

/// Keys a standard 105-key keyboard advertises (evdev codes). Wider than
/// [`TABLE`]: a real keyboard reports F-keys, navigation and keypad too.
pub fn advertised_keys() -> impl Iterator<Item = u16> {
    (1..=88u16).filter(|&k| k != 84).chain(96..=111).chain([119, 125, 126, 127])
}

#[cfg(test)]
mod tests {
    use super::*;
    use humanize_core::Layout;

    #[test]
    fn every_key_the_typist_uses_has_a_scan_code() {
        let layout = Layout::us_qwerty();
        for c in (' '..='~').chain(['\n', '\t']) {
            let stroke = layout.stroke(c).unwrap();
            assert!(scan_code(stroke.key).is_some(), "{c:?}");
        }
        for key in [KeyCode::BACKSPACE, KeyCode::LEFT_SHIFT, KeyCode::RIGHT_SHIFT, KeyCode::LEFT_CTRL] {
            assert!(scan_code(key).is_some());
        }
    }

    #[test]
    fn known_values_match_the_hid_spec() {
        assert_eq!(scan_code(KeyCode(30)), Some(0x70004)); // a
        assert_eq!(scan_code(KeyCode::ENTER), Some(0x70028));
        assert_eq!(scan_code(KeyCode::LEFT_SHIFT), Some(0x700e1));
        assert_eq!(scan_code(KeyCode(200)), None);
    }

    #[test]
    fn advertised_keys_cover_the_table() {
        let advertised: Vec<u16> = advertised_keys().collect();
        assert!(TABLE.iter().all(|(code, _)| advertised.contains(code)));
    }
}
