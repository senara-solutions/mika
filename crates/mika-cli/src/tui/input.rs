use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use std::path::Path;

use crate::tui::app::{AgentStatus, App, ChatMessage, ChatRole};
use crate::tui::attachment::ImageAttachment;

/// Handle a mouse event (scroll wheel for conversation scrolling).
pub fn handle_mouse(app: &mut App<'_>, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            app.scroll_down(3);
        }
        _ => {} // Ignore clicks, drags, etc.
    }
}

/// Handle a key event with autocomplete-aware dispatch.
pub fn handle_key(app: &mut App<'_>, key: KeyEvent) {
    // Ctrl+C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.should_quit = true;
        return;
    }

    if app.autocomplete.visible {
        handle_key_autocomplete(app, key);
    } else {
        handle_key_normal(app, key);
    }
}

/// Key handling when the autocomplete popup is visible.
fn handle_key_autocomplete(app: &mut App<'_>, key: KeyEvent) {
    match key.code {
        // Esc dismisses popup (keeps typed text)
        KeyCode::Esc => {
            app.autocomplete.dismiss();
        }

        // Tab or Down: next suggestion
        KeyCode::Tab | KeyCode::Down => {
            app.autocomplete.next();
        }

        // Up: previous suggestion
        KeyCode::Up => {
            app.autocomplete.previous();
        }

        // Enter: accept selected completion and execute
        KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
            if let Some(name) = app.autocomplete.selected_name() {
                // Set textarea to the full command and execute
                let cmd = format!("/{name}");
                app.textarea = tui_textarea::TextArea::from(vec![cmd.clone()]);
                app.textarea
                    .set_cursor_line_style(ratatui::style::Style::default());
                app.autocomplete.dismiss();
                if app.status == AgentStatus::Idle {
                    app.send_message();
                }
            }
        }

        // Any other key: pass to textarea, then update autocomplete filter
        _ => {
            app.textarea.input(key);
            let input = app.input_text();
            app.autocomplete.update(&input);
        }
    }
}

/// Key handling when autocomplete is NOT visible (normal mode).
fn handle_key_normal(app: &mut App<'_>, key: KeyEvent) {
    // Ctrl+V: check clipboard for image first, else fall through to normal paste
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('v') {
        match try_clipboard_image() {
            ClipboardResult::Image(attachment) => {
                if let Some(err) = app.attach_image(attachment) {
                    app.messages.push(crate::tui::app::ChatMessage {
                        role: crate::tui::app::ChatRole::System,
                        content: err,
                        rendered: None,
                        channel: None,
                    });
                }
                return;
            }
            ClipboardResult::NoImage => {
                // No image in clipboard — fall through to text paste
            }
            ClipboardResult::Error(msg) => {
                tracing::debug!("clipboard image failed: {msg}");
                app.messages.push(ChatMessage {
                    role: ChatRole::System,
                    content:
                        "Clipboard image not available. Use /attach <path> to attach an image file."
                            .to_string(),
                    rendered: None,
                    channel: None,
                });
                return;
            }
        }
    }

    // Esc: clear attachments first, then input
    if key.code == KeyCode::Esc {
        if app.has_attachments() {
            app.clear_attachments();
            return;
        }
        app.textarea = tui_textarea::TextArea::default();
        app.textarea
            .set_cursor_line_style(ratatui::style::Style::default());
        app.textarea.set_placeholder_text("Type a message...");
        app.history.reset();
        return;
    }

    // PageUp / PageDown scroll messages
    if key.code == KeyCode::PageUp {
        app.scroll_up(5);
        return;
    }
    if key.code == KeyCode::PageDown {
        app.scroll_down(5);
        return;
    }

    // Enter sends message or executes slash command (only when idle and not shift-held)
    if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
        if app.status == AgentStatus::Idle {
            app.send_message();
        }
        return;
    }

    // Tab: if input starts with "/", open autocomplete
    if key.code == KeyCode::Tab {
        let input = app.input_text();
        if input.starts_with('/') {
            app.autocomplete.update(&input);
            return;
        }
        // Otherwise let tab fall through to textarea
    }

    // Ctrl+Up/Down: scroll conversation without leaving input field
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Up {
        app.scroll_up(1);
        return;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Down {
        app.scroll_down(1);
        return;
    }

    // Up/Down: cursor-position-aware history navigation
    // Up triggers history when cursor is on the first row
    if key.code == KeyCode::Up && app.textarea.cursor().0 == 0 {
        app.history_previous();
        return;
    }
    // Down triggers history when cursor is on the last row
    if key.code == KeyCode::Down
        && app.textarea.cursor().0 == app.textarea.lines().len().saturating_sub(1)
    {
        app.history_next();
        return;
    }

    // Pass everything else to textarea
    app.textarea.input(key);

    // After typing, check if we should show autocomplete (e.g., user just typed "/")
    let input = app.input_text();
    if input.starts_with('/') && !input[1..].contains(' ') {
        app.autocomplete.update(&input);
    }
}

/// Handle a bracketed paste event — insert multiline text into the textarea.
pub fn handle_paste(app: &mut App<'_>, text: &str) {
    // Insert each line into the textarea. tui-textarea handles multiline via successive inserts.
    // The simplest approach: replace the textarea with current lines + pasted text.
    let mut current: Vec<String> = app.textarea.lines().to_vec();
    let paste_lines: Vec<&str> = text.lines().collect();

    if let Some(last) = current.last_mut() {
        if let Some((first, rest)) = paste_lines.split_first() {
            last.push_str(first);
            for line in rest {
                current.push(line.to_string());
            }
        }
    } else {
        for line in &paste_lines {
            current.push(line.to_string());
        }
    }

    app.textarea = tui_textarea::TextArea::from(current);
    app.textarea
        .set_cursor_line_style(ratatui::style::Style::default());
    app.needs_redraw = true;
}

/// Maximum pixels allowed for clipboard images (20 megapixels).
const MAX_IMAGE_PIXELS: usize = 20_000_000;
/// Maximum image dimension in either axis.
const MAX_IMAGE_DIMENSION: u32 = 8192;
/// Maximum file size for image loading (10MB).
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// Result of attempting to read an image from the clipboard.
enum ClipboardResult {
    /// Successfully read an image.
    Image(ImageAttachment),
    /// Clipboard accessible but no image content.
    NoImage,
    /// Clipboard not accessible or another error.
    Error(String),
}

/// Try to read an image from the system clipboard.
///
/// Tries arboard first, then falls back to xclip/wl-paste on Linux.
fn try_clipboard_image() -> ClipboardResult {
    // Try arboard first (works on X11, macOS, Windows)
    match try_arboard_image() {
        ClipboardResult::Image(img) => return ClipboardResult::Image(img),
        ClipboardResult::NoImage => return ClipboardResult::NoImage,
        ClipboardResult::Error(e) => {
            tracing::debug!("arboard clipboard failed: {e}, trying xclip/wl-paste fallback");
        }
    }

    // Linux fallback: try xclip, then wl-paste
    #[cfg(target_os = "linux")]
    {
        if let Some(img) = try_xclip_image() {
            return ClipboardResult::Image(img);
        }
        if let Some(img) = try_wl_paste_image() {
            return ClipboardResult::Image(img);
        }
    }

    ClipboardResult::Error("clipboard image not available".to_string())
}

/// Try arboard clipboard image reading.
fn try_arboard_image() -> ClipboardResult {
    let mut clipboard = match arboard::Clipboard::new() {
        Ok(c) => c,
        Err(e) => return ClipboardResult::Error(format!("clipboard init: {e}")),
    };

    let img = match clipboard.get_image() {
        Ok(img) => img,
        Err(arboard::Error::ContentNotAvailable) => return ClipboardResult::NoImage,
        Err(e) => return ClipboardResult::Error(format!("get_image: {e}")),
    };

    match encode_rgba_to_attachment(&img.bytes, img.width, img.height) {
        Some(att) => ClipboardResult::Image(att),
        None => ClipboardResult::Error("image too large or invalid".to_string()),
    }
}

/// Encode raw RGBA pixel data into a PNG-based ImageAttachment.
fn encode_rgba_to_attachment(bytes: &[u8], width: usize, height: usize) -> Option<ImageAttachment> {
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;

    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return None;
    }

    let pixel_count = (width as usize).checked_mul(height as usize)?;
    if pixel_count > MAX_IMAGE_PIXELS {
        return None;
    }

    let estimated_size = bytes.len() / 2;
    let mut png_data = Vec::with_capacity(estimated_size);
    {
        let mut encoder = png::Encoder::new(&mut png_data, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(bytes).ok()?;
    }

    let size_bytes = png_data.len();
    if size_bytes > MAX_IMAGE_BYTES {
        return None;
    }

    use base64::Engine;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&png_data);

    Some(ImageAttachment {
        base64_data,
        media_type: "image/png".to_string(),
        size_bytes,
        label: format!(
            "clipboard image ({})",
            ImageAttachment::format_size(size_bytes)
        ),
    })
}

/// Validate PNG bytes and convert to an `ImageAttachment`.
#[cfg(target_os = "linux")]
fn png_bytes_to_attachment(data: Vec<u8>) -> Option<ImageAttachment> {
    if data.len() > MAX_IMAGE_BYTES || data.len() < 4 {
        return None;
    }
    if &data[..4] != b"\x89PNG" {
        return None;
    }
    let size_bytes = data.len();
    use base64::Engine;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&data);
    Some(ImageAttachment {
        base64_data,
        media_type: "image/png".to_string(),
        size_bytes,
        label: format!(
            "clipboard image ({})",
            ImageAttachment::format_size(size_bytes)
        ),
    })
}

/// Timeout for clipboard subprocess calls (prevents hanging on broken display servers).
#[cfg(target_os = "linux")]
const CLIPBOARD_SUBPROCESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Run a clipboard command with a timeout. Returns None on timeout or failure.
#[cfg(target_os = "linux")]
fn run_clipboard_command(program: &str, args: &[&str]) -> Option<Vec<u8>> {
    use std::process::Command;

    let prog = program.to_string();
    let arg_vec: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = Command::new(&prog).args(&arg_vec).output();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(CLIPBOARD_SUBPROCESS_TIMEOUT) {
        Ok(Ok(output)) if output.status.success() && !output.stdout.is_empty() => {
            Some(output.stdout)
        }
        Ok(_) => None,
        Err(_) => {
            tracing::debug!("{program} clipboard read timed out after 3s");
            None
        }
    }
}

/// Linux fallback: try reading image from clipboard via xclip.
#[cfg(target_os = "linux")]
fn try_xclip_image() -> Option<ImageAttachment> {
    let data = run_clipboard_command(
        "xclip",
        &["-selection", "clipboard", "-t", "image/png", "-o"],
    )?;
    png_bytes_to_attachment(data)
}

/// Linux fallback: try reading image from clipboard via wl-paste (Wayland).
#[cfg(target_os = "linux")]
fn try_wl_paste_image() -> Option<ImageAttachment> {
    let data = run_clipboard_command("wl-paste", &["--type", "image/png"])?;
    png_bytes_to_attachment(data)
}

/// Try to load an image from a file path.
/// Supports png, jpg, gif, webp. Max 10MB.
pub fn try_load_image_file(path: &str) -> Option<ImageAttachment> {
    let path = Path::new(path);

    // Canonicalize to resolve symlinks and prevent path traversal
    let path = path.canonicalize().ok()?;
    if !path.is_file() {
        return None;
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())?;
    let (media_type, magic_bytes): (&str, &[u8]) = match ext.as_str() {
        "png" => ("image/png", b"\x89PNG"),
        "jpg" | "jpeg" => ("image/jpeg", b"\xFF\xD8\xFF"),
        "gif" => ("image/gif", b"GIF"),
        "webp" => ("image/webp", b"RIFF"),
        _ => return None,
    };

    // Check file size via metadata BEFORE reading contents
    let metadata = std::fs::metadata(&path).ok()?;
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return None;
    }

    let data = std::fs::read(&path).ok()?;

    // Validate magic bytes match claimed extension
    if data.len() < magic_bytes.len() || &data[..magic_bytes.len()] != magic_bytes {
        return None;
    }

    let size_bytes = data.len();
    use base64::Engine;
    let base64_data = base64::engine::general_purpose::STANDARD.encode(&data);

    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("image");

    Some(ImageAttachment {
        base64_data,
        media_type: media_type.to_string(),
        size_bytes,
        label: filename.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_load_image_file_nonexistent() {
        assert!(try_load_image_file("/tmp/nonexistent-mika-test.png").is_none());
    }

    #[test]
    fn test_try_load_image_file_unsupported_extension() {
        // Create a temp file with unsupported extension
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "not an image").unwrap();
        assert!(try_load_image_file(path.to_str().unwrap()).is_none());
    }

    #[test]
    fn test_try_load_image_file_valid_png() {
        // File must start with PNG magic bytes to pass validation
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n test data").unwrap();
        let result = try_load_image_file(path.to_str().unwrap());
        assert!(result.is_some());
        let attachment = result.unwrap();
        assert_eq!(attachment.media_type, "image/png");
        assert_eq!(attachment.label, "test.png");
        assert!(attachment.size_bytes > 0);
    }

    #[test]
    fn test_try_load_image_file_jpg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.jpg");
        std::fs::write(&path, b"\xFF\xD8\xFF test jpeg").unwrap();
        let result = try_load_image_file(path.to_str().unwrap());
        assert!(result.is_some());
        assert_eq!(result.unwrap().media_type, "image/jpeg");
    }

    #[test]
    fn test_try_load_image_file_wrong_magic_bytes() {
        // PNG extension but JPEG magic bytes — should be rejected
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.png");
        std::fs::write(&path, b"\xFF\xD8\xFF not a png").unwrap();
        assert!(try_load_image_file(path.to_str().unwrap()).is_none());
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(ImageAttachment::format_size(1024), "1KB");
        assert_eq!(ImageAttachment::format_size(245 * 1024), "245KB");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(ImageAttachment::format_size(1_048_576), "1.0MB");
        assert_eq!(ImageAttachment::format_size(5 * 1_048_576), "5.0MB");
    }
}
