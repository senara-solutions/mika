/// Install a tracing-aware panic hook that emits structured log events
/// before chaining to the previous (default) hook.
///
/// Must be called after tracing is initialized and before any
/// tokio tasks are spawned. This is the defense-in-depth layer for
/// spawned tasks whose `JoinHandle` is dropped — tokio's default
/// panic handler writes to stderr as raw text, bypassing the structured
/// JSON tracing layer. See mika#765.
pub fn install_tracing_panic_hook() {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        let payload = info.payload();
        let msg = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string payload>".to_string());

        tracing::error!(
            target: "panic",
            location = %location,
            message = %msg,
            thread = %std::thread::current().name().unwrap_or("<unnamed>"),
            event = "process_panic",
            "Rust panic caught by tracing hook"
        );

        // Chain to previous hook — preserves backtrace formatting,
        // test panic reporting, and any other registered handlers.
        prev_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;

    /// All tests in this module manipulate the process-global panic hook
    /// (`std::panic::set_hook`). Rust's test harness runs tests in parallel,
    /// so concurrent hook mutations cause race conditions. This mutex
    /// serializes all hook-manipulating tests.
    static HOOK_MUTEX: std::sync::LazyLock<Mutex<()>> = std::sync::LazyLock::new(|| Mutex::new(()));

    /// Helper: build a tracing subscriber that writes to an in-memory buffer.
    /// Returns (subscriber, buffer) where buffer can be read after the test.
    fn capturing_subscriber() -> (impl tracing::Subscriber + Send + Sync, Arc<Mutex<Vec<u8>>>) {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let writer = buf.clone();

        let layer = fmt::layer()
            .with_writer(move || {
                // tracing_subscriber calls this per-event to get a writer
                let w = writer.clone();
                WriterHandle(w)
            })
            .with_ansi(false)
            .with_target(true);

        let subscriber = tracing_subscriber::registry().with(layer);
        (subscriber, buf)
    }

    /// A `Write` impl that appends to a shared `Vec<u8>`.
    struct WriterHandle(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for WriterHandle {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_hook_chains_to_previous() {
        let _lock = HOOK_MUTEX.lock().unwrap();

        let chained = Arc::new(AtomicBool::new(false));
        let chained_clone = chained.clone();

        // Install a custom hook that sets the flag
        std::panic::set_hook(Box::new(move |_| {
            chained_clone.store(true, Ordering::SeqCst);
        }));

        // Install our tracing hook on top (chains to the flag-setting hook)
        install_tracing_panic_hook();

        // Trigger a panic and catch it
        let _ = std::panic::catch_unwind(|| {
            panic!("test chain");
        });

        assert!(
            chained.load(Ordering::SeqCst),
            "previous hook should have been called via chaining"
        );

        // Restore default hook so other tests aren't affected
        let _ = std::panic::take_hook();
    }

    #[test]
    fn test_str_payload_captured() {
        let _lock = HOOK_MUTEX.lock().unwrap();

        let (subscriber, buf) = capturing_subscriber();

        // Scope the subscriber so it's active during the panic
        let _guard = tracing::subscriber::set_default(subscriber);

        // Install a no-op previous hook so the default handler doesn't interfere
        std::panic::set_hook(Box::new(|_| {}));
        install_tracing_panic_hook();

        let _ = std::panic::catch_unwind(|| {
            panic!("str payload test");
        });

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(
            output.contains("process_panic"),
            "output should contain event name 'process_panic', got: {output}"
        );
        assert!(
            output.contains("str payload test"),
            "output should contain the panic message, got: {output}"
        );

        let _ = std::panic::take_hook();
    }

    #[test]
    fn test_string_payload_captured() {
        let _lock = HOOK_MUTEX.lock().unwrap();

        let (subscriber, buf) = capturing_subscriber();
        let _guard = tracing::subscriber::set_default(subscriber);

        std::panic::set_hook(Box::new(|_| {}));
        install_tracing_panic_hook();

        let _ = std::panic::catch_unwind(|| {
            panic!("{}", String::from("owned message test"));
        });

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(
            output.contains("process_panic"),
            "output should contain event name 'process_panic', got: {output}"
        );
        assert!(
            output.contains("owned message test"),
            "output should contain the owned string message, got: {output}"
        );

        let _ = std::panic::take_hook();
    }

    #[test]
    fn test_non_string_payload() {
        let _lock = HOOK_MUTEX.lock().unwrap();

        let (subscriber, buf) = capturing_subscriber();
        let _guard = tracing::subscriber::set_default(subscriber);

        std::panic::set_hook(Box::new(|_| {}));
        install_tracing_panic_hook();

        let _ = std::panic::catch_unwind(|| {
            std::panic::panic_any(42i32);
        });

        let binding = buf.lock().unwrap();
        let output = String::from_utf8_lossy(&binding);
        assert!(
            output.contains("process_panic"),
            "output should contain event name 'process_panic', got: {output}"
        );
        assert!(
            output.contains("<non-string payload>"),
            "output should contain non-string payload fallback, got: {output}"
        );

        let _ = std::panic::take_hook();
    }

    #[test]
    fn test_spawned_task_panic_reaches_hook() {
        // Verify the panic hook fires when a spawned task with a dropped
        // JoinHandle panics — the motivating use case for mika#765.
        //
        // Implementation note: we use std::thread::spawn rather than tokio::spawn
        // even though the production use case is tokio's worker pool. The
        // std::panic hook is process-wide (set_hook + take_hook operate on the
        // global hook), so the firing path is identical regardless of which
        // runtime spawned the panicking task. Using std::thread lets the test
        // be a plain #[test] (no async/.await), which means HOOK_MUTEX can be
        // held across the entire panic+join cycle — no race with parallel
        // tokio::tests that mutate the global hook during a tokio::sleep.
        //
        // We verify the hook chain fires (via AtomicBool) rather than capturing
        // tracing output, because set_global_default + tracing dispatch during
        // panic unwinding in the test harness is unreliable. The sync tests
        // above already verify tracing output capture.
        let hook_fired = Arc::new(AtomicBool::new(false));
        let hook_fired_clone = hook_fired.clone();

        let _lock = HOOK_MUTEX.lock().unwrap();

        // Install a flag-setting hook as the base, then layer our tracing hook
        std::panic::set_hook(Box::new(move |_| {
            hook_fired_clone.store(true, Ordering::SeqCst);
        }));
        install_tracing_panic_hook();

        // Spawn a thread that panics. join() waits for the panic to fully
        // unwind through the hook chain — deterministic, no sleep+race.
        let handle = std::thread::spawn(|| {
            panic!("spawned task panic");
        });
        let _ = handle.join();

        assert!(
            hook_fired.load(Ordering::SeqCst),
            "panic hook chain should fire when a spawned task with dropped handle panics"
        );

        let _ = std::panic::take_hook();
    }
}
