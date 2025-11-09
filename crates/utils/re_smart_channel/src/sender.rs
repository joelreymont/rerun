use std::sync::{Arc, atomic::Ordering::Relaxed};

use web_time::Instant;

use crate::{SendError, SharedStats, SizeBytes, SmartMessage, SmartMessagePayload, SmartMessageSource};

#[derive(Clone)]
pub struct Sender<T: Send> {
    tx: crossbeam::channel::Sender<SmartMessage<T>>,
    source: Arc<SmartMessageSource>,
    stats: Arc<SharedStats>,
}

impl<T: Send> Sender<T> {
    pub(crate) fn new(
        tx: crossbeam::channel::Sender<SmartMessage<T>>,
        source: Arc<SmartMessageSource>,
        stats: Arc<SharedStats>,
    ) -> Self {
        Self { tx, source, stats }
    }

    /// Clones the sender with an updated source.
    pub fn clone_as(&self, source: SmartMessageSource) -> Self {
        Self {
            tx: self.tx.clone(),
            source: Arc::new(source),
            stats: Arc::clone(&self.stats),
        }
    }

    pub fn send(&self, msg: T) -> Result<(), SendError<T>>
    where
        T: crate::SizeBytes,
    {
        let smart_msg = SmartMessage {
            time: Instant::now(),
            source: Arc::clone(&self.source),
            payload: SmartMessagePayload::Msg(msg),
        };

        let size = smart_msg.total_size_bytes();

        self.send_at_with_size(
            smart_msg.time,
            smart_msg.source,
            smart_msg.payload,
            size,
        )
        .map_err(|SendError(payload)| match payload {
            SmartMessagePayload::Msg(msg) => SendError(msg),
            SmartMessagePayload::Flush { .. } | SmartMessagePayload::Quit(_) => unreachable!(),
        })
    }

    /// Forwards a message as-is.
    pub fn send_at(
        &self,
        time: Instant,
        source: Arc<SmartMessageSource>,
        payload: SmartMessagePayload<T>,
    ) -> Result<(), SendError<SmartMessagePayload<T>>> {
        // NOTE: We should never be sending a message with an unknown source.
        debug_assert!(!matches!(*source, SmartMessageSource::Unknown));

        self.tx
            .send(SmartMessage {
                time,
                source,
                payload,
            })
            .map_err(|SendError(msg)| SendError(msg.payload))
    }

    /// Forwards a message as-is, tracking the given size in bytes.
    ///
    /// This is used internally when the size is known. For types that implement
    /// [`SizeBytes`], use [`Self::send`] which automatically calculates the size.
    pub fn send_at_with_size(
        &self,
        time: Instant,
        source: Arc<SmartMessageSource>,
        payload: SmartMessagePayload<T>,
        size_bytes: u64,
    ) -> Result<(), SendError<SmartMessagePayload<T>>> {
        // NOTE: We should never be sending a message with an unknown source.
        debug_assert!(!matches!(*source, SmartMessageSource::Unknown));

        // Track the size before sending
        self.stats.queue_bytes.fetch_add(size_bytes, Relaxed);

        match self.tx.send(SmartMessage {
            time,
            source,
            payload,
        }) {
            Ok(()) => Ok(()),
            Err(SendError(msg)) => {
                // If send failed, undo the size tracking
                self.stats.queue_bytes.fetch_sub(size_bytes, Relaxed);
                Err(SendError(msg.payload))
            }
        }
    }

    /// Blocks until all previously sent messages have been received.
    ///
    /// Note: This is only implemented for non-wasm targets since we cannot make
    /// blocking calls on web.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn flush_blocking(&self, timeout: std::time::Duration) -> Result<(), crate::FlushError> {
        use crate::FlushError;

        let (tx, rx) = std::sync::mpsc::sync_channel(0); // oneshot
        self.tx
            .send(SmartMessage {
                time: Instant::now(),
                source: Arc::clone(&self.source),
                payload: SmartMessagePayload::Flush {
                    on_flush_done: Box::new(move || {
                        tx.send(()).ok();
                    }),
                },
            })
            .map_err(|_ignored| FlushError::Closed)?;

        rx.recv_timeout(timeout).map_err(|err| match err {
            std::sync::mpsc::RecvTimeoutError::Timeout => FlushError::Timeout,
            std::sync::mpsc::RecvTimeoutError::Disconnected => FlushError::Closed,
        })
    }

    /// Used to indicate that a sender has left.
    ///
    /// This sends a message down the channel allowing the receiving end to know whether one of the
    /// sender has left, and if so why (if applicable).
    ///
    /// Using a [`Sender`] after calling `quit` is undefined behavior: the receiving end is free
    /// to silently drop those messages (or worse).
    pub fn quit(
        &self,
        err: Option<Box<dyn std::error::Error + Send>>,
    ) -> Result<(), SendError<SmartMessage<T>>> {
        // NOTE: We should never be sending a message with an unknown source.
        debug_assert!(!matches!(*self.source, SmartMessageSource::Unknown));

        self.tx.send(SmartMessage {
            time: Instant::now(),
            source: Arc::clone(&self.source),
            payload: SmartMessagePayload::Quit(err),
        })
    }

    /// Is the channel currently empty of messages?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tx.is_empty()
    }

    /// Number of messages in the channel right now.
    #[inline]
    pub fn len(&self) -> usize {
        self.tx.len()
    }

    /// Latest known latency from sending a message to receiving it, it nanoseconds.
    pub fn latency_nanos(&self) -> u64 {
        self.stats.latency_nanos.load(Relaxed)
    }

    /// Latest known latency from sending a message to receiving it,
    /// in seconds
    pub fn latency_sec(&self) -> f32 {
        self.latency_nanos() as f32 / 1e9
    }

    /// Total bytes currently queued in the channel.
    ///
    /// This is only accurate for types that implement [`SizeBytes`].
    /// For other types, this will return 0.
    pub fn queue_bytes(&self) -> u64 {
        self.stats.queue_bytes.load(Relaxed)
    }
}

// Additional implementations for types that support size tracking
impl<T: Send + SizeBytes> Sender<T> {
    /// Send a message, automatically tracking its size.
    pub fn send_tracking(&self, msg: T) -> Result<(), SendError<T>> {
        let smart_msg = SmartMessage {
            time: Instant::now(),
            source: Arc::clone(&self.source),
            payload: SmartMessagePayload::Msg(msg),
        };

        let size = smart_msg.total_size_bytes();

        self.send_at_with_size(
            smart_msg.time,
            smart_msg.source,
            smart_msg.payload,
            size,
        )
        .map_err(|SendError(payload)| match payload {
            SmartMessagePayload::Msg(msg) => SendError(msg),
            SmartMessagePayload::Flush { .. } | SmartMessagePayload::Quit(_) => unreachable!(),
        })
    }

    /// Forwards a message as-is, automatically calculating and tracking its size.
    pub fn send_at_tracking(
        &self,
        time: Instant,
        source: Arc<SmartMessageSource>,
        payload: SmartMessagePayload<T>,
    ) -> Result<(), SendError<SmartMessagePayload<T>>> {
        let smart_msg = SmartMessage {
            time,
            source,
            payload,
        };

        let size = smart_msg.total_size_bytes();

        self.send_at_with_size(
            smart_msg.time,
            smart_msg.source,
            smart_msg.payload,
            size,
        )
    }
}
