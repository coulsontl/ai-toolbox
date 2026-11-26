use serde_json::Value;

pub(crate) fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.is_empty() {
        return;
    }

    let mut bytes = Vec::with_capacity(remainder.len() + chunk.len());
    bytes.extend_from_slice(remainder);
    bytes.extend_from_slice(chunk);
    remainder.clear();

    let mut offset = 0;
    while offset < bytes.len() {
        match std::str::from_utf8(&bytes[offset..]) {
            Ok(text) => {
                buffer.push_str(text);
                return;
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                if valid_up_to > 0 {
                    buffer.push_str(
                        std::str::from_utf8(&bytes[offset..offset + valid_up_to])
                            .expect("valid_up_to must point at valid UTF-8"),
                    );
                    offset += valid_up_to;
                }
                match error.error_len() {
                    Some(error_len) => {
                        // Invalid byte sequence: replace and keep parsing the rest.
                        buffer.push('\u{FFFD}');
                        offset += error_len;
                    }
                    None => {
                        // Incomplete trailing sequence: wait for more bytes.
                        remainder.extend_from_slice(&bytes[offset..]);
                        return;
                    }
                }
            }
        }
    }
}

/// Locate the first complete SSE event whose delimiter starts at or after
/// `from` (a char boundary). Handles `\n\n`, `\r\n\r\n`, `\r\n\n`, and `\n\r\n`
/// and returns `(block_end, next_block_start)`.
#[inline]
fn find_sse_block_from(buffer: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = buffer.as_bytes();
    let mut offset = from;
    while let Some(pos) = buffer[offset..].find('\n') {
        let newline = offset + pos;
        let after = &bytes[newline + 1..];
        let end = if after.starts_with(b"\n") {
            newline + 2
        } else if after.starts_with(b"\r\n") {
            newline + 3
        } else {
            offset = newline + 1;
            continue;
        };
        let start = if newline > 0 && bytes[newline - 1] == b'\r' {
            newline - 1
        } else {
            newline
        };
        return Some((start, end));
    }
    None
}

/// Earliest position a later scan has to revisit: a newline in the last two
/// bytes may still form a delimiter once more bytes arrive.
#[inline]
fn sse_resume_point(pending: &str) -> usize {
    let mut at = pending.len().saturating_sub(2);
    while !pending.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Borrowing iterator over SSE blocks of an already-complete body.
#[allow(dead_code)]
pub(crate) struct SseBlocks<'a> {
    text: &'a str,
    pos: usize,
}

#[allow(dead_code)]
impl<'a> SseBlocks<'a> {
    pub(crate) fn new(text: &'a str) -> Self {
        Self { text, pos: 0 }
    }

    pub(crate) fn remainder(&self) -> &'a str {
        &self.text[self.pos..]
    }
}

impl<'a> Iterator for SseBlocks<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let rest = &self.text[self.pos..];
        let (end, next) = find_sse_block_from(rest, 0)?;
        self.pos += next;
        Some(&rest[..end])
    }
}

/// Incremental SSE framer for streamed bodies (cc-switch e217bfcb).
///
/// Events are yielded as borrowed slices of the internal buffer; consumed text
/// is compacted once per incoming chunk instead of once per event, and a failed
/// delimiter scan resumes near the end rather than rescanning long partial
/// events in O(N^2).
#[derive(Debug, Default)]
pub(crate) struct SseBuffer {
    text: String,
    utf8_remainder: Vec<u8>,
    consumed: usize,
    scan_from: usize,
}

impl SseBuffer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push_bytes(&mut self, chunk: &[u8]) {
        self.compact();
        append_utf8_safe(&mut self.text, &mut self.utf8_remainder, chunk);
    }

    pub(crate) fn next_block(&mut self) -> Option<&str> {
        let scan_start = self.scan_from.max(self.consumed);
        if let Some((block_end, next_start)) = find_sse_block_from(&self.text, scan_start) {
            let block_start = self.consumed;
            self.consumed = next_start;
            self.scan_from = next_start;
            return Some(&self.text[block_start..block_end]);
        }
        let pending = &self.text[self.consumed..];
        self.scan_from = self.consumed + sse_resume_point(pending);
        None
    }

    pub(crate) fn take_tail(&mut self) -> String {
        self.compact();
        if !self.utf8_remainder.is_empty() {
            self.text
                .push_str(&String::from_utf8_lossy(&self.utf8_remainder));
            self.utf8_remainder.clear();
        }
        self.consumed = 0;
        self.scan_from = 0;
        std::mem::take(&mut self.text)
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.utf8_remainder.clear();
        self.consumed = 0;
        self.scan_from = 0;
    }

    fn compact(&mut self) {
        if self.consumed == 0 {
            return;
        }
        if self.consumed >= self.text.len() {
            self.text.clear();
        } else {
            self.text.drain(..self.consumed);
        }
        self.scan_from = self.scan_from.saturating_sub(self.consumed);
        self.consumed = 0;
    }
}

pub(crate) fn take_sse_block(buffer: &mut String) -> Option<String> {
    let (block_end, next_start) = find_sse_block_from(buffer, 0)?;
    let block = buffer[..block_end].to_string();
    buffer.replace_range(..next_start, "");
    Some(block)
}

pub(crate) fn parse_sse_block(block: &str) -> ParsedSseBlock {
    let mut event: Option<String> = None;
    let mut data_parts = Vec::new();

    for line in block.lines() {
        if let Some(value) = strip_sse_field(line, "event") {
            event = Some(value.trim().to_string());
        } else if let Some(value) = strip_sse_field(line, "data") {
            data_parts.push(value.to_string());
        }
    }

    ParsedSseBlock {
        event,
        data: data_parts.join("\n"),
    }
}

pub(crate) fn strip_sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?;
    let rest = rest.strip_prefix(':')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

pub(crate) fn sse_event(event: Option<&str>, value: &Value) -> Vec<u8> {
    let payload = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    match event {
        Some(event) if !event.is_empty() => format!("event: {event}\ndata: {payload}\n\n").into(),
        _ => format!("data: {payload}\n\n").into(),
    }
}

pub(crate) fn sse_done() -> Vec<u8> {
    b"data: [DONE]\n\n".to_vec()
}

pub(crate) struct ParsedSseBlock {
    pub(crate) event: Option<String>,
    pub(crate) data: String,
}

#[cfg(test)]
mod tests {
    use super::{append_utf8_safe, take_sse_block, SseBlocks, SseBuffer};

    #[test]
    fn parser_uses_the_earliest_mixed_line_ending_delimiter() {
        let mut buffer =
            "event: first\ndata: {\"type\":\"first\"}\n\nevent: second\r\ndata: {}\r\n\r\n"
                .to_string();
        assert_eq!(
            take_sse_block(&mut buffer).as_deref(),
            Some("event: first\ndata: {\"type\":\"first\"}")
        );
        assert_eq!(
            take_sse_block(&mut buffer).as_deref(),
            Some("event: second\r\ndata: {}")
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn append_utf8_safe_replaces_invalid_bytes_and_continues() {
        let mut buffer = String::new();
        let mut remainder = Vec::new();
        append_utf8_safe(&mut buffer, &mut remainder, b"hello");
        append_utf8_safe(&mut buffer, &mut remainder, b"\xff world");
        append_utf8_safe(&mut buffer, &mut remainder, b" tail");
        assert_eq!(buffer, "hello\u{FFFD} world tail");
        assert!(remainder.is_empty());
    }

    #[test]
    fn append_utf8_safe_keeps_incomplete_multibyte_sequence() {
        let mut buffer = String::new();
        let mut remainder = Vec::new();
        // Leading bytes of a 3-byte UTF-8 sequence without continuation.
        append_utf8_safe(&mut buffer, &mut remainder, &[0xE4, 0xBD]);
        assert!(buffer.is_empty());
        assert_eq!(remainder, vec![0xE4, 0xBD]);
        append_utf8_safe(&mut buffer, &mut remainder, &[0xA0]);
        assert_eq!(buffer.chars().count(), 1);
        assert!(remainder.is_empty());
    }

    #[test]
    fn sse_buffer_handles_split_delimiters_mixed_endings_and_tail() {
        let mut buf = SseBuffer::new();
        buf.push_bytes(b"event: a\r\ndata: 1\r");
        assert!(buf.next_block().is_none());
        buf.push_bytes(b"\n\r\ndata: 2\n\ndata: trailing");
        assert_eq!(buf.next_block(), Some("event: a\r\ndata: 1"));
        assert_eq!(buf.next_block(), Some("data: 2"));
        assert!(buf.next_block().is_none());
        assert_eq!(buf.take_tail(), "data: trailing");
    }

    #[test]
    fn sse_blocks_iterates_in_place_and_exposes_remainder() {
        let body = "data: one\n\ndata: two\r\n\r\ndata: three";
        let mut iter = SseBlocks::new(body);
        assert_eq!(iter.next(), Some("data: one"));
        assert_eq!(iter.next(), Some("data: two"));
        assert_eq!(iter.next(), None);
        assert_eq!(iter.remainder(), "data: three");
    }
}
