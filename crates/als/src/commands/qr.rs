//! Tiny QR renderer for `als auth login`.
//!
//! Encodes the verification URL with the `qrcode` crate, then prints
//! it to the supplied writer using unicode half-block characters
//! (`U+2580 UPPER HALF BLOCK`, `U+2584 LOWER HALF BLOCK`, space, full
//! block). Two QR modules fit in one terminal row, so the rendered
//! grid stays square at typical monospace cell aspect ratios.
//!
//! The crate's default features (`image`, `svg`, `pic`) are turned off
//! in `Cargo.toml`; we only need the bare `QrCode` encoder.

use std::io::Write;

use qrcode::{EcLevel, QrCode};

const QUIET_ZONE: usize = 2;

/// Encode `data` as a QR code and write the unicode-block rendering to
/// `out`. Errors propagate from the underlying writer; encoding itself
/// is infallible for the URL lengths we deal with (under ~2 KB).
///
/// The output ends with a trailing newline.
pub(crate) fn render<W: Write + ?Sized>(out: &mut W, data: &str) -> std::io::Result<()> {
    let Ok(code) = QrCode::with_error_correction_level(data, EcLevel::M) else {
        return Ok(());
    };
    let width = code.width();
    let total = width + QUIET_ZONE * 2;
    let modules: Vec<bool> = code
        .to_colors()
        .into_iter()
        .map(|c| c == qrcode::Color::Dark)
        .collect();
    let is_dark = |x: usize, y: usize| -> bool {
        if x < QUIET_ZONE || y < QUIET_ZONE {
            return false;
        }
        let (gx, gy) = (x - QUIET_ZONE, y - QUIET_ZONE);
        if gx >= width || gy >= width {
            return false;
        }
        modules[gy * width + gx]
    };
    let mut y = 0;
    while y < total {
        for x in 0..total {
            let top = is_dark(x, y);
            let bot = if y + 1 < total {
                is_dark(x, y + 1)
            } else {
                false
            };
            let glyph = match (top, bot) {
                (true, true) => '\u{2588}',  // full block
                (true, false) => '\u{2580}', // upper half
                (false, true) => '\u{2584}', // lower half
                (false, false) => ' ',
            };
            write!(out, "{glyph}")?;
        }
        writeln!(out)?;
        y += 2;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn render_emits_non_empty_block_grid() {
        let mut buf = Vec::new();
        render(&mut buf, "https://a.ls/verify?user_code=ABCD-EFGH").unwrap();
        let s = String::from_utf8(buf).unwrap();
        // Half-block glyphs and at least a few rows.
        assert!(s.contains('\u{2588}') || s.contains('\u{2580}') || s.contains('\u{2584}'));
        assert!(s.lines().count() > 4);
    }

    #[test]
    fn render_short_string_succeeds() {
        let mut buf = Vec::new();
        render(&mut buf, "hello").unwrap();
        assert!(!buf.is_empty());
    }
}
