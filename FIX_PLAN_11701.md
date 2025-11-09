# Fix Plan for Issue #11701: Webviewer Memory Management Bug

**Issue:** [#11701 - Webviewer does not show large chunks when memory limit is exceeded but blocks memory](https://github.com/rerun-io/rerun/issues/11701)

**Status:** Planning Phase
**Created:** 2025-11-09
**Author:** Joel Reymont

---

## Executive Summary

The webviewer experiences a critical bug where large data batches sent via `send_columns()` cause memory allocation without rendering, accompanied by "Data dropped" messages despite claimed available memory headroom. This creates "zombie" allocations that aren't properly cleared.

**Root Cause:** Race condition and synchronization issues between gRPC server-side memory limits (500MB) and webviewer client-side limits (2.5GB hardcoded), combined with lack of chunk size validation and backpressure mechanisms.

---

## Problem Analysis

### Symptoms
1. Memory allocated (visible in memory panel) but data doesn't render
2. "Data dropped" messages appear despite available memory headroom
3. "Zombie" allocations that aren't cleared when data is rejected
4. Example: ~1.8GB image batch triggers drops with 2.3GB limit claimed available

### Current Architecture

```
┌─────────────────┐
│  Python SDK     │
│  send_columns() │──────┐
└─────────────────┘      │
                         │ Creates large chunks
                         │ (bypasses batching)
                         ▼
                  ┌──────────────────────┐
                  │  bindings.send_      │
                  │  arrow_chunk()       │
                  └──────────────────────┘
                         │
                         ▼
┌──────────────────────────────────────────┐
│         gRPC Server                      │
│  - Memory Limit: Configurable (500MB)    │
│  - MessageBuffer with 3 queues:          │
│    * persistent (never GC'd)             │
│    * static_ (GC'd second)               │
│    * disposable (GC'd first)             │
│  - GC on every message received          │
└──────────────────────────────────────────┘
                         │
                         │ Streams via gRPC
                         ▼
┌──────────────────────────────────────────┐
│         Web Viewer (32-bit WASM)         │
│  - Hard Memory Limit: 2.5GB              │
│  - AccountingAllocator tracks memory     │
│  - No automatic GC                       │
│  - Memory panel shows allocations        │
└──────────────────────────────────────────┘
```

### Key Code Locations

#### gRPC Server Memory Management
- **File:** `/crates/store/re_grpc_server/src/lib.rs`
- **Lines:** 656-704 (GC function)
- **Logic:**
  ```rust
  pub fn gc(&mut self, max_bytes: u64) {
      // Drops disposable messages first
      while self.disposable.pop_front().is_some() {
          if self.size_bytes() < max_bytes { break; }
      }
      // Then drops static messages if still over limit
      while self.static_.pop_front().is_some() {
          if self.size_bytes() < max_bytes { break; }
      }
  }
  ```

#### Web Viewer Memory Limit
- **File:** `/crates/viewer/re_viewer/src/web.rs`
- **Lines:** 744-747
- **Hardcoded limit:**
  ```rust
  memory_limit: re_memory::MemoryLimit {
      // On wasm32 we only have 4GB of memory to play around with.
      max_bytes: Some(2_500_000_000),  // 2.5GB
  },
  ```

#### send_columns Implementation
- **File:** `/rerun_py/rerun_sdk/rerun/_send_columns.py`
- **Line:** 319
- **Bypasses batching:**
  ```python
  bindings.send_arrow_chunk(
      entity_path,
      timelines={t.timeline_name(): t.as_arrow_array() for t in indexes},
      components=columns_args,
      recording=recording.to_native() if recording is not None else None,
  )
  ```

---

## Root Cause Analysis

### Primary Issues

1. **Race Condition Between Server and Client**
   - Server GCs chunks after memory limit exceeded (500MB)
   - But chunks may already be in-flight over gRPC stream
   - Client receives partial/incomplete chunks
   - Client allocates memory for invalid data
   - Result: Memory shows used, but data doesn't render

2. **No Chunk Size Validation**
   - `send_columns()` can create arbitrarily large chunks
   - Single chunk of 1.8GB sent to server with 500MB limit
   - Server immediately GCs it, but transmission may have started
   - No warning to user before data is created

3. **Mismatched Memory Accounting**
   - Server counts: Memory in `MessageBuffer` (buffers)
   - Client counts: Memory via `AccountingAllocator` (actual allocations)
   - In-flight data counted nowhere
   - Dropped data may still allocate on client

4. **No Backpressure Mechanism**
   - `send_columns()` sends immediately, doesn't check server capacity
   - No flow control between SDK and server
   - Server can't signal "slow down" to SDK

5. **Silent Failures**
   - GC logs via `info_once!()` - user may never see
   - No feedback to Python SDK that data was dropped
   - User only notices when data missing from viewer

---

## Proposed Solutions

### Solution 1: Chunk Size Limits (Quick Win)

**Complexity:** Low
**Impact:** Medium
**Timeline:** 1-2 days

#### Changes

1. **Add max chunk size configuration**
   - Default: 100MB per chunk
   - Configurable via environment variable or SDK parameter
   - Document in `send_columns()` API

2. **Validate chunk size before sending**
   ```python
   # In _send_columns.py
   def send_columns(entity_path, times, columns, recording=None):
       # ... existing validation ...

       # Calculate estimated chunk size
       chunk_size = sum(col.nbytes for col in columns_args.values())
       max_chunk_size = recording.max_chunk_size if recording else DEFAULT_MAX_CHUNK_SIZE

       if chunk_size > max_chunk_size:
           raise ValueError(
               f"Chunk size ({format_bytes(chunk_size)}) exceeds maximum "
               f"({format_bytes(max_chunk_size)}). Consider splitting into smaller batches."
           )
   ```

3. **Provide helper for batching**
   ```python
   def send_columns_batched(entity_path, times, columns, batch_size=None, recording=None):
       """Split large datasets into appropriately-sized batches."""
       # Auto-calculate batch_size based on max_chunk_size
       # Yield batches to send_columns()
   ```

**Pros:**
- Simple to implement
- Prevents worst-case scenarios
- Clear error messages to users
- No changes to core architecture

**Cons:**
- Doesn't fix race condition
- Users must manually batch large datasets
- Breaking change if validation is strict

---

### Solution 2: Server-Side Flow Control (Comprehensive)

**Complexity:** Medium
**Impact:** High
**Timeline:** 1-2 weeks

#### Changes

1. **Add bounded channels between SDK and server**
   - Replace unbounded channels with bounded ones
   - Size based on server memory limit
   - Provides natural backpressure

2. **Track in-flight data separately**
   ```rust
   struct MessageBuffer {
       persistent: VecDeque<LogMsgProto>,
       static_: VecDeque<LogMsgProto>,
       disposable: VecDeque<LogMsgProto>,

       // New: Track data being transmitted
       in_flight: HashSet<MessageId>,
       in_flight_bytes: u64,
   }

   impl MessageBuffer {
       fn total_memory(&self) -> u64 {
           self.size_bytes() + self.in_flight_bytes
       }
   }
   ```

3. **GC before transmission, not after**
   - Check memory limit before adding to queue
   - Drop oldest messages to make room
   - Only add to queue if space available
   - Prevents in-flight data from being GC'd

4. **Add feedback mechanism**
   - Server sends "memory pressure" signals to SDK
   - SDK can pause/slow sending
   - User notified via warning

**Implementation Plan:**

```rust
// In re_grpc_server/src/lib.rs

async fn handle_log_message(&mut self, msg: LogMsgProto) -> Result<(), ServerError> {
    // 1. Estimate message size
    let msg_size = msg.encoded_len() as u64;

    // 2. Pre-emptively GC if needed
    let available = self.options.memory_limit.max_bytes
        .saturating_sub(self.messages.total_memory());

    if msg_size > available {
        // Not enough room even after GC
        self.messages.gc(self.options.memory_limit.max_bytes);

        if msg_size > available {
            re_log::warn!(
                "Dropping incoming message ({}) - exceeds available memory ({})",
                re_format::format_bytes(msg_size),
                re_format::format_bytes(available)
            );

            // Send backpressure signal
            self.send_memory_pressure_signal().await?;
            return Err(ServerError::MemoryPressure);
        }
    }

    // 3. Mark as in-flight
    let msg_id = msg.id();
    self.messages.mark_in_flight(msg_id, msg_size);

    // 4. Broadcast to clients
    self.broadcast_log_tx.send(msg.clone())?;

    // 5. Add to buffer
    self.messages.add_message(msg);

    // 6. Remove from in-flight tracking
    self.messages.unmark_in_flight(msg_id);

    Ok(())
}
```

**Pros:**
- Fixes race condition
- Provides proper flow control
- Better memory accounting
- User feedback on memory pressure

**Cons:**
- More complex implementation
- Requires changes across multiple layers
- May impact performance (backpressure)

---

### Solution 3: Client-Side Validation (Defense in Depth)

**Complexity:** Low
**Impact:** Medium
**Timeline:** 2-3 days

#### Changes

1. **Validate chunks before full allocation**
   ```rust
   // In web viewer chunk processing
   fn receive_chunk(&mut self, chunk: ArrowChunk) -> Result<()> {
       // 1. Calculate chunk size
       let chunk_size = estimate_chunk_size(&chunk);

       // 2. Check against available memory
       let current_usage = self.memory_tracker.usage();
       let available = self.memory_limit.max_bytes - current_usage;

       if chunk_size > available {
           re_log::warn!(
               "Rejecting chunk ({}) - would exceed memory limit",
               re_format::format_bytes(chunk_size)
           );

           // Show user-visible error
           self.show_memory_error(chunk_size, available);

           // Request server to re-send later (future enhancement)
           return Err(ChunkError::OutOfMemory);
       }

       // 3. Proceed with allocation
       self.store_chunk(chunk)
   }
   ```

2. **Better memory panel feedback**
   - Show pending/in-flight data
   - Warning when approaching limit
   - Clear indication of dropped data

3. **Graceful degradation**
   - Keep partial data if useful
   - Allow user to request specific time ranges
   - Don't allocate for incomplete chunks

**Pros:**
- Prevents "zombie" allocations
- Better user experience
- Works independently of server fixes

**Cons:**
- Doesn't prevent server-side waste
- Still loses data if server drops it
- Requires viewer changes

---

### Solution 4: Adaptive Batching (Future Enhancement)

**Complexity:** High
**Impact:** High
**Timeline:** 2-3 weeks

#### Concept

Make `send_columns()` automatically split large batches based on:
- Server memory limit (queried via gRPC)
- Network conditions
- Client capabilities

```python
def send_columns_adaptive(entity_path, times, columns, recording=None):
    """
    Automatically batch large datasets for optimal transmission.

    Queries server for memory limit and network conditions,
    then splits data into appropriately-sized chunks.
    """
    # 1. Query server for limits
    server_info = recording.query_server_info()
    max_chunk_size = min(
        server_info.memory_limit * 0.1,  # 10% of server memory
        100 * 1024 * 1024,  # 100MB max
    )

    # 2. Calculate batch size
    total_size = sum(col.nbytes for col in columns)
    num_batches = ceil(total_size / max_chunk_size)

    # 3. Send in batches with progress
    for i, batch in enumerate(split_into_batches(times, columns, num_batches)):
        logger.info(f"Sending batch {i+1}/{num_batches}...")
        send_columns(entity_path, batch.times, batch.columns, recording)

        # Optional: wait for server acknowledgment
        if server_info.supports_flow_control:
            recording.wait_for_ack()
```

**Pros:**
- Best user experience (automatic)
- Optimal performance
- Respects all constraints

**Cons:**
- Complex implementation
- Requires protocol changes
- May add latency

---

## Recommended Implementation Plan

### Phase 1: Quick Wins (Week 1)

**Goal:** Prevent worst-case scenarios and improve error messages

1. **Add chunk size validation to `send_columns()`**
   - Default max: 100MB
   - Raise clear error with suggestion to batch
   - Add to Python SDK

2. **Improve logging**
   - Make "Data dropped" messages more visible
   - Add to viewer UI, not just logs
   - Include suggestions for user

3. **Document batching patterns**
   - Add examples to `send_columns()` docstring
   - Show how to split large datasets
   - Explain memory limits

**Files to modify:**
- `/rerun_py/rerun_sdk/rerun/_send_columns.py`
- `/crates/store/re_grpc_server/src/lib.rs` (logging)
- `/crates/viewer/re_viewer/src/` (UI feedback)

---

### Phase 2: Architectural Improvements (Weeks 2-3)

**Goal:** Fix race conditions and improve memory accounting

1. **Track in-flight data**
   - Add `in_flight` tracking to `MessageBuffer`
   - Update GC logic to consider in-flight data
   - File: `/crates/store/re_grpc_server/src/lib.rs`

2. **Pre-emptive GC**
   - Check memory before queueing messages
   - GC before accepting new data, not after
   - Prevents in-flight waste

3. **Client-side validation**
   - Validate chunks before allocation in web viewer
   - File: `/crates/viewer/re_viewer/src/` (chunk handling)

**Files to modify:**
- `/crates/store/re_grpc_server/src/lib.rs`
- `/crates/viewer/re_viewer/src/` (multiple files)
- `/crates/store/re_chunk/src/` (chunk utilities)

---

### Phase 3: Flow Control (Weeks 4-5)

**Goal:** Implement proper backpressure mechanism

1. **Add bounded channels**
   - Replace unbounded channels in gRPC server
   - Size based on memory limit

2. **Backpressure signals**
   - Server sends "slow down" to SDK
   - SDK pauses/batches when signaled

3. **API for adaptive batching**
   - Helper function in Python SDK
   - Automatic splitting of large batches

**Files to modify:**
- `/rerun_py/rerun_sdk/rerun/_send_columns.py`
- `/crates/store/re_grpc_server/src/lib.rs`
- gRPC protocol definitions

---

## Testing Strategy

### Unit Tests

1. **Chunk size validation**
   ```python
   def test_send_columns_chunk_size_limit():
       # Create data exceeding limit
       large_images = np.random.randint(0, 255, (5000, 480, 640, 3), dtype=np.uint8)

       # Should raise ValueError
       with pytest.raises(ValueError, match="exceeds maximum"):
           rr.send_columns("test/images",
                          times=[rr.TimeSequenceColumn("frame", range(5000))],
                          components=[rr.components.ImageBufferBatch(large_images)])
   ```

2. **MessageBuffer GC logic**
   ```rust
   #[test]
   fn test_gc_prevents_in_flight_drop() {
       let mut buffer = MessageBuffer::default();

       // Add messages
       buffer.add_disposable(msg1);
       buffer.mark_in_flight(msg1.id(), 1000);

       // GC should not drop in-flight messages
       buffer.gc(500);

       assert!(buffer.contains(msg1.id()));
   }
   ```

### Integration Tests

1. **Memory limit exceeded scenario**
   - Start server with 100MB limit
   - Send 200MB of data via `send_columns()`
   - Verify:
     - Server doesn't crash
     - Client shows error message
     - Memory stays under limit
     - No "zombie" allocations

2. **Large batch batching**
   - Use reproduction script with adaptive batching
   - Verify all data arrives
   - Check memory usage stays reasonable

### End-to-End Tests

1. **Reproduction case**
   - Run `webviewer_memory_repro_11701.py`
   - With fixes, should either:
     - Reject upfront with clear error, OR
     - Successfully batch and display all data

2. **Stress test**
   - Send continuous stream of data
   - Approaching but not exceeding limits
   - Verify stable operation over time

---

## Metrics for Success

1. **No "zombie" allocations**
   - Memory panel matches actual data displayed
   - Dropped data doesn't allocate memory

2. **Clear error messages**
   - Users know immediately if data is too large
   - Suggestions on how to fix

3. **No silent data loss**
   - If data is dropped, user is notified clearly
   - Viewer UI shows warning

4. **Memory accounting accuracy**
   - Server and client memory tracking aligned
   - In-flight data accounted for

---

## Risks and Considerations

### Breaking Changes

- Adding chunk size validation may break existing code that sends large batches
- **Mitigation:** Make configurable, add grace period with warnings

### Performance Impact

- Bounded channels add backpressure, may slow sending
- **Mitigation:** Tune buffer sizes, make configurable

### Backward Compatibility

- Older clients won't support new flow control
- **Mitigation:** Feature detection, graceful fallback

### Testing Coverage

- Need to test with various memory limits and data sizes
- **Mitigation:** Parameterized tests, stress tests

---

## Future Enhancements

1. **Query API for server capabilities**
   - SDK can ask server about memory limits
   - Adaptive behavior based on server config

2. **Resumable transfers**
   - If data is dropped, allow retry
   - Checkpoint mechanism for large batches

3. **Compression**
   - Compress chunks before sending
   - Reduces memory usage and network traffic

4. **Streaming chunks**
   - Don't materialize entire chunk in memory
   - Stream directly from source to network

5. **Smart GC policies**
   - LRU (least recently used) instead of FIFO
   - Prioritize visible data over off-screen

---

## References

### Key Files

- `/crates/store/re_grpc_server/src/lib.rs` - Server memory management
- `/crates/viewer/re_viewer/src/web.rs` - Web viewer limits
- `/rerun_py/rerun_sdk/rerun/_send_columns.py` - Python API
- `/crates/store/re_chunk/src/batcher.rs` - Chunk batching logic
- `/crates/utils/re_memory/src/memory_limit.rs` - Memory limit types

### Related Issues

- Issue #11701 - Main bug report
- Discord thread on WebAssembly memory limits

### Documentation

- `send_columns()` API documentation
- Memory management architecture (to be created)
- Best practices for large datasets (to be created)

---

## Appendix: Code Examples

### Example 1: Manual Batching (Current Workaround)

```python
import numpy as np
import rerun as rr

def send_large_dataset_batched(entity_path, images, batch_size=100):
    """
    Send large image dataset in batches to avoid memory issues.

    Args:
        entity_path: Where to log the images
        images: ndarray of shape (N, H, W, C)
        batch_size: Number of images per batch
    """
    num_images = len(images)
    num_batches = (num_images + batch_size - 1) // batch_size

    for i in range(num_batches):
        start = i * batch_size
        end = min(start + batch_size, num_images)
        batch = images[start:end]
        times = np.arange(start, end)

        print(f"Sending batch {i+1}/{num_batches} ({len(batch)} images)...")

        rr.send_columns(
            entity_path,
            times=[rr.TimeSequenceColumn("frame", times)],
            components=[rr.components.ImageBufferBatch(batch)],
        )

# Usage
rr.init("app", spawn=True)
large_images = np.random.randint(0, 255, (2000, 480, 640, 3), dtype=np.uint8)
send_large_dataset_batched("camera/images", large_images, batch_size=100)
```

### Example 2: Estimated Size Calculation

```python
def estimate_chunk_size(components):
    """Estimate the size of a chunk in bytes."""
    total = 0
    for component in components:
        if hasattr(component, 'nbytes'):
            total += component.nbytes
        elif isinstance(component, pa.Array):
            # PyArrow arrays
            for buffer in component.buffers():
                if buffer:
                    total += buffer.size
    return total
```

### Example 3: Server-Side Memory Pressure Check

```rust
fn should_accept_message(&self, msg_size: u64) -> bool {
    let current = self.messages.total_memory();
    let limit = self.options.memory_limit.max_bytes;

    // Accept if we have at least 20% headroom after adding message
    let after_adding = current + msg_size;
    let headroom_ratio = (limit - after_adding) as f64 / limit as f64;

    headroom_ratio > 0.20
}
```

---

**End of Fix Plan**
