/// Background worker for ingesting Arrow messages.
///
/// This module provides a dedicated background thread that processes Arrow messages
/// into chunks, moving CPU-intensive work off the UI thread. Uses a bounded channel
/// to provide backpressure.

use std::sync::Arc;

use re_log_types::{ArrowMsg, StoreId};
use re_smart_channel::SmartChannelSource;

/// Maximum number of pending work items before backpressure kicks in.
/// This prevents unbounded memory growth while allowing sufficient buffering.
const WORK_QUEUE_CAPACITY: usize = 2000;

/// Work item to be processed by the ingestion worker.
struct WorkItem {
    store_id: StoreId,
    arrow_msg: ArrowMsg,
    channel_source: Arc<SmartChannelSource>,
    msg_will_add_new_store: bool,
}

/// Result of processing a work item.
pub struct ProcessedChunk {
    pub store_id: StoreId,
    pub chunk: Arc<re_chunk::Chunk>,
    pub timestamps: re_sorbet::TimestampMetadata,
    pub channel_source: Arc<SmartChannelSource>,
    pub msg_will_add_new_store: bool,
}

/// Background worker for processing Arrow messages into chunks.
///
/// Runs on a dedicated thread and provides backpressure via bounded channels.
pub struct IngestionWorker {
    input_tx: crossbeam::channel::Sender<WorkItem>,
    output_rx: crossbeam::channel::Receiver<ProcessedChunk>,
    #[allow(dead_code)] // Kept alive for thread lifecycle
    worker_thread: Option<std::thread::JoinHandle<()>>,
}

impl IngestionWorker {
    /// Create a new ingestion worker with a dedicated background thread.
    pub fn new() -> Self {
        let (input_tx, input_rx) = crossbeam::channel::bounded::<WorkItem>(WORK_QUEUE_CAPACITY);
        let (output_tx, output_rx) = crossbeam::channel::unbounded::<ProcessedChunk>();

        let worker_thread = std::thread::Builder::new()
            .name("ingestion_worker".to_owned())
            .spawn(move || {
                Self::worker_loop(input_rx, output_tx);
            })
            .expect("Failed to spawn ingestion worker thread");

        Self {
            input_tx,
            output_rx,
            worker_thread: Some(worker_thread),
        }
    }

    /// Submit an arrow message for processing, blocking if necessary.
    pub fn submit_arrow_msg_blocking(
        &self,
        store_id: StoreId,
        arrow_msg: ArrowMsg,
        channel_source: Arc<SmartChannelSource>,
        msg_will_add_new_store: bool,
    ) {
        let work_item = WorkItem {
            store_id,
            arrow_msg,
            channel_source,
            msg_will_add_new_store,
        };

        // Block until we can send
        if let Err(e) = self.input_tx.send(work_item) {
            re_log::warn!("Failed to send to ingestion worker: {e}");
        }
    }

    /// Poll for processed chunks. Returns None if no chunks are ready.
    pub fn poll_processed_chunks(&self) -> Vec<ProcessedChunk> {
        let mut chunks = Vec::new();

        // Drain all available processed chunks without blocking
        while let Ok(chunk) = self.output_rx.try_recv() {
            chunks.push(chunk);
        }

        chunks
    }

    /// Main worker loop that processes arrow messages.
    fn worker_loop(
        input_rx: crossbeam::channel::Receiver<WorkItem>,
        output_tx: crossbeam::channel::Sender<ProcessedChunk>,
    ) {
        re_log::debug!("Ingestion worker thread started");

        while let Ok(work_item) = input_rx.recv() {
            re_tracing::profile_scope!("process_arrow_msg");

            let WorkItem {
                store_id,
                arrow_msg,
                channel_source,
                msg_will_add_new_store,
            } = work_item;

            // Do the work of converting Arrow data to chunks
            let result = Self::process_arrow_msg(&arrow_msg);

            match result {
                Ok((chunk, timestamps)) => {
                    let processed = ProcessedChunk {
                        store_id,
                        chunk: Arc::new(chunk),
                        timestamps,
                        channel_source,
                        msg_will_add_new_store,
                    };

                    if output_tx.send(processed).is_err() {
                        // Main thread has disconnected, time to exit
                        break;
                    }
                }
                Err(err) => {
                    re_log::warn_once!("Failed to process arrow message: {err}");
                }
            }
        }

        re_log::debug!("Ingestion worker thread exiting");
    }

    /// Process an arrow message into a chunk.
    ///
    /// This is the work that we want to do off the main thread.
    fn process_arrow_msg(
        arrow_msg: &ArrowMsg,
    ) -> re_entity_db::Result<(re_chunk::Chunk, re_sorbet::TimestampMetadata)> {
        re_tracing::profile_function!();

        let chunk_batch = re_sorbet::ChunkBatch::try_from(&arrow_msg.batch)
            .map_err(re_chunk::ChunkError::from)?;
        let mut chunk = re_chunk::Chunk::from_chunk_batch(&chunk_batch)?;
        chunk.sort_if_unsorted();

        Ok((chunk, chunk_batch.sorbet_schema().timestamps.clone()))
    }
}

impl Drop for IngestionWorker {
    fn drop(&mut self) {
        // Dropping input_tx will cause the worker thread to exit gracefully
        // when it finishes processing remaining items
        re_log::debug!("Dropping ingestion worker");
    }
}
