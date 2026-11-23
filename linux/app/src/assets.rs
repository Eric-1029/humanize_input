//! Bundled font and background, unpacked at start so one binary is enough.

use std::ffi::{CString, c_void};
use std::path::{Path, PathBuf};

const SAIRA: &[u8] = include_bytes!("../../assets/fonts/Saira.ttf");
const GROUND_A: &str = include_str!("../../assets/ground/contour-a.svg");
const GROUND_B: &str = include_str!("../../assets/ground/contour-b.svg");
const THEME: &str = include_str!("theme.css");

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const libc_char) -> i32;
}
#[allow(non_camel_case_types)]
type libc_char = u8;

fn asset_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("humanize_input")
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|old| old == bytes) {
        return Ok(());
    }
    std::fs::write(path, bytes)
}

/// Contour lines in `colour` at the ground opacity (9 %).
fn tint(svg: &str, colour: &str) -> String {
    svg.replacen(r##"stroke="#000""##, &format!(r#"stroke="{colour}" stroke-opacity="0.09""#), 1)
}

/// Registers Saira with fontconfig (must run before GTK starts) and returns
/// the stylesheet with asset paths filled in.
pub fn prepare() -> Result<String, String> {
    let dir = asset_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let font = dir.join("Saira.ttf");
    write_if_changed(&font, SAIRA).map_err(|e| format!("{}: {e}", font.display()))?;
    let c_path = CString::new(font.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: null selects the current fontconfig configuration; the path is
    // a valid NUL-terminated string that outlives the call.
    let added = unsafe { FcConfigAppFontAddFile(std::ptr::null_mut(), c_path.as_ptr().cast()) };
    if added == 0 {
        eprintln!("warning: fontconfig did not accept {}", font.display());
    }

    let mut css = THEME.to_owned();
    for (key, svg, colour) in [
        ("GROUND_A_LIGHT", GROUND_A, "#161616"),
        ("GROUND_B_LIGHT", GROUND_B, "#161616"),
        ("GROUND_A_DARK", GROUND_A, "#EDEDED"),
        ("GROUND_B_DARK", GROUND_B, "#EDEDED"),
    ] {
        let path = dir.join(format!("{}.svg", key.to_lowercase()));
        write_if_changed(&path, tint(svg, colour).as_bytes())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        css = css.replace(&format!("{{{{{key}}}}}"), &format!("file://{}", path.display()));
    }
    Ok(css)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contours_are_tinted_and_placeholders_filled() {
        assert!(tint(GROUND_A, "#161616").contains(r##"stroke="#161616" stroke-opacity="0.09""##));
        assert!(THEME.contains("{{GROUND_A_LIGHT}}"));
    }
}
