use anyhow::Result;
use qrcode::{render::unicode, QrCode};

pub fn render(text: &str) -> Result<Vec<String>> {
    let qr = QrCode::new(text.as_bytes())?
        .render::<unicode::Dense1x2>()
        .quiet_zone(false)
        .build();
    Ok(qr.lines().map(str::to_owned).collect())
}
