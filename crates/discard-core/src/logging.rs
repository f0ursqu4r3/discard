//! Structured logging with secret redaction. Every log line passes through `redact`.

use std::io::{self, Write};
use tracing_subscriber::fmt::MakeWriter;

/// Wraps a writer and redacts secrets in everything written through it.
/// `tracing`'s formatter writes each event in one call, so a secret is not split across calls.
pub struct RedactingWriter<W>(pub W);

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let clean = crate::redact::redact(&String::from_utf8_lossy(buf));
        self.0.write_all(clean.as_bytes())?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// `MakeWriter` that redacts. Use with `tracing_subscriber::fmt().with_writer(...)`.
pub struct RedactingMakeWriter<M>(pub M);

impl<'a, M: MakeWriter<'a>> MakeWriter<'a> for RedactingMakeWriter<M> {
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter(self.0.make_writer())
    }
}

/// Install the global subscriber: JSON lines on stderr, redacted, filtered by `RUST_LOG`
/// (default `info`). Safe to call once at startup.
pub fn init() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    // `try_init` so a second call (or a test harness subscriber) is not a panic.
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(RedactingMakeWriter(io::stderr))
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Write for Capture {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    #[test]
    fn the_writer_redacts_and_reports_the_original_length() {
        let cap = Capture::default();
        let mut w = RedactingWriter(cap.clone());
        let input = b"password=hunter2 ok\n";
        assert_eq!(w.write(input).unwrap(), input.len());
        assert_eq!(cap.text(), "password=[REDACTED] ok\n");
    }

    #[test]
    fn events_keep_structured_ids_but_lose_secrets() {
        let cap = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(RedactingMakeWriter(cap.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(session = "s-1", turn = "t-1", "provider said api_key=sk-live-secret-value-12345");
        });
        let out = cap.text();
        assert!(out.contains("\"session\":\"s-1\""), "{out}");
        assert!(out.contains("\"turn\":\"t-1\""), "{out}");
        assert!(!out.contains("sk-live-secret"), "{out}");
        assert!(out.contains("[REDACTED]"), "{out}");
    }
}
