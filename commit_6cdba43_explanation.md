# Commit 6cdba43: Parallel RRD Ingestion Architecture

**Commit Hash:** `6cdba43c4b7919a6fe5e27ce26537ea4f8fa2847`
**Author:** Joel Reymont
**Date:** November 7, 2025
**Title:** Redesign parallel RRD ingestion with per-EntityDb workers

## Executive Summary

This commit implements a fundamental architectural redesign of how Rerun processes incoming data. It moves from a single, application-level ingestion worker to a **per-EntityDb worker architecture**, enabling true parallel processing of data across multiple entity databases. This change significantly improves performance when loading large RRD (Rerun Recording Data) files.

---

## Key Architectural Changes

### 1. Per-EntityDb Workers (Native Platforms)

**Before:** A single ingestion worker at the application level processed all incoming data sequentially.

**After:** Each `EntityDb` instance now owns its own dedicated ingestion worker with:
- Separate channels for commands and message batches
- Independent background thread for parallel processing
- Per-database backpressure management

**Location:** `crates/store/re_entity_db/src/entity_db.rs`

```rust
pub struct EntityDb {
    // ... existing fields ...

    /// Background worker for processing Arrow messages (native only).
    /// Lazily initialized on first Arrow message.
    #[cfg(not(target_arch = "wasm32"))]
    ingestion_worker: Option<crate::ingestion_worker::IngestionWorker>,
}
```

**Why this matters:**
- Multiple entity databases can process data **simultaneously** instead of waiting in a queue
- Better CPU utilization on multi-core systems
- Reduced latency when loading multiple recordings

---

### 2. New Ingestion Worker Module

**File:** `crates/store/re_entity_db/src/ingestion_worker.rs` (new file, 655 lines)

This module implements the core worker logic with platform-specific behavior:

#### Platform Support

**Native (Linux/macOS/Windows):**
- Uses dedicated background threads via `crossbeam::channel`
- Bounded channels (capacity: 1024) for backpressure
- Blocks on submission when channel is full to prevent memory exhaustion

**WASM:**
- Processes synchronously (no threads available in browser)
- No-op worker implementation

#### Key Components

1. **ProcessedChunk Structure**
   ```rust
   pub struct ProcessedChunk {
       pub chunk: Arc<Chunk>,
       pub timestamps: re_sorbet::TimestampMetadata,
       pub channel_source: Arc<re_smart_channel::SmartChannelSource>,
       pub msg_will_add_new_store: bool,
   }
   ```
   Encapsulates a fully processed chunk with all necessary metadata.

2. **Worker Thread Loop**
   - Receives Arrow messages from the main thread
   - Converts Arrow format → ChunkBatch → Chunk
   - Sorts chunks if needed
   - Sends processed chunks back to main thread

3. **Message Flow**
   ```
   Main Thread                     Worker Thread
   -----------                     -------------
   ArrowMsg received
        ↓
   submit_arrow_msg_blocking() →  [Channel Queue]
   (blocks if full)                      ↓
                                   Convert to Chunk
                                   Sort if needed
                                         ↓
   poll_processed_chunks()     ←  [Output Queue]
        ↓
   add_chunk_to_store()
   ```

---

### 3. Message Transformation Refactoring

**File:** `crates/store/re_data_loader/src/loader_rrd.rs`

The message processing logic was extracted into a separate `transform_message()` function to improve code organization:

**Before:** Transformation logic was embedded in the message processing loop
**After:** Clean separation of concerns

```rust
fn transform_message(
    msg: re_log_types::LogMsg,
    forced_application_id: Option<&ApplicationId>,
    forced_recording_id: Option<&String>,
) -> re_log_types::LogMsg {
    // Handles ID transformation for .rbl (blueprint) files
}
```

**Purpose:** When loading `.rbl` (blueprint) files, store IDs must be transformed to match the opened store. This refactoring makes the transformation logic reusable and testable.

---

### 4. Message Ordering Guarantees

A critical requirement for correctness is preserving message order during parallel processing. Two comprehensive tests were added:

#### Test 1: `test_message_order_preserved`
- Creates 500 sequential messages with embedded sequence numbers
- Encodes to RRD file
- Loads via parallel pipeline
- Verifies sequence numbers arrive in correct order

#### Test 2: `test_message_order_preserved_with_transformation`
- Tests 250 messages with ID transformation applied (.rbl files)
- Ensures transformation doesn't break ordering
- Critical for blueprint loading

**Why this matters:** Out-of-order messages could cause temporal inconsistencies in the visualization, showing data at wrong timestamps.

---

### 5. Benchmark Infrastructure

**File:** `crates/store/re_data_loader/benches/parallel_ingestion_bench.rs` (new file)

A comprehensive benchmarking suite was added to measure ingestion performance:

#### Features
- Uses `criterion` for statistical benchmarking
- Uses `mimalloc` allocator for consistent performance
- Generates 10,000 messages in release, 100 in debug
- Measures throughput in messages/second

#### Benchmark Design
```rust
fn benchmark_load_from_file_contents(c: &mut Criterion) {
    // Generate synthetic data
    let messages = generate_messages(NUM_MESSAGES);
    let encoded = encode_messages(&messages);

    // Benchmark the loading pipeline
    group.bench_function("rrd_loader", |b| {
        b.iter(|| {
            // Full load from file contents
            loader.load_from_file_contents(...);
            // Verify all messages received
        });
    });
}
```

**Metrics tracked:**
- Throughput: Elements (messages) per second
- Latency: Time per iteration
- Memory allocation patterns (via mimalloc)

---

## Implementation Details

### EntityDb Changes

1. **Custom Clone Implementation**
   - `EntityDb` is no longer automatically cloneable
   - Custom implementation skips `ingestion_worker` (thread handles can't be cloned)
   - Useful for test scenarios

2. **Public API Additions**

   **`submit_arrow_msg()` (native only)**
   ```rust
   pub fn submit_arrow_msg(
       &mut self,
       arrow_msg: re_log_types::ArrowMsg,
       channel_source: Arc<re_smart_channel::SmartChannelSource>,
       msg_will_add_new_store: bool,
   )
   ```
   - Submits Arrow message to background worker
   - Blocks if worker queue is full (backpressure)
   - Lazily initializes worker on first call

   **`poll_worker_output()` (native only)**
   ```rust
   pub fn poll_worker_output(&mut self)
       -> Vec<(ProcessedChunk, bool, Result<Vec<ChunkStoreEvent>, Error>)>
   ```
   - Retrieves processed chunks from worker
   - Adds chunks to store
   - Returns events for UI updates

3. **Made Public:** `add_chunk_with_timestamp_metadata()`
   - Previously private
   - Now public to support worker output processing

---

## Performance Considerations

### Benefits

1. **Parallel Processing**
   - Multiple EntityDbs process simultaneously
   - Better multi-core CPU utilization
   - Reduced wall-clock time for loading

2. **Backpressure Management**
   - Bounded channels (1024 capacity) prevent unbounded memory growth
   - Blocking submission ensures producer doesn't overwhelm consumer

3. **Lazy Initialization**
   - Workers only created when needed
   - No overhead for EntityDbs that don't receive Arrow messages

### Trade-offs

1. **Thread Overhead**
   - Each EntityDb spawns a thread (on native)
   - For many small EntityDbs, this could be expensive
   - Mitigated by lazy initialization

2. **Memory Usage**
   - Each worker has bounded queues (1024 × message size)
   - Multiple workers = multiple queues
   - Acceptable trade-off for performance gain

3. **WASM Limitations**
   - No parallel processing on WASM (single-threaded)
   - Falls back to synchronous processing
   - Same correctness, lower performance

---

## Testing Strategy

### Unit Tests

1. **Message Order Preservation**
   - 500 messages with sequence numbers
   - Verifies no reordering during parallel processing

2. **Transform + Order**
   - 250 messages with ID transformation
   - Ensures transformation doesn't break ordering

### Benchmarks

1. **End-to-End Pipeline**
   - Measures full ingestion performance
   - Generates realistic synthetic data
   - Statistical analysis via Criterion

### Manual Testing
- Load large RRD files (implied by commit message)
- Verify visual correctness
- Monitor CPU usage during loading

---

## Dependencies Added

### Cargo.toml Changes

**`re_data_loader/Cargo.toml`:**
- `criterion` - Industry-standard Rust benchmarking
- `mimalloc` - High-performance allocator for consistent benchmarks

**`re_entity_db/Cargo.toml`:**
- `arrow` - Apache Arrow data format support
- `crossbeam` - Lock-free concurrent primitives
- `re_types` (dev) - For test data generation

---

## Migration Impact

### Breaking Changes
- `EntityDb` no longer derives `Clone` automatically
- Custom clone implementation provided (skips worker)

### API Additions
- New public methods: `submit_arrow_msg()`, `poll_worker_output()`
- New public method: `add_chunk_with_timestamp_metadata()`

### Backward Compatibility
- WASM builds unaffected (compile-time feature gating)
- Existing sync path still works
- Workers are opt-in via `submit_arrow_msg()` usage

---

## Code Quality Improvements

1. **Separation of Concerns**
   - Message transformation logic extracted to `transform_message()`
   - Worker logic isolated in dedicated module
   - Clear boundaries between components

2. **Documentation**
   - Extensive module-level docs in `ingestion_worker.rs`
   - Platform-specific behavior clearly marked
   - API contracts documented

3. **Error Handling**
   - Worker errors logged via `re_log::error_once!()`
   - Graceful degradation on worker failure
   - Results propagated to caller

---

## Related Files Modified

### `.gitignore`
```diff
+# Development notes
+notes
```
Added `notes` to gitignore (developer's working notes).

### `Cargo.lock`
- Updated with new dependencies
- Criterion and mimalloc transitive dependencies added

---

## Future Improvements (Implied)

1. **Batching**
   - Currently processes one message at a time
   - Could batch multiple messages for efficiency

2. **Thread Pool**
   - Shared thread pool instead of per-EntityDb threads
   - Better for many small EntityDbs

3. **Adaptive Concurrency**
   - Adjust worker count based on system load
   - Dynamic channel sizing

4. **Metrics**
   - Expose worker queue depth metrics
   - Track processing latency per EntityDb

---

## Summary

This commit represents a significant architectural evolution in Rerun's data ingestion pipeline. By moving from centralized to distributed (per-EntityDb) workers, it unlocks true parallel processing while maintaining strict ordering guarantees. The addition of comprehensive tests and benchmarks ensures both correctness and measurable performance improvements.

The design is platform-aware (native vs WASM), properly handles backpressure, and maintains backward compatibility. This foundation enables future optimizations while improving the current user experience when loading large datasets.

**Key Takeaway:** Load multiple entity databases faster by processing them in parallel, while keeping single-database loading unchanged and correct.
