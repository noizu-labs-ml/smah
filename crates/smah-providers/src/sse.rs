//! Hand-rolled SSE (Server-Sent Events) line parser over raw byte chunks
//! (plan §B: "~100L; content_block_delta / chat.completion.chunk / [DONE]").
//!
//! This is a line protocol, not a stream combinator: the caller feeds each
//! `bytes_stream` chunk into [`SseParser::feed`] and hands the returned
//! [`SseEvent`]s to the provider-specific decoder (Anthropic
//! `content_block_delta`, OpenAI `chat.completion.chunk` / `[DONE]`).
//!
//! WHATWG-compliant where it matters for the two vendors: `data:`/`event:`
//! fields with one optional leading space, `:`-comment lines ignored,
//! CRLF tolerated, multi-line data joined with `\n`, dispatch on the blank
//! line. Buffering is byte-based so a UTF-8 sequence split across chunks
//! survives (line boundaries are ASCII `\n`, which never appears inside a
//! multi-byte sequence).

/// One dispatched SSE event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, when the server sent one (Anthropic names its
    /// events; OpenAI does not).
    pub event: Option<String>,
    /// The `data:` payload; multiple data lines are joined with `\n`.
    pub data: String,
}

/// Incremental SSE parser: feed chunks, receive complete events.
#[derive(Debug, Default)]
pub struct SseParser {
    /// Bytes not yet terminated by a newline.
    pending: Vec<u8>,
    /// Data lines collected for the event being assembled.
    data_lines: Vec<String>,
    /// `event:` field collected for the event being assembled.
    event: Option<String>,
}

impl SseParser {
    /// A fresh parser.
    pub fn new() -> Self {
        SseParser::default()
    }

    /// Feed one body chunk; returns every event completed by it.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(pos) = self.pending.iter().position(|&byte| byte == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=pos).collect();
            line.pop(); // the '\n'
            if let Some(event) = self.line(&line) {
                events.push(event);
            }
        }
        events
    }

    /// Flush at end-of-body: process any unterminated final line, then
    /// dispatch a trailing event if one was left open (well-behaved servers
    /// end with a blank line; this covers the ones that do not).
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        let rest = std::mem::take(&mut self.pending);
        if !rest.is_empty() {
            if let Some(event) = self.line(&rest) {
                events.push(event);
            }
        }
        if let Some(event) = self.take_event() {
            events.push(event);
        }
        events
    }

    /// Process one complete (newline-stripped) line; `Some` when the line
    /// was the blank line that dispatches the assembled event.
    fn line(&mut self, raw: &[u8]) -> Option<SseEvent> {
        let mut bytes = raw;
        if bytes.last() == Some(&b'\r') {
            bytes = &bytes[..bytes.len() - 1];
        }
        let line = String::from_utf8_lossy(bytes);
        if line.is_empty() {
            return self.take_event();
        }
        if line.starts_with(':') {
            return None; // comment
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line.as_ref(), ""),
        };
        match field {
            "data" => self.data_lines.push(value.to_string()),
            "event" => self.event = Some(value.to_string()),
            // `id:`, `retry:`, unknown fields — ignored.
            _ => {}
        }
        None
    }

    /// Dispatch and reset the assembled event, if any data was collected.
    fn take_event(&mut self) -> Option<SseEvent> {
        if self.data_lines.is_empty() {
            self.event = None;
            return None;
        }
        Some(SseEvent {
            event: self.event.take(),
            data: std::mem::take(&mut self.data_lines).join("\n"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events_of(text: &str) -> Vec<SseEvent> {
        let mut parser = SseParser::new();
        let mut events = parser.feed(text.as_bytes());
        events.extend(parser.finish());
        events
    }

    #[test]
    fn parses_anthropic_event_shapes() {
        let text = "event: message_start\n\
                    data: {\"type\":\"message_start\"}\n\
                    \n\
                    event: content_block_delta\n\
                    data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\
                    \n";
        let events = events_of(text);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            SseEvent {
                event: Some("message_start".to_string()),
                data: "{\"type\":\"message_start\"}".to_string(),
            }
        );
        assert_eq!(events[1].event.as_deref(), Some("content_block_delta"));
    }

    #[test]
    fn parses_openai_done_sentinel() {
        let text = "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n\
                    data: [DONE]\n\n";
        let events = events_of(text);
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].data, "[DONE]");
        assert!(events[1].event.is_none());
    }

    #[test]
    fn chunks_may_split_lines_and_utf8() {
        let mut parser = SseParser::new();
        // Split mid-line, mid-field, and mid multi-byte char.
        let full = "data: {\"text\":\"héllo\"}\n\n";
        let (a, b) = full.split_at(9);
        let (b1, b2) = b.split_at(7);
        assert!(parser.feed(a.as_bytes()).is_empty());
        assert!(parser.feed(b1.as_bytes()).is_empty());
        let events = parser.feed(b2.as_bytes());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"text\":\"héllo\"}");
    }

    #[test]
    fn crlf_line_endings_dispatch() {
        let events = events_of("data: x\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn multiple_data_lines_join_with_newline() {
        let events = events_of("data: a\ndata: b\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "a\nb");
    }

    #[test]
    fn comments_and_ignored_fields_do_not_dispatch() {
        let events = events_of(": keep-alive\nid: 7\nretry: 100\n\n");
        assert!(events.is_empty());
    }

    #[test]
    fn event_field_does_not_survive_dispatch() {
        let events = events_of("event: one\ndata: a\n\ndata: b\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.as_deref(), Some("one"));
        assert_eq!(events[1].event, None);
    }

    #[test]
    fn finish_dispatches_event_without_trailing_blank_line() {
        let mut parser = SseParser::new();
        assert!(parser.feed(b"data: tail").is_empty());
        let events = parser.finish();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "tail");
    }

    #[test]
    fn colon_without_space_strips_nothing() {
        let events = events_of("data:x\n\n");
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn lone_event_field_is_not_dispatched() {
        let events = events_of("event: ping\n\n");
        assert!(events.is_empty());
    }
}
