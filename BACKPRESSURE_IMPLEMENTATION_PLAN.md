# Backpressure Implementation Plan

**Issue**: [#11024](https://github.com/rerun-io/rerun/issues/11024) - Apply backpressure when viewer cannot ingest data fast enough

**Author**: Joel Reymont
**Date**: 2025-11-09

---

## Executive Summary

The Rerun viewer currently experiences memory bloat when unable to process incoming data at sufficient speed, particularly in web-based environments with constrained memory resources. This document outlines findings from codebase analysis and provides a detailed implementation plan for a backpressure mechanism.

**Key Problem**: Unbounded channels with no flow control allow fast data producers to overwhelm slow consumers, leading to unbounded memory growth and eventual message dropping via broadcast channel lagging.

**Proposed Solution**: Implement byte-size tracking in `re_smart_channel` and add gRPC-level backpressure to throttle producers when queues become too heavy.

---

## Current Architecture Analysis

### 1. Smart Channel Implementation

**Location**: `crates/utils/re_smart_channel/`

**Current Capabilities**:
- ✅ Latency tracking (time from send to receive)
- ✅ Message count tracking (`len()` method)
- ✅ Source identification (channel and message level)
- ✅ Connection status monitoring
- ✅ Flush and graceful shutdown support

**Critical Limitations**:
- ❌ **No byte-size tracking** - only message counts available
- ❌ **No capacity limits** - uses `crossbeam::channel::unbounded()` (lib.rs:294)
- ❌ **No backpressure mechanism** - senders can overwhelm receivers
- ❌ **No historical metrics** - only latest latency value stored
- ❌ **No memory usage visibility** - can't determine queue memory footprint

**Key Files**:
- `src/lib.rs` - Core types, channel creation, source enums
- `src/sender.rs` - Sender with monitoring methods
- `src/receiver.rs` - Receiver with latency calculation
- `src/receive_set.rs` - Multi-source multiplexing

### 2. gRPC Server Implementation

**Location**: `crates/store/re_grpc_server/`

**Data Flow**:
```
Client gRPC Stream
  → write_messages() RPC
  → mpsc::channel (MESSAGE_QUEUE_CAPACITY)
  → EventLoop processing
  → broadcast::channel (to all clients)
  → SmartChannel output
  → Viewer ReceiveSet
```

**Current Behavior**:
- **Input**: Accepts unbounded streaming from `write_messages()` RPC (lib.rs:954-984)
- **Buffering**: Uses `mpsc::channel` with ~16 MiB capacity (lib.rs:464-479)
- **Distribution**: Broadcasts to all clients via `broadcast::channel`
- **Overflow Handling**: **Silently drops messages** when receivers lag (lib.rs:397-402)

**Critical Issues**:
```rust
// lib.rs:397-402
Err(broadcast::error::RecvError::Lagged(n)) => {
    re_log::warn!(
        "message proxy receiver dropped {n} messages due to backpressure"
    );
    continue; // DATA LOSS!
}
```

**No Flow Control**:
- ❌ No backpressure to gRPC clients when queues are full
- ❌ No rate limiting or throttling
- ❌ No notification to sender when messages are dropped

### 3. Viewer Data Ingestion

**Location**: `crates/viewer/re_viewer/src/app.rs`

**Processing Loop** (app.rs:2082-2156):
```rust
// Time-boxed message processing
let start = web_time::Instant::now();
while let Some((channel_source, msg)) = self.rx_log.try_recv() {
    // Process message...

    if start.elapsed() > web_time::Duration::from_millis(10) {
        egui_ctx.request_repaint(); // Continue next frame
        break; // Prevent UI blocking
    }
}
```

**Key Characteristics**:
- **10ms time budget** per frame for message processing
- **Fair scheduling** via `crossbeam::Select` across sources
- **Memory-based GC** when 75% RAM threshold exceeded
- **Latency monitoring** with automatic warnings

**Consequence**: Fast producers can accumulate unbounded backlogs during the periods between 10ms processing windows.

---

## Problem Statement

### Root Cause

The viewer's data pipeline uses **unbounded channels** at multiple levels:
1. Smart channels use `crossbeam::channel::unbounded()`
2. No flow control in gRPC streaming RPCs
3. Time-boxed processing (10ms/frame) limits consumption rate

This creates a **producer-consumer imbalance** where fast producers can overwhelm slow consumers.

### Failure Modes

1. **Memory Bloat**:
   - Unbounded queues grow indefinitely
   - Particularly severe on web (limited memory)
   - Triggers aggressive GC or OOM

2. **Message Loss**:
   - Broadcast channels drop messages when receivers lag
   - No notification to sender
   - Silent data corruption

3. **Latency Explosion**:
   - Queue depth → unbounded latency
   - User sees stale visualizations
   - E2E latency warnings appear

4. **UI Degradation**:
   - GC pauses affect frame rates
   - Memory pressure impacts performance
   - Poor user experience

### Previous Mitigation Attempt

The `--drop-at-latency` flag was removed (PR #11025, see CHANGELOG.md:309) as it was:
- Non-functional
- Wrong approach (reactive dropping vs. proactive throttling)
- Lacked proper backpressure semantics

---

## Findings

### Finding 1: No Byte-Size Visibility

**Impact**: HIGH

`SmartChannel` tracks message count but not byte size:
```rust
// Can query:
sender.len()              // Message count ✅
sender.latency_nanos()    // Latest latency ✅

// Cannot query:
sender.queue_bytes()      // Total queued bytes ❌
sender.message_size()     // Size of messages ❌
```

**Consequences**:
- Can't set memory-based backpressure thresholds
- Can't correlate queue depth with memory usage
- Can't predict when OOM will occur

**Location**: `crates/utils/re_smart_channel/src/lib.rs`

### Finding 2: Unbounded Allocation

**Impact**: HIGH

All channels are unbounded:
```rust
// lib.rs:294
let (tx, rx) = crossbeam::channel::unbounded();
```

**Consequences**:
- No built-in backpressure
- Memory grows until system exhaustion
- GC is reactive, not proactive

**Location**: `crates/utils/re_smart_channel/src/lib.rs:294`

### Finding 3: gRPC Has No Flow Control

**Impact**: HIGH

The `write_messages` RPC accepts unbounded streaming:
```rust
// lib.rs:954-984
async fn write_messages(
    &self,
    request: tonic::Request<tonic::Streaming<WriteMessagesRequest>>,
) -> Result<tonic::Response<WriteMessagesResponse>, tonic::Status> {
    let mut stream = request.into_inner();

    while let Some(msg) = stream.message().await? {
        // No flow control - just keep accepting!
        self.push_msg(log_msg).await;
    }
}
```

**Consequences**:
- Fast clients can overwhelm server
- No signaling when server is overloaded
- Can't implement proper backpressure

**Location**: `crates/store/re_grpc_server/src/lib.rs:954-984`

### Finding 4: Broadcast Channel Message Loss

**Impact**: MEDIUM

When receivers lag, messages are silently dropped:
```rust
// lib.rs:397-402
Err(broadcast::error::RecvError::Lagged(n)) => {
    re_log::warn!("...dropped {n} messages due to backpressure");
    continue; // Skip dropped messages - DATA LOSS
}
```

**Consequences**:
- Silent data corruption
- No recovery mechanism
- Poor observability (just a log warning)

**Location**: `crates/store/re_grpc_server/src/lib.rs:397-402`

### Finding 5: Time-Boxed Processing Bottleneck

**Impact**: MEDIUM

Viewer processes messages for only 10ms per frame:
```rust
// app.rs:2153-2156
if start.elapsed() > web_time::Duration::from_millis(10) {
    egui_ctx.request_repaint();
    break; // Continue in next frame
}
```

At 60 FPS, this means:
- **10ms processing** per 16.67ms frame
- **Max throughput**: Limited by what can be processed in 10ms
- **Backlog accumulation**: Inevitable if producer rate > consumption rate

**Consequences**:
- Hard upper bound on ingestion rate
- Any producer exceeding this rate will cause queue growth
- Backpressure is essential, not optional

**Location**: `crates/viewer/re_viewer/src/app.rs:2153-2156`

### Finding 6: Good Latency Tracking Foundation

**Impact**: POSITIVE

Existing latency infrastructure is solid:
```rust
// entity_db/src/ingestion_statistics.rs
pub struct LatencySnapshot {
    pub e2e: Option<f32>,           // End-to-end
    pub log2chunk: Option<f32>,     // Batching
    pub chunk2encode: Option<f32>,  // Encoding
    pub transmission: Option<f32>,  // Network
    pub decode2ingest: Option<f32>, // Decoding
}
```

**Benefits**:
- Comprehensive pipeline visibility
- Rolling averages (1s window)
- Automatic UI warnings
- Good foundation for backpressure decisions

**Location**: `crates/store/re_entity_db/src/ingestion_statistics.rs`

---

## Implementation Plan

### Phase 1: Smart Channel Byte Tracking

**Goal**: Add byte-size tracking to `re_smart_channel` to monitor queue memory usage.

#### 1.1 Add Shared Byte Counter

**File**: `crates/utils/re_smart_channel/src/lib.rs`

**Changes**:
```rust
pub(crate) struct SharedStats {
    latency_nanos: AtomicU64,
    queue_bytes: AtomicU64,  // NEW: Total bytes in queue
}
```

**Rationale**:
- `AtomicU64` for thread-safe updates from sender/receiver
- Tracks cumulative size of all queued messages
- Incremented on send, decremented on receive

#### 1.2 Add Size Estimation Trait

**File**: `crates/utils/re_smart_channel/src/lib.rs`

**Changes**:
```rust
/// Trait for types that can estimate their in-memory size
pub trait SizeBytes {
    /// Estimate the heap size in bytes of this value
    fn size_bytes(&self) -> u64;
}

impl<T: SizeBytes + Send> SmartMessage<T> {
    pub fn size_bytes(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
            + self.payload.size_bytes()
    }
}
```

**Rationale**:
- Trait-based approach allows custom sizing logic
- Important for complex types (e.g., `LogMsg` with variable-size chunks)
- Falls back to `std::mem::size_of` for simple types

#### 1.3 Update Sender to Track Bytes

**File**: `crates/utils/re_smart_channel/src/sender.rs`

**Changes**:
```rust
impl<T: SizeBytes + Send> Sender<T> {
    pub fn send(&self, msg: T) -> Result<(), SendError<T>> {
        let smart_msg = SmartMessage {
            time: Instant::now(),
            source: self.source.clone(),
            payload: SmartMessagePayload::Msg(msg),
        };

        let size = smart_msg.size_bytes();
        self.stats.queue_bytes.fetch_add(size, Ordering::Relaxed);

        self.tx.send(smart_msg)?;
        Ok(())
    }

    pub fn queue_bytes(&self) -> u64 {
        self.stats.queue_bytes.load(Ordering::Relaxed)
    }
}
```

**Rationale**:
- Atomic increment on send
- Exposes `queue_bytes()` API for monitoring
- Minimal performance impact (relaxed ordering sufficient)

#### 1.4 Update Receiver to Decrement Bytes

**File**: `crates/utils/re_smart_channel/src/receiver.rs`

**Changes**:
```rust
impl<T: SizeBytes + Send> Receiver<T> {
    pub fn recv(&self) -> Result<SmartMessage<T>, RecvError> {
        let msg = self.rx.recv()?;

        let size = msg.size_bytes();
        self.stats.queue_bytes.fetch_sub(size, Ordering::Relaxed);

        self.update_latency(&msg);
        Ok(msg)
    }

    pub fn queue_bytes(&self) -> u64 {
        self.stats.queue_bytes.load(Ordering::Relaxed)
    }
}
```

**Rationale**:
- Atomic decrement on receive
- Maintains accurate queue size
- Available on both sender and receiver

#### 1.5 Add ReceiveSet Aggregation

**File**: `crates/utils/re_smart_channel/src/receive_set.rs`

**Changes**:
```rust
impl<T: SizeBytes + Send> ReceiveSet<T> {
    pub fn queue_bytes(&self) -> u64 {
        let receivers = self.receivers.lock();
        receivers.iter()
            .map(|r| r.queue_bytes())
            .sum()
    }
}
```

**Rationale**:
- Aggregate byte count across all sources
- Provides total memory footprint
- Used for backpressure decisions

#### 1.6 Implement SizeBytes for Key Types

**File**: `crates/store/re_log_types/src/lib.rs` (or relevant type files)

**Changes**:
```rust
impl SizeBytes for LogMsg {
    fn size_bytes(&self) -> u64 {
        match self {
            LogMsg::SetStoreInfo(msg) => msg.size_bytes(),
            LogMsg::ArrowMsg(_, arrow_msg) => arrow_msg.size_bytes(),
            // ... other variants
        }
    }
}

impl SizeBytes for ArrowMsg {
    fn size_bytes(&self) -> u64 {
        std::mem::size_of::<Self>() as u64
            + self.chunk.size_bytes()  // Chunk has byte size tracking
    }
}
```

**Rationale**:
- Accurate sizing for variable-length data
- Leverage existing `Chunk::heap_size_bytes()` method
- Critical for meaningful backpressure thresholds

**Testing**:
- Unit tests for byte tracking accuracy
- Verify sender.queue_bytes() == sum(message sizes)
- Test with various message types and sizes
- Benchmark performance impact (should be negligible)

---

### Phase 2: gRPC Backpressure Mechanism

**Goal**: Implement flow control in gRPC server to throttle clients when queues are too heavy.

#### 2.1 Define Backpressure Policy

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
#[derive(Clone, Debug)]
pub struct BackpressurePolicy {
    /// Max bytes allowed in smart channel queue before applying backpressure
    pub max_queue_bytes: u64,

    /// How long to sleep when backpressure is active
    pub backpressure_delay: Duration,

    /// Max time to wait for queue to drain (prevents deadlock)
    pub max_backpressure_wait: Duration,
}

impl Default for BackpressurePolicy {
    fn default() -> Self {
        Self {
            max_queue_bytes: 100 * 1024 * 1024,  // 100 MiB
            backpressure_delay: Duration::from_millis(10),
            max_backpressure_wait: Duration::from_secs(5),
        }
    }
}
```

**Rationale**:
- Configurable thresholds for different environments
- Web: Lower limits (e.g., 50 MiB)
- Desktop: Higher limits (e.g., 200 MiB)
- Prevents unbounded waiting via max timeout

#### 2.2 Add Backpressure to write_messages RPC

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
async fn write_messages(
    &self,
    request: tonic::Request<tonic::Streaming<WriteMessagesRequest>>,
) -> Result<tonic::Response<WriteMessagesResponse>, tonic::Status> {
    let mut stream = request.into_inner();

    while let Some(request) = stream.message().await? {
        let Some(log_msg) = request.log_msg else {
            re_log::warn!("missing log_msg in WriteMessagesRequest");
            continue;
        };

        // NEW: Apply backpressure before processing
        self.wait_for_queue_capacity().await?;

        self.push_msg(log_msg).await;
    }

    Ok(tonic::Response::new(WriteMessagesResponse {}))
}
```

**Rationale**:
- Block gRPC stream reading when queue is full
- Prevents unbounded buffering
- TCP backpressure propagates to client

#### 2.3 Implement Queue Monitoring

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
impl EventLoop {
    async fn wait_for_queue_capacity(&self) -> Result<(), tonic::Status> {
        let policy = &self.backpressure_policy;
        let start = Instant::now();

        loop {
            // Check smart channel queue size
            let queue_bytes = self.get_output_queue_bytes();

            if queue_bytes < policy.max_queue_bytes {
                return Ok(());  // Capacity available
            }

            // Check timeout
            if start.elapsed() > policy.max_backpressure_wait {
                re_log::error!(
                    "Backpressure timeout after {:?} with queue at {} MiB",
                    policy.max_backpressure_wait,
                    queue_bytes / 1024 / 1024
                );
                return Err(tonic::Status::resource_exhausted(
                    "Viewer cannot keep up with data rate"
                ));
            }

            // Log warning (rate-limited)
            static LAST_WARN: AtomicU64 = AtomicU64::new(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let last = LAST_WARN.load(Ordering::Relaxed);
            if now - last > 5 {  // Warn every 5 seconds max
                re_log::warn!(
                    "Applying backpressure: queue at {} MiB (limit {} MiB)",
                    queue_bytes / 1024 / 1024,
                    policy.max_queue_bytes / 1024 / 1024
                );
                LAST_WARN.store(now, Ordering::Relaxed);
            }

            // Wait before checking again
            tokio::time::sleep(policy.backpressure_delay).await;
        }
    }

    fn get_output_queue_bytes(&self) -> u64 {
        // Query the smart channel's queue_bytes()
        // This requires access to the output channel sender/receiver
        // May need to add this to EventLoop struct
        self.output_channel_sender.queue_bytes()
    }
}
```

**Rationale**:
- Async waiting doesn't block other tasks
- Rate-limited logging prevents log spam
- Timeout prevents deadlock
- Returns gRPC error if timeout exceeded

#### 2.4 Add Backpressure Metrics

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
#[derive(Default)]
pub struct BackpressureStats {
    pub total_waits: AtomicU64,
    pub total_wait_time_nanos: AtomicU64,
    pub timeouts: AtomicU64,
}

impl EventLoop {
    pub fn backpressure_stats(&self) -> BackpressureStatsSnapshot {
        BackpressureStatsSnapshot {
            total_waits: self.stats.total_waits.load(Ordering::Relaxed),
            total_wait_time_nanos: self.stats.total_wait_time_nanos.load(Ordering::Relaxed),
            timeouts: self.stats.timeouts.load(Ordering::Relaxed),
        }
    }
}
```

**Rationale**:
- Observability into backpressure behavior
- Can be exposed in UI or logs
- Helps tuning thresholds

#### 2.5 Wire Up Output Channel Reference

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
struct EventLoop {
    // ... existing fields ...

    /// Reference to output channel sender for queue monitoring
    output_sender: Option<Sender<DataSourceMessage>>,

    backpressure_policy: BackpressurePolicy,
    backpressure_stats: BackpressureStats,
}
```

**Rationale**:
- EventLoop needs access to smart channel to query queue_bytes()
- May need to thread this through from `spawn_with_recv` or `serve_from_channel`
- Alternative: Use a separate monitoring channel

**Testing**:
- Integration test: Fast producer, slow consumer
- Verify backpressure is applied
- Verify queue stays under limit
- Verify timeout behavior
- Test gRPC error propagation

---

### Phase 3: Configuration and Tuning

**Goal**: Make backpressure configurable and tune defaults for different platforms.

#### 3.1 Add CLI Options

**File**: `crates/viewer/re_viewer/src/lib.rs` (or CLI parsing file)

**Changes**:
```rust
pub struct ViewerOptions {
    // ... existing fields ...

    /// Maximum queue size in bytes before applying backpressure (default: 100 MiB)
    pub max_queue_bytes: Option<u64>,

    /// Disable backpressure (dangerous, for testing only)
    pub disable_backpressure: bool,
}
```

**Rationale**:
- Power users can tune for their environment
- Testing and benchmarking flexibility
- Override defaults on resource-constrained systems

#### 3.2 Platform-Specific Defaults

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
impl BackpressurePolicy {
    pub fn platform_default() -> Self {
        #[cfg(target_arch = "wasm32")]
        let max_queue_bytes = 50 * 1024 * 1024;  // 50 MiB for web

        #[cfg(not(target_arch = "wasm32"))]
        let max_queue_bytes = 200 * 1024 * 1024;  // 200 MiB for desktop

        Self {
            max_queue_bytes,
            ..Default::default()
        }
    }
}
```

**Rationale**:
- Web has much more constrained memory
- Desktop can afford larger buffers
- Reduces memory pressure on web

#### 3.3 Environment Variable Overrides

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
impl BackpressurePolicy {
    pub fn from_env() -> Self {
        let mut policy = Self::platform_default();

        if let Ok(val) = std::env::var("RERUN_MAX_QUEUE_BYTES") {
            if let Ok(bytes) = val.parse::<u64>() {
                policy.max_queue_bytes = bytes;
            }
        }

        if std::env::var("RERUN_DISABLE_BACKPRESSURE").is_ok() {
            policy.max_queue_bytes = u64::MAX;  // Effectively disabled
        }

        policy
    }
}
```

**Rationale**:
- Easy runtime configuration
- Debugging and profiling
- CI/testing environments

#### 3.4 UI Exposure

**File**: `crates/viewer/re_viewer/src/ui/top_panel.rs`

**Changes**:
```rust
// Add to performance panel
if self.show_metrics {
    ui.label(format!(
        "Queue: {:.1} MiB / {:.1} MiB",
        rx_log.queue_bytes() as f64 / 1024.0 / 1024.0,
        max_queue_bytes as f64 / 1024.0 / 1024.0
    ));

    // Optional: Show backpressure indicator
    if rx_log.queue_bytes() > max_queue_bytes * 8 / 10 {
        ui.colored_label(
            egui::Color32::YELLOW,
            "⚠ Backpressure active"
        );
    }
}
```

**Rationale**:
- User visibility into backpressure state
- Helps diagnose performance issues
- Feedback during development

**Testing**:
- Test CLI option parsing
- Verify environment variable overrides
- Test platform-specific defaults
- UI integration testing

---

### Phase 4: Monitoring and Observability

**Goal**: Provide comprehensive visibility into backpressure behavior for debugging and tuning.

#### 4.1 Add Queue Metrics to IngestionStatistics

**File**: `crates/store/re_entity_db/src/ingestion_statistics.rs`

**Changes**:
```rust
pub struct IngestionStatistics {
    // ... existing fields ...

    pub queue_bytes: AtomicU64,
    pub max_queue_bytes_seen: AtomicU64,
    pub backpressure_events: AtomicU64,
}

impl IngestionStatistics {
    pub fn queue_snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            current_bytes: self.queue_bytes.load(Ordering::Relaxed),
            max_bytes_seen: self.max_queue_bytes_seen.load(Ordering::Relaxed),
            backpressure_events: self.backpressure_events.load(Ordering::Relaxed),
        }
    }
}
```

**Rationale**:
- Centralized metrics collection
- Historical max tracking
- Integration with existing statistics

#### 4.2 Enhanced Logging

**File**: `crates/store/re_grpc_server/src/lib.rs`

**Changes**:
```rust
// On backpressure activation
re_log::info!(
    "Backpressure applied: queue={:.1} MiB, threshold={:.1} MiB, wait_count={}",
    queue_bytes as f64 / 1024.0 / 1024.0,
    max_queue_bytes as f64 / 1024.0 / 1024.0,
    wait_count
);

// On backpressure release
re_log::info!(
    "Backpressure released: queue={:.1} MiB, waited for {:.2}s",
    queue_bytes as f64 / 1024.0 / 1024.0,
    wait_duration.as_secs_f64()
);

// On timeout
re_log::error!(
    "Backpressure timeout: viewer cannot keep up (queue={:.1} MiB, waited {:.2}s)",
    queue_bytes as f64 / 1024.0 / 1024.0,
    wait_duration.as_secs_f64()
);
```

**Rationale**:
- Clear diagnostic messages
- Rate-limited to avoid spam
- Structured logging for analysis

#### 4.3 Metrics Export (Optional)

**File**: `crates/store/re_grpc_server/src/metrics.rs` (new file)

**Changes**:
```rust
// Optional: Prometheus/OpenTelemetry integration
pub struct BackpressureMetrics {
    queue_bytes: Gauge,
    backpressure_waits: Counter,
    backpressure_duration: Histogram,
}

impl BackpressureMetrics {
    pub fn record_wait(&self, duration: Duration, queue_bytes: u64) {
        self.backpressure_waits.increment(1);
        self.backpressure_duration.record(duration);
        self.queue_bytes.set(queue_bytes as f64);
    }
}
```

**Rationale**:
- Production monitoring
- Performance analysis
- Alerting on backpressure issues

**Testing**:
- Verify metrics are collected correctly
- Test log output
- Integration with UI metrics panel

---

## Technical Design Details

### Memory Safety

**Overflow Protection**:
```rust
// Use saturating arithmetic to prevent overflow
self.stats.queue_bytes.fetch_add(size, Ordering::Relaxed)
    .saturating_add(size);

// Or use checked arithmetic with error handling
let current = self.stats.queue_bytes.load(Ordering::Relaxed);
let new = current.checked_add(size)
    .ok_or_else(|| Error::QueueOverflow)?;
self.stats.queue_bytes.store(new, Ordering::Relaxed);
```

**Underflow Protection**:
```rust
// Ensure we don't decrement below zero
self.stats.queue_bytes.fetch_sub(size, Ordering::Relaxed)
    .saturating_sub(size);
```

### Atomicity Considerations

**Ordering Guarantees**:
- Use `Ordering::Relaxed` for queue_bytes (performance)
- Accuracy not critical (approximate is fine)
- Eventual consistency acceptable

**Race Conditions**:
- queue_bytes may be slightly inaccurate during concurrent access
- Acceptable: backpressure thresholds have margin
- Alternative: Use mutex if exact accuracy needed (slower)

### gRPC Flow Control

**How TCP Backpressure Works**:
1. Server stops reading from gRPC stream
2. gRPC internal buffers fill up
3. TCP receive window fills
4. TCP advertises zero window to client
5. Client's TCP send blocks
6. Client's `send()` operations slow down

**Timeout Handling**:
```rust
// Client-side timeout to prevent indefinite blocking
let response = client
    .write_messages(stream)
    .timeout(Duration::from_secs(30))
    .await?;
```

### Performance Impact

**Expected Overhead**:
- Atomic increment/decrement: ~5-10 ns per operation
- Size calculation: Depends on type (LogMsg ~100-500 ns)
- Backpressure check: ~100 ns (atomic load + comparison)
- Total: <1% overhead on message processing

**Benchmarking**:
```rust
#[bench]
fn bench_send_with_size_tracking(b: &mut Bencher) {
    let (tx, rx) = smart_channel::channel();
    b.iter(|| {
        tx.send(test_message()).unwrap();
    });
}
```

---

## Testing Strategy

### Unit Tests

#### Smart Channel Tests
**File**: `crates/utils/re_smart_channel/src/lib.rs`

```rust
#[test]
fn test_queue_bytes_tracking() {
    let (tx, rx) = channel::<TestMessage>();

    // Send messages
    tx.send(TestMessage { data: vec![0; 1000] }).unwrap();
    tx.send(TestMessage { data: vec![0; 2000] }).unwrap();

    // Check sender and receiver agree on queue size
    let sender_bytes = tx.queue_bytes();
    let receiver_bytes = rx.queue_bytes();
    assert_eq!(sender_bytes, receiver_bytes);
    assert!(sender_bytes >= 3000);  // At least data size

    // Receive and verify decrement
    rx.recv().unwrap();
    assert!(rx.queue_bytes() < sender_bytes);

    rx.recv().unwrap();
    assert_eq!(rx.queue_bytes(), 0);
}

#[test]
fn test_size_bytes_implementations() {
    let log_msg = LogMsg::ArrowMsg(...);
    let size = log_msg.size_bytes();
    assert!(size > 0);
    assert!(size >= std::mem::size_of_val(&log_msg) as u64);
}
```

#### gRPC Backpressure Tests
**File**: `crates/store/re_grpc_server/src/lib.rs`

```rust
#[tokio::test]
async fn test_backpressure_applied() {
    let policy = BackpressurePolicy {
        max_queue_bytes: 1000,
        backpressure_delay: Duration::from_millis(10),
        max_backpressure_wait: Duration::from_secs(1),
    };

    let server = create_test_server(policy);

    // Fill queue beyond limit
    for _ in 0..100 {
        server.push_large_message().await;
    }

    // Next send should block
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        server.push_large_message()
    ).await;

    // Should timeout because backpressure is applied
    assert!(result.is_err());
    assert!(start.elapsed() >= Duration::from_millis(100));
}

#[tokio::test]
async fn test_backpressure_timeout() {
    let policy = BackpressurePolicy {
        max_queue_bytes: 1000,
        max_backpressure_wait: Duration::from_millis(100),
        ..Default::default()
    };

    let server = create_test_server(policy);

    // Fill queue and never drain
    fill_queue_forever(&server).await;

    // Should eventually timeout with gRPC error
    let result = server.push_message().await;
    assert!(matches!(result, Err(Status::ResourceExhausted(_))));
}
```

### Integration Tests

#### End-to-End Backpressure Test
**File**: `crates/store/re_grpc_server/tests/backpressure_integration.rs`

```rust
#[tokio::test]
async fn test_e2e_backpressure() {
    // Start viewer with small queue limit
    let viewer = Viewer::new(BackpressurePolicy {
        max_queue_bytes: 10 * 1024 * 1024,  // 10 MiB
        ..Default::default()
    });

    // Connect fast producer
    let producer = Producer::connect(&viewer).await;

    // Send data faster than viewer can process
    let send_task = tokio::spawn(async move {
        for i in 0..10000 {
            producer.send_large_chunk(i).await?;
        }
        Ok::<_, Error>(())
    });

    // Slow down viewer processing
    viewer.set_processing_delay(Duration::from_millis(100));

    // Producer should slow down due to backpressure
    let start = Instant::now();
    send_task.await.unwrap().unwrap();
    let elapsed = start.elapsed();

    // Should take longer due to backpressure
    assert!(elapsed > Duration::from_secs(5));

    // Verify queue never exceeded limit
    let stats = viewer.backpressure_stats();
    assert!(stats.max_queue_bytes_seen <= 12 * 1024 * 1024);  // Some margin
}
```

#### Memory Bloat Prevention Test
**File**: `crates/viewer/re_viewer/tests/memory_bloat.rs`

```rust
#[tokio::test]
async fn test_no_memory_bloat() {
    let viewer = Viewer::new(BackpressurePolicy::default());
    let producer = Producer::connect(&viewer).await;

    // Record baseline memory
    let baseline_memory = viewer.memory_usage();

    // Send large amounts of data
    for _ in 0..1000 {
        producer.send_chunk().await.unwrap();
    }

    // Wait for processing
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Memory should not grow unbounded
    let current_memory = viewer.memory_usage();
    let growth = current_memory - baseline_memory;

    assert!(growth < 150 * 1024 * 1024);  // Less than 150 MiB growth
}
```

### Performance Benchmarks

**File**: `crates/utils/re_smart_channel/benches/throughput.rs`

```rust
#[bench]
fn bench_throughput_with_backpressure(b: &mut Bencher) {
    let (tx, rx) = smart_channel::channel();

    // Spawn receiver
    std::thread::spawn(move || {
        while let Ok(_) = rx.recv() {
            // Simulate processing
            std::thread::sleep(Duration::from_micros(100));
        }
    });

    // Benchmark sender
    b.iter(|| {
        tx.send(test_message()).unwrap();
    });
}
```

### Manual Testing

1. **Web Browser Test**:
   - Open viewer in Chrome/Firefox
   - Connect producer sending 100 MB/s
   - Monitor browser memory (should stay bounded)
   - Verify no "out of memory" crashes

2. **Desktop Stress Test**:
   - Run viewer with high data rate SDK
   - Monitor process memory with `htop`
   - Verify backpressure warnings appear
   - Confirm queue stays under limit

3. **Network Latency Test**:
   - Add artificial network delay
   - Verify backpressure adjusts appropriately
   - Check no message loss

---

## Migration Strategy

### Backwards Compatibility

**API Compatibility**:
- `SmartChannel::channel()` signature unchanged
- Existing code continues to work
- `SizeBytes` trait is additive (doesn't break existing types)

**Opt-In Rollout**:
```rust
// Phase 1: Backpressure disabled by default
impl Default for BackpressurePolicy {
    fn default() -> Self {
        Self {
            enabled: false,  // Disabled initially
            max_queue_bytes: u64::MAX,
        }
    }
}

// Phase 2: Enable with high limits
impl Default for BackpressurePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_queue_bytes: 500 * 1024 * 1024,  // Very high
        }
    }
}

// Phase 3: Enable with tuned limits
impl Default for BackpressurePolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_queue_bytes: platform_default(),  // Optimized
        }
    }
}
```

### Rollout Plan

1. **Week 1-2**: Implement Phase 1 (byte tracking)
   - No behavior change, just metrics
   - Deploy and monitor

2. **Week 3-4**: Implement Phase 2 (gRPC backpressure)
   - Disabled by default
   - Opt-in via environment variable
   - Gather data from beta testers

3. **Week 5-6**: Enable with high limits
   - Catch only extreme cases
   - Monitor for issues
   - Tune based on telemetry

4. **Week 7+**: Tune limits based on data
   - Adjust platform-specific defaults
   - Document best practices
   - Add UI controls

---

## Risks and Mitigations

### Risk 1: Incorrect Size Estimation

**Description**: `SizeBytes` implementations may under/over-estimate actual memory usage.

**Impact**:
- Underestimate → Backpressure applied too late
- Overestimate → Backpressure applied too early

**Mitigation**:
- Start with conservative (over) estimates
- Add debug mode that validates estimates against actual allocator
- Log warnings when estimates seem inaccurate
- Provide `RERUN_VALIDATE_SIZE_BYTES` env var for testing

**Code**:
```rust
#[cfg(debug_assertions)]
fn validate_size_estimate<T: SizeBytes>(value: &T) {
    let estimated = value.size_bytes();
    let actual = measure_actual_heap_size(value);  // Platform-specific

    if (estimated as f64 - actual as f64).abs() / actual as f64 > 0.5 {
        re_log::warn!(
            "Size estimate off by >50%: estimated {} bytes, actual {} bytes",
            estimated, actual
        );
    }
}
```

### Risk 2: Deadlock

**Description**: If viewer stops processing and queue fills, gRPC client could deadlock waiting for capacity.

**Impact**: Complete hang, poor user experience

**Mitigation**:
- Always use timeout in `wait_for_queue_capacity()`
- Return gRPC error on timeout (fails fast)
- Log detailed error with actionable message
- Document client-side timeout configuration

**Code**:
```rust
if start.elapsed() > policy.max_backpressure_wait {
    return Err(tonic::Status::resource_exhausted(
        format!(
            "Viewer cannot process data fast enough. \
            Queue has been full for {:?}. \
            Consider reducing data rate or increasing --max-queue-bytes.",
            policy.max_backpressure_wait
        )
    ));
}
```

### Risk 3: Performance Regression

**Description**: Atomic operations and size calculations could slow down hot path.

**Impact**: Reduced throughput, higher latency

**Mitigation**:
- Benchmark before/after
- Use `Ordering::Relaxed` for non-critical atomics
- Profile with `perf` or `cargo flamegraph`
- Consider sampling (update size every Nth message)

**Acceptance Criteria**:
- <5% throughput regression
- <10ns per-message overhead

### Risk 4: Inconsistent Behavior Across Platforms

**Description**: Web vs desktop may behave differently due to different limits.

**Impact**: Hard to debug, inconsistent user experience

**Mitigation**:
- Document platform-specific defaults clearly
- Make limits visible in UI
- Provide override via env var
- Add platform-detection logic with logging

**Code**:
```rust
re_log::info!(
    "Backpressure policy: max_queue_bytes={} MiB (platform: {})",
    policy.max_queue_bytes / 1024 / 1024,
    if cfg!(target_arch = "wasm32") { "web" } else { "desktop" }
);
```

### Risk 5: Breaking SDK Clients

**Description**: Clients not prepared for backpressure may timeout or fail.

**Impact**: Existing integrations break

**Mitigation**:
- Document behavior change in migration guide
- Provide client-side timeout recommendations
- Add gRPC metadata hint when backpressure is active
- Gradual rollout with opt-in period

**Documentation**:
```markdown
## Backpressure Behavior (0.26.0+)

Starting in version 0.26.0, the Rerun viewer applies backpressure when it
cannot process data fast enough. Your SDK client should handle this:

1. Set appropriate gRPC timeouts (recommended: 30s)
2. Implement retry logic with exponential backoff
3. Monitor for `RESOURCE_EXHAUSTED` status codes
4. Consider reducing data rate if backpressure is persistent

See [docs/backpressure.md](docs/backpressure.md) for details.
```

---

## Success Metrics

### Primary Metrics

1. **Memory Stability**:
   - Max memory usage stays below 2x baseline
   - No OOM crashes on web (currently a problem)
   - 95th percentile memory usage reduced by 30%

2. **No Data Loss**:
   - Zero "dropped messages due to backpressure" warnings
   - All sent messages eventually processed (or explicit error)

3. **Latency Bounds**:
   - E2E latency stays below 5 seconds under normal load
   - Backpressure applied before latency exceeds 10 seconds

### Secondary Metrics

4. **Performance**:
   - <5% throughput regression
   - <10ns per-message overhead
   - No impact on UI frame rate

5. **Observability**:
   - Queue size visible in UI
   - Backpressure events logged
   - Metrics available for monitoring

### Validation Tests

```rust
// Test 1: Memory bounds
assert!(max_memory < baseline_memory * 2);

// Test 2: No message loss
assert_eq!(broadcast_lag_warnings, 0);

// Test 3: Latency bounds
assert!(e2e_latency_p95 < Duration::from_secs(5));

// Test 4: Performance
assert!(throughput_with_backpressure / baseline_throughput > 0.95);
```

---

## Future Enhancements

### Adaptive Backpressure

Instead of fixed thresholds, adjust based on system state:
```rust
pub struct AdaptivePolicy {
    target_latency: Duration,
    current_threshold: AtomicU64,
}

impl AdaptivePolicy {
    fn adjust_threshold(&self, current_latency: Duration) {
        if current_latency > self.target_latency * 2 {
            // Latency too high, lower threshold (more aggressive)
            self.current_threshold.fetch_sub(10 * 1024 * 1024, Ordering::Relaxed);
        } else if current_latency < self.target_latency / 2 {
            // Latency low, raise threshold (less aggressive)
            self.current_threshold.fetch_add(10 * 1024 * 1024, Ordering::Relaxed);
        }
    }
}
```

### Per-Source Backpressure

Different limits for different data sources:
```rust
pub struct PerSourcePolicy {
    default: BackpressurePolicy,
    overrides: HashMap<SmartChannelSource, BackpressurePolicy>,
}
```

### Client-Side Rate Limiting

SDK-side throttling based on server feedback:
```rust
// gRPC metadata hint
response.metadata().insert(
    "x-rerun-queue-usage",
    format!("{:.2}", queue_bytes as f64 / max_bytes as f64)
);

// Client adjusts send rate
if queue_usage > 0.8 {
    send_delay *= 2;  // Slow down
}
```

### Priority-Based Backpressure

Apply backpressure selectively:
```rust
pub enum Priority {
    Critical,   // Never throttle (e.g., blueprint)
    Normal,     // Throttle when queue > 80%
    Bulk,       // Throttle when queue > 50%
}
```

---

## References

### Related Issues
- [#11024](https://github.com/rerun-io/rerun/issues/11024) - Apply backpressure (this issue)
- [#11025](https://github.com/rerun-io/rerun/pull/11025) - Remove `--drop-at-latency`

### Documentation
- `crates/utils/re_smart_channel/README.md` - Smart channel overview
- `crates/store/re_grpc_server/README.md` - gRPC server architecture
- `docs/content/reference/migration/migration-0-25.md` - Migration guide

### External References
- [gRPC Flow Control](https://grpc.io/docs/guides/flow-control/)
- [Tokio Backpressure Patterns](https://tokio.rs/tokio/tutorial/channels#backpressure)
- [Crossbeam Channel Docs](https://docs.rs/crossbeam/latest/crossbeam/channel/)

---

## Appendix: Code Locations

### Key Files to Modify

| File | Lines | Purpose |
|------|-------|---------|
| `crates/utils/re_smart_channel/src/lib.rs` | 32-294 | Add `queue_bytes` to `SharedStats`, define `SizeBytes` trait |
| `crates/utils/re_smart_channel/src/sender.rs` | 34, 119 | Track bytes on send, add `queue_bytes()` method |
| `crates/utils/re_smart_channel/src/receiver.rs` | 45, 62, 83 | Decrement bytes on receive, add `queue_bytes()` method |
| `crates/utils/re_smart_channel/src/receive_set.rs` | 113 | Add `queue_bytes()` aggregation |
| `crates/store/re_grpc_server/src/lib.rs` | 954-984 | Add backpressure to `write_messages()` |
| `crates/store/re_grpc_server/src/lib.rs` | 709-803 | Add backpressure monitoring to `EventLoop` |
| `crates/store/re_log_types/src/lib.rs` | Various | Implement `SizeBytes` for `LogMsg` types |
| `crates/viewer/re_viewer/src/ui/top_panel.rs` | 96, 456 | Add queue metrics to UI |

### Test Files to Create

| File | Purpose |
|------|---------|
| `crates/utils/re_smart_channel/tests/byte_tracking.rs` | Unit tests for byte tracking |
| `crates/store/re_grpc_server/tests/backpressure.rs` | Integration tests for gRPC backpressure |
| `crates/viewer/re_viewer/tests/memory_bloat.rs` | E2E tests for memory bounds |
| `crates/utils/re_smart_channel/benches/throughput.rs` | Performance benchmarks |

---

## Timeline Estimate

| Phase | Duration | Deliverable |
|-------|----------|-------------|
| Phase 1: Smart Channel Byte Tracking | 1-2 weeks | Byte tracking merged, no behavior change |
| Phase 2: gRPC Backpressure | 2-3 weeks | Backpressure implemented, opt-in |
| Phase 3: Configuration & Tuning | 1 week | CLI options, defaults tuned |
| Phase 4: Monitoring & Observability | 1 week | UI metrics, logging complete |
| Testing & Refinement | 1-2 weeks | All tests passing, benchmarks acceptable |
| Documentation & Rollout | 1 week | Docs written, migration guide ready |
| **Total** | **7-10 weeks** | Feature complete and production-ready |

---

## Conclusion

This implementation plan addresses the root cause of memory bloat in the Rerun viewer by adding comprehensive byte-size tracking to smart channels and implementing gRPC-level backpressure. The phased approach allows for incremental delivery, thorough testing, and safe rollout.

The design prioritizes:
- **Safety**: Timeouts prevent deadlocks, errors fail fast
- **Observability**: Comprehensive metrics and logging
- **Performance**: Minimal overhead (<5% regression target)
- **Flexibility**: Configurable policies, platform-specific defaults

Upon completion, the viewer will gracefully handle high-throughput data sources without unbounded memory growth, providing a stable experience especially on resource-constrained web platforms.
