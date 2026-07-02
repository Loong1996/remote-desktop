use std::io::Write;
use std::process::{Command, Stdio};

/// Approximate 256 KB cap on clipboard payloads (bytes), matching the web
/// CLIP_MAX_BYTES guard.
pub const CLIP_MAX_BYTES: usize = 262_144;

/// Read the macOS clipboard via `pbpaste`. Non-macOS: unsupported (Err).
pub fn read_clipboard() -> anyhow::Result<String> {
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("/usr/bin/pbpaste").output()?;
        if !out.status.success() {
            anyhow::bail!("pbpaste exited with {}", out.status);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("clipboard unsupported on this platform")
    }
}

/// Write `text` to the macOS clipboard via `pbcopy`. Non-macOS: unsupported (Err).
pub fn write_clipboard(text: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut child = Command::new("/usr/bin/pbcopy").stdin(Stdio::piped()).spawn()?;
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no pbcopy stdin"))?
            .write_all(text.as_bytes())?;
        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("pbcopy exited with {status}");
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        anyhow::bail!("clipboard unsupported on this platform")
    }
}

/// Decide the text to broadcast given the current clipboard and last-known
/// value. Returns None when unchanged, empty, or over the size cap (skip —
/// never truncate).
pub fn clipboard_to_send(current: &str, last_known: &str, cap_bytes: usize) -> Option<String> {
    if current.is_empty() || current == last_known || current.len() > cap_bytes {
        return None;
    }
    Some(current.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_to_send_skips_unchanged_empty_and_oversized() {
        assert_eq!(clipboard_to_send("a", "a", 100), None); // unchanged
        assert_eq!(clipboard_to_send("", "a", 100), None); // empty
        assert_eq!(clipboard_to_send("b", "a", 100), Some("b".to_string())); // changed
        let big = "x".repeat(101);
        assert_eq!(clipboard_to_send(&big, "a", 100), None); // over cap
    }

    // Requires macOS + a session pasteboard. Run explicitly:
    // cargo test --manifest-path agent/Cargo.toml -- --ignored clipboard_roundtrip
    #[test]
    #[ignore]
    fn clipboard_roundtrip() {
        write_clipboard("rd-clip-test-123").unwrap();
        assert_eq!(read_clipboard().unwrap(), "rd-clip-test-123");
    }
}
