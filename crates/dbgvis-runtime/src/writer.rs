use std::fmt;

/// A UTF-8 prefix writer. Once full, all further writes fail, even empty writes.
pub struct BoundedWriter<'a> {
    buffer: &'a mut [u8],
    written: usize,
    truncated: bool,
}

impl<'a> BoundedWriter<'a> {
    pub fn new(buffer: &'a mut [u8]) -> Self {
        Self {
            buffer,
            written: 0,
            truncated: false,
        }
    }

    pub fn len(&self) -> usize {
        self.written
    }
    pub fn is_empty(&self) -> bool {
        self.written == 0
    }
    pub fn truncated(&self) -> bool {
        self.truncated
    }
    pub fn remaining(&self) -> usize {
        self.buffer.len() - self.written
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer[..self.written]
    }
}

impl fmt::Write for BoundedWriter<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.truncated {
            return Err(fmt::Error);
        }
        let remaining = self.buffer.len() - self.written;
        let mut end = remaining.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.buffer[self.written..self.written + end].copy_from_slice(&text.as_bytes()[..end]);
        self.written += end;
        if end != text.len() {
            self.truncated = true;
            Err(fmt::Error)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn preserves_prefix_at_every_utf8_boundary() {
        let text = "a中🙂\0z";
        for capacity in 0..=text.len() + 1 {
            let mut buffer = [0xCC; 32];
            let mut writer = BoundedWriter::new(&mut buffer[..capacity]);
            let result = writer.write_str(text);
            let prefix = std::str::from_utf8(writer.as_bytes()).unwrap();
            assert!(text.starts_with(prefix));
            assert_eq!(result.is_ok(), capacity >= text.len());
            assert_eq!(writer.truncated(), capacity < text.len());
            if writer.truncated() {
                assert!(writer.write_str("x").is_err());
                assert!(writer.write_str("").is_err());
            }
            assert_eq!(buffer[capacity], 0xCC);
        }
    }

    #[test]
    fn empty_exact_and_fragmented_writes() {
        let mut buffer = [0; 4];
        let mut writer = BoundedWriter::new(&mut buffer);
        writer.write_str("").unwrap();
        writer.write_str("a").unwrap();
        writer.write_str("中").unwrap();
        writer.write_str("").unwrap();
        assert_eq!(writer.as_bytes(), "a中".as_bytes());
        assert!(!writer.truncated());
        assert!(writer.write_str("!").is_err());
    }
}
