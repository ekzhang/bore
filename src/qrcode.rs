//! QR code rendering helpers for console output.

use anyhow::Result;
use qrcode::{render::unicode, QrCode};

/// Render a text payload as console-safe QR code lines.
pub fn render(text: &str) -> Result<Vec<String>> {
    let qr = QrCode::new(text.as_bytes())?
        .render::<unicode::Dense1x2>()
        .quiet_zone(false)
        .build();
    Ok(qr.lines().map(str::to_owned).collect())
}
