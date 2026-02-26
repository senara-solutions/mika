use crossterm::event::{self, Event as CrosstermEvent, KeyEvent};
use std::time::Duration;
use tokio::sync::mpsc;

/// Application events.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Paste(String),
    Tick,
    Resize,
}

/// Async event reader that polls crossterm events.
pub struct EventReader {
    rx: mpsc::UnboundedReceiver<AppEvent>,
    thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl EventReader {
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();

        // Spawn a dedicated thread for crossterm event polling.
        // crossterm::event::poll is blocking, so we use a real thread.
        let thread_handle = std::thread::Builder::new()
            .name("mika-events".to_string())
            .spawn(move || {
                loop {
                    if event::poll(tick_rate).unwrap_or(false) {
                        match event::read() {
                            Ok(CrosstermEvent::Key(key)) => {
                                if tx.send(AppEvent::Key(key)).is_err() {
                                    return;
                                }
                            }
                            Ok(CrosstermEvent::Paste(text)) => {
                                if tx.send(AppEvent::Paste(text)).is_err() {
                                    return;
                                }
                            }
                            Ok(CrosstermEvent::Resize(_, _)) => {
                                if tx.send(AppEvent::Resize).is_err() {
                                    return;
                                }
                            }
                            _ => {}
                        }
                    } else {
                        // Tick event on timeout
                        if tx.send(AppEvent::Tick).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("failed to spawn event reader thread");

        Self {
            rx,
            thread_handle: Some(thread_handle),
        }
    }

    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }

    /// Shut down the event reader, joining the background thread.
    /// The thread will exit once the channel sender is dropped (when rx is dropped).
    pub fn shutdown(mut self) {
        // Drop the receiver so the sender detects a closed channel and exits.
        drop(self.rx);
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}
