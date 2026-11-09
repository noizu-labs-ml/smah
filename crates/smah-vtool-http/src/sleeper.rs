//! Interruptible wall-clock waits for the web driver's `--watch` loop
//! (#19): sleep for N seconds OR stop early on ctrl-c (or an injected stop
//! signal, for tests). Lives here because smah-core has no tokio dependency.

use std::time::Duration;

/// Why a wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// The window elapsed — keep going.
    Elapsed,
    /// ctrl-c or the injected stop signal fired — stop the loop.
    Stop,
}

/// A reusable runtime for interruptible sleeps.
pub struct Sleeper {
    rt: tokio::runtime::Runtime,
}

impl Sleeper {
    /// Build the sleeper runtime (current-thread, io + time enabled so the
    /// ctrl-c signal handler works).
    pub fn new() -> std::io::Result<Self> {
        Ok(Sleeper {
            rt: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
        })
    }

    /// Sleep `seconds`, or return early on ctrl-c. `Wake::Stop` = stop the
    /// watch loop, `Wake::Elapsed` = the window elapsed (keep watching).
    pub fn wait(&self, seconds: u64) -> Wake {
        self.wait_for(seconds, None)
    }

    /// [`wait`] with an injected stop signal (`Notify::notify_one` ends the
    /// wait early) — how tests bound the web driver's watch loop without a
    /// real ctrl-c.
    pub fn wait_for(&self, seconds: u64, stop: Option<&tokio::sync::Notify>) -> Wake {
        self.rt.block_on(async {
            let sleep = tokio::time::sleep(Duration::from_secs(seconds));
            tokio::pin!(sleep);
            match stop {
                Some(notify) => {
                    tokio::select! {
                        _ = &mut sleep => Wake::Elapsed,
                        _ = tokio::signal::ctrl_c() => Wake::Stop,
                        _ = notify.notified() => Wake::Stop,
                    }
                }
                None => {
                    tokio::select! {
                        _ = &mut sleep => Wake::Elapsed,
                        _ = tokio::signal::ctrl_c() => Wake::Stop,
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_wait_returns_elapsed_immediately() {
        let sleeper = Sleeper::new().expect("sleeper");
        let start = std::time::Instant::now();
        assert_eq!(sleeper.wait(0), Wake::Elapsed);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn short_wait_elapses() {
        let sleeper = Sleeper::new().expect("sleeper");
        let start = std::time::Instant::now();
        assert_eq!(sleeper.wait(1), Wake::Elapsed);
        assert!(start.elapsed() >= Duration::from_millis(900));
    }

    #[test]
    fn injected_stop_ends_the_wait_early() {
        let sleeper = Sleeper::new().expect("sleeper");
        let stop = std::sync::Arc::new(tokio::sync::Notify::new());
        let stop_now = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            stop_now.notify_one();
        });
        let start = std::time::Instant::now();
        assert_eq!(sleeper.wait_for(30, Some(&stop)), Wake::Stop);
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
