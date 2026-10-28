//! Image attachments: read from the clipboard or picked from disk, carried to the
//! daemon as the `{type:"image", data:<base64>, mimeType}` blocks a prompt takes.
//!
//! The TUI reads the clipboard by shelling out per platform, and on native Windows
//! has no reader at all. PowerShell can hand one over (`Clipboard::GetImage`), so
//! that is the reader here: one process per paste, and the bytes are sniffed before
//! they are trusted.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The image types the daemon accepts. Anything else is refused at the door rather
/// than sent and dropped later.
const SUPPORTED: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// One image ready to send: the payload, its type, and a file on disk for the
/// thumbnail the composer draws.
#[derive(Debug, Clone)]
pub struct Attachment {
    pub mime: String,
    pub base64: String,
    pub path: PathBuf,
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

fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
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
    let path = std::env::temp_dir().join(format!(
        "pa-attach-{}-{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
        extension_for(mime)
    ));
    std::fs::write(&path, &bytes).ok()?;
    Some(Attachment {
        mime: mime.to_string(),
        base64: base64(&bytes),
        path,
    })
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
    let status = Command::new("powershell")
        .args(["-STA", "-NoProfile", "-NonInteractive", "-Command", &script])
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
    let Ok(output) = Command::new("powershell")
        .args(["-STA", "-NoProfile", "-NonInteractive", "-Command", script])
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
        assert_eq!(SUPPORTED.len(), 4);
    }
}
