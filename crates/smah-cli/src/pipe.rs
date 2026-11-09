//! Bounded stdin plumbing (RA-10 F4). The model-facing input can never grow
//! unbounded in memory: reads stop retaining at `cap` bytes while the pipe
//! is still fully **drained** in fixed chunks so a fast producer cannot
//! block on a full pipe — over-cap bytes are counted, never buffered.
//! Truncation stays loud (`smah ask` reports `(sent, total)` on stderr and
//! the model is told).

use std::io::Read;

/// Real-stdin wrapper over [`read_stdin_from`]: `is_tty` short-circuits
/// (a TTY is never read as a pipe). `None` truncation means nothing was
/// read (TTY or read error); `Some((sent, total))` marks a loud
/// truncation.
pub fn read_stdin(cap: usize, is_tty: bool) -> (Option<String>, Option<(usize, usize)>) {
    if is_tty {
        return (None, None);
    }
    let mut stdin = std::io::stdin().lock();
    let (text, total) = read_stdin_from(&mut stdin, cap);
    let truncation = (total > cap).then_some((text.len(), total));
    (Some(text), truncation)
}

/// Capped read of an arbitrary reader: (lossy-decoded capped text,
/// uncapped byte total). At most `cap` bytes are retained; the rest is
/// drained in fixed-size chunks and only counted.
pub fn read_stdin_from<R: Read>(reader: &mut R, cap: usize) -> (String, usize) {
    let mut raw: Vec<u8> = Vec::with_capacity(cap.min(1 << 20));
    let mut buf = [0u8; 16_384];
    let mut total = 0usize;
    let mut kept = 0usize;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if kept < cap {
                    let take = (cap - kept).min(n);
                    raw.extend_from_slice(&buf[..take]);
                    kept += take;
                }
                // over-cap bytes: counted above, dropped here
            }
            Err(_) => break,
        }
    }
    (String::from_utf8_lossy(&raw).to_string(), total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_stdin_is_never_read() {
        assert_eq!(read_stdin(10, true), (None, None));
    }

    #[test]
    fn pipe_stdin_read_and_truncated_at_cap() {
        let data = "x".repeat(100);
        let (text, total) = read_stdin_from(&mut data.as_bytes(), 16);
        assert_eq!(text, "x".repeat(16));
        assert_eq!(total, 100);
    }

    #[test]
    fn pipe_stdin_under_cap_read_fully() {
        let (text, total) = read_stdin_from(&mut "hello".as_bytes(), 16);
        assert_eq!(text, "hello");
        assert_eq!(total, 5);
    }

    #[test]
    fn capped_read_reports_total() {
        let mut data = std::io::Cursor::new(b"0123456789".to_vec());
        let (text, total) = read_stdin_from(&mut data, 4);
        assert_eq!(text, "0123");
        assert_eq!(total, 10);
    }

    #[test]
    fn under_cap_read_is_whole() {
        let mut data = std::io::Cursor::new(b"hi".to_vec());
        let (text, total) = read_stdin_from(&mut data, 16);
        assert_eq!(text, "hi");
        assert_eq!(total, 2);
    }

    /// F4: the drain counts far-over-cap input without retaining it (a
    /// chunked read loop, so the buffer stays ~cap-sized regardless of
    /// producer volume).
    #[test]
    fn far_over_cap_drains_and_counts() {
        let chunk = "y".repeat(100_000);
        let mut data = std::io::Cursor::new(chunk.repeat(30)); // 3 MB
        let (text, total) = read_stdin_from(&mut data, 1_000);
        assert_eq!(text.len(), 1_000);
        assert_eq!(total, 3_000_000);
        assert!(text.chars().all(|c| c == 'y'));
    }

    #[test]
    fn empty_pipe_is_empty_string_zero_total() {
        let mut data = std::io::empty();
        let (text, total) = read_stdin_from(&mut data, 16);
        assert_eq!(text, "");
        assert_eq!(total, 0);
    }
}
