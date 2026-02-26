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
        if self.size_bytes >= 1_048_576 {
            format!("{:.1}MB", self.size_bytes as f64 / 1_048_576.0)
        } else {
            format!("{}KB", self.size_bytes / 1024)
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
