/// Maximum number of image attachments per message.
pub const MAX_ATTACHMENTS: usize = 10;
/// Maximum total image bytes across all attachments (20MB).
pub const MAX_TOTAL_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// An image attachment pending send with the next message.
#[derive(Clone)]
pub struct ImageAttachment {
    pub base64_data: String,
    pub media_type: String,
    pub size_bytes: usize,
    pub label: String,
}

impl ImageAttachment {
    /// Human-readable size (e.g., "245KB").
    pub fn size_display(&self) -> String {
        Self::format_size(self.size_bytes)
    }

    /// Format a byte count as a human-readable size string.
    pub fn format_size(bytes: usize) -> String {
        if bytes >= 1_048_576 {
            format!("{:.1}MB", bytes as f64 / 1_048_576.0)
        } else {
            format!("{}KB", bytes / 1024)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_display_kilobytes() {
        let att = ImageAttachment {
            base64_data: String::new(),
            media_type: "image/png".into(),
            size_bytes: 245 * 1024,
            label: "test".into(),
        };
        assert_eq!(att.size_display(), "245KB");
    }

    #[test]
    fn test_size_display_megabytes() {
        let att = ImageAttachment {
            base64_data: String::new(),
            media_type: "image/png".into(),
            size_bytes: 2 * 1024 * 1024 + 512 * 1024,
            label: "test".into(),
        };
        assert_eq!(att.size_display(), "2.5MB");
    }
}
