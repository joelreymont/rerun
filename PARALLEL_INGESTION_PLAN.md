# Plan to Address PR Review #3427288069

## Problem Summary

The reviewer identified 5 key issues with the current parallel ingestion implementation:

1. **Message Ordering Concerns**: Global worker struggles with multiple SetStoreInfo and interleaved ArrowMsg sequences
2. **Architecture Issue**: Global worker design makes ordering hard to reason about
3. **Code Quality**: Spurious diffs in entity_db.rs (import formatting, extra blank line)
4. **Profiling Gaps**: Need comprehensive profiling data to justify queue capacity of 2000
5. **WASM Conditionals**: Platform-specific code scattered throughout instead of abstracted

## Solution Architecture

### Core Change: Per-EntityDb Workers Instead of Global Worker

**Current Design:**
- Single global IngestionWorker in App struct (app.rs:105)
- All Arrow messages routed to same worker regardless of store
- Ordering issues when multiple stores send interleaved messages

**Proposed Design:**
- Each EntityDb owns its own IngestionWorker
- Messages for a specific store are guaranteed to be processed in order
- Natural isolation between different stores/recordings
- Worker lifecycle tied to EntityDb lifecycle

## Implementation Steps

### 1. Research Message Ordering Issue
- Document current message flow: SetStoreInfo → ArrowMsg sequences
- Identify scenarios where interleaving causes problems
- Create test cases demonstrating ordering guarantees needed

### 2. Design Per-EntityDb Architecture

Key decisions:
- **Location**: Add `Option<IngestionWorker>` field to EntityDb struct
- **Lifecycle**: Create worker lazily on first Arrow message
- **Platform**: Use `Option<IngestionWorker>` (Some on native, None on WASM)
- **Cleanup**: Worker drops automatically when EntityDb is dropped

### 3. Abstract WASM Conditionals

Create platform-agnostic wrapper in ingestion_worker.rs:

```rust
// On WASM: no-op synchronous implementation
// On native: background thread with channels
pub struct IngestionWorker { /* platform-specific internals */ }

impl IngestionWorker {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Option<Self> { Some(Self { /* thread-based */ }) }

    #[cfg(target_arch = "wasm32")]
    pub fn new() -> Option<Self> { None }

    pub fn submit_arrow_msg(/* ... */) { /* platform-specific */ }
    pub fn poll_processed_chunks(/* ... */) { /* platform-specific */ }
}
```

This moves ALL `#[cfg(not(target_arch = "wasm32"))]` directives into the worker module.

### 4. Implementation Changes

#### File: `crates/viewer/re_viewer/src/ingestion_worker.rs`
- Make WASM-compatible with conditional compilation internals only
- Add `Option<Self>` return type for constructor
- Synchronous fallback for WASM

#### File: `crates/store/re_entity_db/src/entity_db.rs`
- Add `ingestion_worker: Option<IngestionWorker>` field
- Remove spurious diffs:
  - Fix import formatting (line 14-16)
  - Remove extra blank line (line 884)
- Keep `add_chunk_with_timestamp_metadata` public (needed by worker)
- Keep `last_modified_at` move (correct location)

#### File: `crates/viewer/re_viewer/src/app.rs`
- Remove global `ingestion_worker` field (line 105)
- Remove `#[cfg(not(target_arch = "wasm32"))]` directives (lines 49, 104, 126, 201, 330, 368)
- Update `handle_log_msg` to delegate to EntityDb's worker
- Simplify `process_ingestion_worker_output` to iterate over all EntityDbs

### 5. Add Comprehensive Profiling

Create benchmark variations testing:
- **Queue capacities**: 100, 500, 1000, 2000, 5000, 10000
- **Message sizes**: small (100 rows), medium (1000 rows), large (10000 rows)
- **Concurrent stores**: 1, 2, 5, 10

Puffin profiles showing:
- UI thread blocking time
- Worker thread utilization
- Memory pressure from queue depth

### 6. Update Tests
- Modify `test_message_order_preserved` to verify per-store ordering
- Add test for multiple concurrent stores with interleaved messages
- Verify WASM compatibility (synchronous path)

### 7. Documentation
- Update module docs in ingestion_worker.rs explaining per-EntityDb design
- Document ordering guarantees: within-store ordered, cross-store unordered
- Add architecture diagram showing message flow

## Expected Benefits

- **Message Ordering**: Within-store ordering naturally preserved by per-EntityDb workers
- **Cleaner Code**: No scattered WASM conditionals in app.rs
- **Better Isolation**: Different recordings can't interfere with each other
- **Simpler Reasoning**: Worker lifecycle tied to EntityDb makes ownership clear
- **Performance**: Same or better (each EntityDb gets dedicated thread on native)

## Trade-offs

**Pros:**
- Natural ordering guarantees
- Cleaner architecture
- Better thread utilization for multi-store scenarios

**Cons:**
- More threads (one per active EntityDb instead of one global)
- Slightly more memory (one channel pair per EntityDb)

**Mitigation**: Threads are cheap on modern systems, and most use cases have <10 concurrent recordings
