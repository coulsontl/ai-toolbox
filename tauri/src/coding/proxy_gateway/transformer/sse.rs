use serde_json::Value;

pub(crate) fn append_utf8_safe(buffer: &mut String, remainder: &mut Vec<u8>, chunk: &[u8]) {
    if chunk.is_empty() {
        return;
    }

    let mut bytes = Vec::with_capacity(remainder.len() + chunk.len());
    bytes.extend_from_slice(remainder);
    bytes.extend_from_slice(chunk);
    remainder.clear();

    match std::str::from_utf8(&bytes) {
        Ok(text) => buffer.push_str(text),
        Err(error) => {
            let valid_up_to = error.valid_up_to();
            if valid_up_to > 0 {
                buffer.push_str(&String::from_utf8_lossy(&bytes[..valid_up_to]));
            }
            remainder.extend_from_slice(&bytes[valid_up_to..]);
        }
    }
}

pub(crate) fn take_sse_block(buffer: &mut String) -> Option<String> {
    let (index, delimiter_len) = find_sse_delimiter(buffer)?;
    let block = buffer[..index].to_string();
    let rest_start = index + delimiter_len;
    buffer.replace_range(..rest_start, "");
    Some(block)
}

fn find_sse_delimiter(buffer: &str) -> Option<(usize, usize)> {
    let bytes = buffer.as_bytes();
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        })
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
