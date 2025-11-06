# Message Ordering Analysis

## Current Architecture (Global Worker)

### Message Flow

1. **Messages arrive** via `rx_log` channel in `App::update()`
2. **Processing order**:
   - First: `process_ingestion_worker_output()` polls completed chunks from worker
   - Then: Process new messages from `rx_log`

### Message Types and Processing

#### SetStoreInfo Messages
- Processed **synchronously** in `receive_log_msg()`
- Immediately added to EntityDb
- No worker involvement

#### ArrowMsg Messages (on native)
- Sent **asynchronously** to global `ingestion_worker` queue
- Worker processes in background thread
- Results polled in next frame via `process_ingestion_worker_output()`

### Identified Ordering Problems

#### Problem 1: Cross-Store Interference

**Scenario**: Multiple stores sending interleaved messages

```
Timeline:
Frame N:
  - Receive: SetStoreInfo(Store A)  → processed immediately
  - Receive: ArrowMsg(Store A, msg1) → queued to global worker
  - Receive: SetStoreInfo(Store B)  → processed immediately
  - Receive: ArrowMsg(Store B, msg1) → queued to global worker (position 2)
  - Receive: ArrowMsg(Store A, msg2) → queued to global worker (position 3)

Global Worker Queue: [A-msg1, B-msg1, A-msg2]

Frame N+1:
  - Worker processes A-msg1 (Store A blocked until complete)
  - Then processes B-msg1 (Store B blocked until complete)
  - Then processes A-msg2 (Store A blocked AGAIN)

Result: Store A's msg2 must wait for Store B's msg1 to complete,
        even though they're for different stores and shouldn't interfere.
```

**Impact**:
- Unnecessary head-of-line blocking between unrelated stores
- One slow store blocks all other stores
- Poor multi-store performance

#### Problem 2: SetStoreInfo / ArrowMsg Ordering

**Scenario**: SetStoreInfo and ArrowMsg for same store

```
Timeline:
Frame N:
  - Receive: SetStoreInfo(Store A) → processed immediately, store created
  - Receive: ArrowMsg(Store A) → queued to worker

Frame N+1:
  - process_ingestion_worker_output() → polls worker, processes ArrowMsg

Timing: SetStoreInfo processed in Frame N, ArrowMsg processed in Frame N+1
```

**Current behavior**: Works correctly because:
- SetStoreInfo creates the store immediately
- ArrowMsg is queued with `msg_will_add_new_store=false`
- Worker processes it in next frame, store already exists

**Potential issue**: Code is hard to reason about because:
- Synchronous vs asynchronous processing split by message type
- Ordering depends on frame timing
- Multiple conditional compilation branches (`#[cfg(not(target_arch = "wasm32"))]`)

## Proposed Architecture (Per-EntityDb Workers)

### Key Changes

1. **Worker location**: Each `EntityDb` owns an `Option<IngestionWorker>`
2. **Message routing**: ArrowMsg for Store A goes to Store A's worker
3. **Isolation**: Stores don't interfere with each other

### Benefits

#### Benefit 1: No Cross-Store Interference

```
Timeline:
Frame N:
  - Store A worker queue: [A-msg1, A-msg2]
  - Store B worker queue: [B-msg1]
  - Both workers process independently in parallel

Result: Store A's msg2 doesn't wait for Store B's msg1
```

#### Benefit 2: Cleaner Architecture

- Worker lifecycle tied to EntityDb lifecycle
- No global worker state in App
- WASM conditionals isolated to worker module
- Easier to reason about ownership and ordering

#### Benefit 3: Better Performance for Multi-Store Scenarios

- Multiple stores = multiple CPU cores utilized
- No head-of-line blocking
- Natural parallelism

### Trade-offs

**More threads**: One worker thread per EntityDb instead of one global thread
- **Mitigation**: Most use cases have <10 concurrent recordings
- Modern systems handle hundreds of threads easily
- Threads only created when EntityDb receives Arrow messages

**More memory**: One channel pair per EntityDb
- **Impact**: ~1KB per EntityDb for channels
- Negligible compared to chunk data size

## Test Cases Needed

### Test 1: Per-Store Ordering Preserved
```rust
// Verify messages for Store A arrive in order
Store A: msg1, msg2, msg3
Store B: msg1, msg2
Expected: Store A chunks arrive as 1,2,3; Store B chunks arrive as 1,2
```

### Test 2: No Cross-Store Blocking
```rust
// Verify Store B doesn't block Store A
Store A: fast_msg1, fast_msg2
Store B: slow_msg (10ms processing)
Store A: fast_msg3

Expected: Store A's fast_msg3 completes before Store B's slow_msg
```

### Test 3: WASM Fallback
```rust
// Verify synchronous processing works on WASM
#[cfg(target_arch = "wasm32")]
fn test_wasm_synchronous_processing() {
    // Should process without worker thread
}
```

## Implementation Strategy

### Phase 1: Abstract WASM Conditionals
- Move all `#[cfg(not(target_arch = "wasm32"))]` into `ingestion_worker.rs`
- Create platform-agnostic API
- WASM returns `None` from constructor

### Phase 2: Move Worker to EntityDb
- Add `Option<IngestionWorker>` field to EntityDb
- Lazy initialization on first ArrowMsg
- Update `add()` method to use worker if present

### Phase 3: Update App
- Remove global `ingestion_worker` field
- Update `receive_log_msg` to delegate to EntityDb
- Iterate over all EntityDbs in `process_ingestion_worker_output`

### Phase 4: Add Benchmarks
- Test various queue capacities
- Profile CPU and memory usage
- Validate performance improvements
