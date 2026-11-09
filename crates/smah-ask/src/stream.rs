//! Incremental tag filter for streamed answers: prose passes through to the
//! console as it arrives; `<exec>`/`<cot>` tag bodies are held back (exec
//! bodies surface as completed events for block rendering; cot bodies are
//! dropped). Split-delta safety: a `<` that might still be the start of a
//! managed tag is held until more bytes arrive or the stream ends.

/// One filtered stream event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// Prose safe to print now (never contains a managed tag).
    Prose(String),
    /// A completed `<exec>` tag body (raw inner text) — rendered as a
    /// suggestion block by the caller.
    Exec(String),
}

/// What the text at a `<` position is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TagKind {
    /// A managed tag by name (`exec` / `cot`).
    Managed(String),
    /// Definitely not a managed tag — treat the `<` as prose.
    NotManaged,
    /// Could still become a managed name once more bytes arrive — hold.
    Ambiguous,
}

fn classify_tag(from_tag: &str) -> TagKind {
    let after_lt = &from_tag[1..];
    let name_len = after_lt
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .unwrap_or(after_lt.len());
    let name = after_lt[..name_len].to_ascii_lowercase();
    if name == "exec" || name == "cot" {
        return TagKind::Managed(name);
    }
    // A name that runs to the end of the available text could still grow
    // into `exec`/`cot` — hold it. A terminated name is final.
    if name_len == after_lt.len() {
        return TagKind::Ambiguous;
    }
    TagKind::NotManaged
}

/// Stateful filter over text deltas.
#[derive(Debug, Default)]
pub struct StreamFilter {
    /// Unemitted bytes (a held partial tag, or prose awaiting classification).
    pending: String,
}

impl StreamFilter {
    /// A fresh filter.
    pub fn new() -> Self {
        StreamFilter::default()
    }

    /// Feed one delta; returns the events now safe to surface.
    pub fn feed(&mut self, delta: &str) -> Vec<StreamEvent> {
        self.pending.push_str(delta);
        let mut events = Vec::new();
        while let Some(event) = self.drain_once() {
            events.push(event);
        }
        events
    }

    /// End of stream: flushes held bytes as prose. A never-closed managed
    /// tag degrades to prose (same rule as the batch parser).
    pub fn finish(&mut self) -> Vec<StreamEvent> {
        let rest = std::mem::take(&mut self.pending);
        if rest.is_empty() {
            Vec::new()
        } else {
            vec![StreamEvent::Prose(rest)]
        }
    }

    /// Make one unit of progress on [`Self::pending`]; `None` = nothing
    /// more can be decided yet.
    fn drain_once(&mut self) -> Option<StreamEvent> {
        let pending = self.pending.clone();
        if pending.is_empty() {
            return None;
        }
        // Find the first `<` that is (or could become) a managed tag —
        // earlier not-managed `<`s are plain prose.
        let mut search_from = 0usize;
        let tag_start = loop {
            match pending[search_from..].find('<') {
                Some(offset) => {
                    let at = offset + search_from;
                    if classify_tag(&pending[at..]) == TagKind::NotManaged {
                        search_from = at + 1;
                    } else {
                        break at;
                    }
                }
                None => {
                    // No managed tag pending: everything is prose.
                    self.pending = String::new();
                    return Some(StreamEvent::Prose(pending));
                }
            }
        };
        let (before, from_tag) = pending.split_at(tag_start);
        match classify_tag(from_tag) {
            TagKind::NotManaged => unreachable!("search skips not-managed tags"),
            TagKind::Ambiguous => {
                if before.is_empty() {
                    None
                } else {
                    self.pending = from_tag.to_string();
                    Some(StreamEvent::Prose(before.to_string()))
                }
            }
            TagKind::Managed(name) => {
                if !before.is_empty() {
                    // Prose first; the tag waits for the next drain pass.
                    self.pending = from_tag.to_string();
                    return Some(StreamEvent::Prose(before.to_string()));
                }
                // Need the full opening tag before scanning for the close.
                let open_end = from_tag.find('>')? + 1;
                let close = format!("</{name}>");
                let body_region = &from_tag[open_end..];
                let close_idx = body_region.find(close.as_str())?;
                let body = body_region[..close_idx].to_string();
                self.pending = body_region[close_idx + close.len()..].to_string();
                Some(if name == "exec" {
                    StreamEvent::Exec(body)
                } else {
                    StreamEvent::Prose(String::new())
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prose(events: &[StreamEvent]) -> String {
        events
            .iter()
            .map(|e| match e {
                StreamEvent::Prose(text) => text.clone(),
                StreamEvent::Exec(_) => String::new(),
            })
            .collect()
    }

    #[test]
    fn plain_prose_passes_through() {
        let mut filter = StreamFilter::new();
        let events = filter.feed("hello world");
        assert_eq!(prose(&events), "hello world");
    }

    #[test]
    fn exec_held_until_closed() {
        let mut filter = StreamFilter::new();
        let e1 = filter.feed("answer:\n<exec><title>t</title>");
        assert_eq!(prose(&e1), "answer:\n");
        let e2 = filter.feed("<command>ls</command></exec>done");
        assert_eq!(
            e2,
            vec![
                StreamEvent::Exec("<title>t</title><command>ls</command>".to_string()),
                StreamEvent::Prose("done".to_string()),
            ]
        );
    }

    #[test]
    fn split_tag_across_deltas() {
        let mut filter = StreamFilter::new();
        assert_eq!(
            filter.feed("one <e"),
            vec![StreamEvent::Prose("one ".to_string())]
        );
        assert!(filter.feed("xec>").is_empty());
        assert_eq!(filter.feed("<command>x</command></exec>").len(), 1);
    }

    #[test]
    fn literal_lt_not_a_tag() {
        let mut filter = StreamFilter::new();
        assert_eq!(
            filter.feed("a < b and c > d"),
            vec![StreamEvent::Prose("a < b and c > d".to_string())]
        );
    }

    #[test]
    fn unknown_tag_passes_verbatim() {
        let mut filter = StreamFilter::new();
        let events = filter.feed("x<smah-foo>bar</smah-foo>y");
        assert_eq!(prose(&events), "x<smah-foo>bar</smah-foo>y");
    }

    #[test]
    fn cot_dropped() {
        let mut filter = StreamFilter::new();
        let events = filter.feed("x<cot>reasoning</cot>y");
        assert_eq!(prose(&events), "xy");
    }

    #[test]
    fn unclosed_exec_flushes_as_prose() {
        let mut filter = StreamFilter::new();
        filter.feed("ok <exec><command>half");
        let events = filter.finish();
        assert_eq!(prose(&events), "<exec><command>half");
    }

    #[test]
    fn char_by_char_streaming() {
        let text = "lead <exec><command>rm -rf /</command></exec> tail";
        let mut filter = StreamFilter::new();
        let mut collected = String::new();
        let mut execs = 0;
        for ch in text.chars() {
            for event in filter.feed(ch.to_string().as_str()) {
                match event {
                    StreamEvent::Prose(p) => collected.push_str(&p),
                    StreamEvent::Exec(_) => execs += 1,
                }
            }
        }
        for event in filter.finish() {
            if let StreamEvent::Prose(p) = event {
                collected.push_str(&p);
            }
        }
        assert_eq!(collected, "lead  tail");
        assert_eq!(execs, 1);
    }
}
