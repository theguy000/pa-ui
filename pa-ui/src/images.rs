//! Image attachments: read from the clipboard or picked from disk, carried to the
//! daemon as the `{type:"image", data:<base64>, mimeType}` blocks a prompt takes.
//!
//! The TUI reads the clipboard by shelling out per platform, and on native Windows
//! has no reader at all. PowerShell can hand one over (`Clipboard::GetImage`), so
//! that is the reader here: one process per paste, and the bytes are sniffed before
//! they are trusted.

use std::path::Path;
use std::process::Command;

/// One image ready to send: the payload and the type it was sniffed as.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub mime: String,
    pub base64: String,
}

impl Attachment {
    /// The wire block for this attachment.
    #[must_use]
    pub fn block(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "image",
            "data": self.base64,
            "mimeType": self.mime,
        })
    }
}

/// Sniff a supported image type from the bytes themselves — the extension and the
/// clipboard's reported type are both worth less than the magic number.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    None
}

/// Base64, standard alphabet with padding — what every image block on this wire uses.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Write the payload somewhere Slint can load it from and wrap it up. The path keeps
/// the sniffed extension: the loader picks its decoder by content, but a file called
/// `.png` that is really a JPEG is a trap for whoever looks at the temp directory.
fn attach(bytes: Vec<u8>) -> Option<Attachment> {
    let mime = sniff(&bytes)?;
    Some(Attachment {
        mime: mime.to_string(),
        base64: base64(&bytes),
    })
}


/// A PowerShell invocation that never paints a console window: the child is a
/// console app, and a black box flashing over the composer is not part of the UI.
#[cfg(windows)]
fn powershell(script: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("powershell");
    command
        .args(["-STA", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(std::process::Stdio::null());
    command
}

/// Is a bitmap on the clipboard right now? One cheap user32 call, so a plain text
/// paste never pays for a PowerShell round trip — and so Ctrl+V can decide, before
/// the key is consumed, whether it is pasting text or attaching an image.
#[cfg(windows)]
#[must_use]
pub fn clipboard_has_image() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn IsClipboardFormatAvailable(format: u32) -> i32;
    }
    // CF_DIB and CF_DIBV5 are what a copy from a browser, a screenshot tool or a
    // paint program puts down; Explorer's file copy (CF_HDROP) deliberately does
    // not count — a file is the picker's job, not a paste's.
    const CF_DIB: u32 = 8;
    const CF_DIBV5: u32 = 17;
    unsafe { IsClipboardFormatAvailable(CF_DIB) != 0 || IsClipboardFormatAvailable(CF_DIBV5) != 0 }
}

#[cfg(not(windows))]
#[must_use]
pub fn clipboard_has_image() -> bool {
    false
}

/// Read an image off the Windows clipboard, if there is one. PowerShell runs the
/// save because it owns the OS pieces (`Clipboard`, `Drawing`) without dragging a
/// windowing toolkit into this crate.
#[cfg(windows)]
pub fn read_clipboard() -> Option<Attachment> {
    let path = std::env::temp_dir().join(format!(
        "pa-clip-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    ));
    let script = format!(
        "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; \
         Add-Type -AssemblyName System.Drawing; \
         $img=[System.Windows.Forms.Clipboard]::GetImage(); \
         if ($img -eq $null) {{ exit 3 }}; \
         $img.Save('{}', [System.Drawing.Imaging.ImageFormat]::Png); exit 0",
        path.display().to_string().replace('\'', "''")
    );
    let status = powershell(&script)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    let bytes = if status.success() { std::fs::read(&path).ok() } else { None };
    let _ = std::fs::remove_file(&path);
    bytes.and_then(attach)
}

#[cfg(not(windows))]
pub fn read_clipboard() -> Option<Attachment> {
    None
}

/// Pick image files. Multi-select, because attaching one screenshot at a time is
/// busywork when the point is to show the model three of them.
#[cfg(windows)]
pub fn pick_files() -> Vec<Attachment> {
    let script = "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; \
         $d=New-Object System.Windows.Forms.OpenFileDialog; \
         $d.Multiselect=$true; \
         $d.Filter='Images|*.png;*.jpg;*.jpeg;*.webp;*.gif|All files|*.*'; \
         if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { \
             $d.FileNames | ForEach-Object { Write-Output $_ } }";
    let Ok(output) = powershell(script)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| std::fs::read(Path::new(line)).ok())
        .filter_map(attach)
        .collect()
}

#[cfg(not(windows))]
pub fn pick_files() -> Vec<Attachment> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffing_trusts_the_bytes() {
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G', 13, 10, 26, 10, 0]), Some("image/png"));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"RIFF____WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"GIF89a...."), Some("image/gif"));
        assert_eq!(sniff(b"not an image"), None);
    }

    #[test]
    fn base64_matches_the_textbook() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
    }
}

/// Base64 decode: the encoder above hands bytes out, this is how the pictures in
/// a transcript come back in.
#[must_use]
pub fn decode_base64(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4 + 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for ch in input.bytes() {
        let value = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

const MAX_IMAGE_CACHE: usize = 64;

struct ImageCache {
    map: std::collections::HashMap<u64, slint::Image>,
    order: std::collections::VecDeque<u64>,
}

impl ImageCache {
    fn new() -> Self {
        Self {
            map: std::collections::HashMap::new(),
            order: std::collections::VecDeque::new(),
        }
    }

    fn get(&mut self, key: u64) -> Option<slint::Image> {
        let img = self.map.get(&key)?.clone();
        if let Some(pos) = self.order.iter().position(|&k| k == key) {
            self.order.remove(pos);
        }
        self.order.push_back(key);
        Some(img)
    }

    fn insert(&mut self, key: u64, image: slint::Image) {
        if self.map.contains_key(&key) {
            if let Some(pos) = self.order.iter().position(|&k| k == key) {
                self.order.remove(pos);
            }
        } else if self.map.len() >= MAX_IMAGE_CACHE {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
        self.order.push_back(key);
        self.map.insert(key, image);
    }
}

thread_local! {
    /// Decoded once per payload, bounded by LRU so long sessions stay lean.
    static CACHE: std::cell::RefCell<ImageCache> =
        std::cell::RefCell::new(ImageCache::new());
}

/// A base64 payload as something Slint can draw, cached by content.
///
/// Decoded straight to the size the transcript draws it at: a 787 × 515
/// screenshot is 1.6 MB of RGBA at full size and is shown in a 220 × 150 box, so
/// the full bitmap is never worth holding — and the poll rebuilds the row.
#[must_use]
pub fn from_wire(base64: &str) -> slint::Image {
    from_wire_sized(base64, THUMB_W, THUMB_H)
}

/// Transcript images are drawn at this size with `image-fit: contain`.
pub const THUMB_W: u32 = 440;
pub const THUMB_H: u32 = 300;

/// The same decode, at an explicit bound (2× the drawn box, so a high-DPI
/// screen still has pixels to spare).
#[must_use]
pub fn from_wire_sized(base64: &str, max_w: u32, max_h: u32) -> slint::Image {
    if base64.is_empty() {
        return slint::Image::default();
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&(base64, max_w, max_h), &mut hasher);
    let key = std::hash::Hasher::finish(&hasher);
    if let Some(image) = CACHE.with(|cache| cache.borrow_mut().get(key)) {
        return image;
    }
    let bytes = decode_base64(base64);
    let image = decode_thumbnail(&bytes, max_w, max_h).unwrap_or_default();
    CACHE.with(|cache| cache.borrow_mut().insert(key, image.clone()));
    image
}

/// Decode a picture and shrink it to fit the box, keeping the aspect ratio.
/// Falls back to the file path when the bytes cannot be read as an image.
#[must_use]
fn decode_thumbnail(bytes: &[u8], max_w: u32, max_h: u32) -> Option<slint::Image> {
    use image::imageops::FilterType;
    let decoded = image::load_from_memory(bytes).ok()?;
    let (width, height) = (decoded.width(), decoded.height());
    if width <= max_w && height <= max_h {
        let rgba = decoded.to_rgba8();
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            rgba.as_raw(),
            width,
            height,
        );
        return Some(slint::Image::from_rgba8(buffer));
    }
    // Triangle is the cheap filter that still looks clean at half size; Lanczos3
    // doubles the decode cost for a difference nobody sees in a 220 px box.
    let scale = (f64::from(max_w) / f64::from(width)).min(f64::from(max_h) / f64::from(height));
    let target_w = ((f64::from(width) * scale).round() as u32).max(1);
    let target_h = ((f64::from(height) * scale).round() as u32).max(1);
    let thumb = decoded.resize_exact(target_w, target_h, FilterType::Triangle);
    let rgba = thumb.to_rgba8();
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        rgba.as_raw(),
        target_w,
        target_h,
    );
    Some(slint::Image::from_rgba8(buffer))
}

#[cfg(test)]
mod decode_tests {
    use super::{base64, decode_base64, from_wire_sized};

    #[test]
    fn a_big_picture_arrives_at_thumbnail_size() {
        // 800 x 600, so the bound has to bite
        let mut picture = image::RgbaImage::new(800, 600);
        for (x, y, pixel) in picture.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]);
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(picture)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let payload = base64(png.get_ref());
        let drawn = from_wire_sized(&payload, 440, 300);
        let size = drawn.size();
        assert!(size.width <= 440 && size.height <= 300, "got {size:?}");
        // and the aspect ratio is kept: 800x600 -> 400x300
        assert_eq!((size.width, size.height), (400, 300));
    }

    #[test]
    fn decodes_what_the_encoder_writes() {
        assert_eq!(decode_base64("QUJD"), b"ABC");
        assert_eq!(decode_base64("QUJDRA=="), b"ABCD");
        assert_eq!(decode_base64(&base64(b"a longer payload \x00\x01\xf0")), b"a longer payload \x00\x01\xf0");
    }
}
