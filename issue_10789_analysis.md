# Issue #10789: Performance Hickup When First Showing a Video

**Analysis Date:** 2025-11-09
**Issue URL:** https://github.com/rerun-io/rerun/issues/10789
**Status:** Open (Reopened)
**Milestone:** 0.26.0
**Labels:** feat-video, 📉 performance, 🪳 bug

## Executive Summary

When displaying H264 videos natively for the first time, the Rerun viewer experiences significant UI stalls:
- **Release builds:** ~100ms delay
- **Debug builds:** Several seconds delay

The root cause is blocking operations during ffmpeg process initialization and version checking, particularly on macOS where initialization can take up to **7 seconds** on cold boot. While PR #10797 addressed the initial manifestation, the issue persists through additional code paths, particularly when opening `.mcap` files containing videos via CLI.

---

## Issue Timeline

1. **August 4, 2025** - Issue opened by @emilk
2. **August 5, 2025** - PR #10797 merged ("Fix GUI hickup when starting native video player")
3. **August 11, 2025** - Issue reopened after discovering persistent blocking via `for_executable_poll` and global `VersionCache` lock

---

## Root Cause Analysis

### Primary Problem: FFmpeg Version Checking

**Location:** `crates/utils/re_video/src/decode/ffmpeg_cli/version.rs:102-112`

```rust
pub fn for_executable_blocking(path: Option<&std::path::Path>) -> FfmpegVersionResult {
    re_tracing::profile_function!();

    let modification_time = file_modification_time(path)?;
    VersionCache::global(|cache| {
        cache
            .version(path, modification_time)
            .block_until_ready()  // <-- BLOCKING!
            .clone()
    })
}
```

**Critical Documentation Warning:**
> "WARNING: this can block for SEVEN SECONDS on Mac, but usually only the first time after a reboot. NEVER call this on the GUI thread!"

**Why it takes so long on macOS:**
The delay is caused by dynamic library loading and plugin discovery when spawning the ffmpeg process for version detection. This is a known macOS behavior that occurs primarily on cold boot.

### Secondary Problem: Global Lock Contention

**Location:** `crates/utils/re_video/src/decode/ffmpeg_cli/version.rs:147-151`

```rust
impl VersionCache {
    fn global<R>(f: impl FnOnce(&mut Self) -> R) -> R {
        static CACHE: std::sync::LazyLock<Mutex<VersionCache>> =
            std::sync::LazyLock::new(|| Mutex::new(VersionCache::default()));
        f(&mut CACHE.lock())  // <-- GLOBAL MUTEX LOCK
    }
}
```

**Impact:**
- Any thread attempting to check ffmpeg version will block on this global mutex
- If the first check is still in progress (e.g., during the 7-second Mac delay), subsequent threads are blocked
- This affects opening multiple videos or `.mcap` files containing videos

---

## Detailed Blocking Operations Inventory

### 1. File I/O for Metadata

**Location:** `crates/utils/re_video/src/decode/ffmpeg_cli/version.rs:122-139`

```rust
fn file_modification_time(
    path: Option<&std::path::Path>,
) -> Result<Option<std::time::SystemTime>, FFmpegVersionParseError> {
    Ok(if let Some(path) = path {
        path.metadata()  // <-- BLOCKING FILE I/O!
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    FFmpegVersionParseError::FFmpegNotFound(path.display().to_string())
                } else {
                    FFmpegVersionParseError::RetrieveFileModificationTime(err.to_string())
                }
            })?
            .modified()
            .ok()
    } else {
        None
    })
}
```

**Risk:** Low to moderate - filesystem metadata reads are usually fast but can be slow on network filesystems or under heavy I/O load.

### 2. FFmpeg Process Spawning

**Location:** `crates/utils/re_video/src/decode/ffmpeg_cli/ffmpeg.rs:318-322`

```rust
let mut ffmpeg = ffmpeg_command
    .spawn()  // <-- PROCESS SPAWN - Can be slow
    .map_err(Error::FailedToStartFfmpeg)?;
```

**Risk:** Moderate - process spawning overhead, particularly on macOS where dynamic library loading occurs.

### 3. Blocking Channel I/O

**Location:** `crates/utils/re_video/src/decode/ffmpeg_cli/ffmpeg.rs:494-555`

```rust
fn write_ffmpeg_input(
    ffmpeg_stdin: &mut dyn std::io::Write,
    frame_data_rx: &Receiver<FFmpegFrameData>,
    output_sender: &OutputSender,
    codec_meta: &CodecMeta,
) {
    let mut state = AnnexBStreamState::default();

    while let Ok(data) = frame_data_rx.recv() {  // <-- BLOCKING CHANNEL RECV
        // ...
        if let Err(err) = write_result {
            // ...
        } else {
            ffmpeg_stdin.flush().ok();  // <-- BLOCKING FLUSH
        }
    }
}
```

**Risk:** High if called on UI thread - channel operations and I/O flushes can block indefinitely.

### 4. Video Stream Cache RwLock Contention

**Location:** `crates/viewer/re_viewer_context/src/cache/video_stream_cache.rs:534-538`

```rust
for entry in self.0.values_mut() {
    entry.used_this_frame.store(false, Ordering::Release);
    let video_stream = entry.video_stream.write();  // <-- BLOCKING WRITE LOCK
    video_stream.video_renderer.begin_frame();
}
```

**Risk:** Moderate - write locks can block if background threads are decoding video frames.

### 5. Video Player Mutex

**Location:** `crates/viewer/re_renderer/src/video/mod.rs:206,310,341`

```rust
pub struct Video {
    debug_name: String,
    video_description: re_video::VideoDataDescription,
    players: Mutex<HashMap<VideoPlayerStreamId, PlayerEntry>>,  // <-- MUTEX
    decode_settings: DecodeSettings,
}

// Called on UI thread:
let mut players = self.players.lock();  // <-- BLOCKING MUTEX LOCK
```

**Risk:** High - this mutex is accessed from both UI and background threads.

### 6. H264 Parsing

**Location:** `crates/utils/re_video/src/h264.rs:104-144`

```rust
pub fn detect_h264_annexb_gop(
    mut sample_data: &[u8],
) -> Result<GopStartDetection, DetectGopStartError> {
    let mut reader = AnnexBReader::accumulate(H264GopDetectionState::default());

    while !sample_data.is_empty() {
        const MAX_CHUNK_SIZE: usize = 256;
        let chunk_size = MAX_CHUNK_SIZE.min(sample_data.len());
        reader.push(&sample_data[..chunk_size]);  // <-- Synchronous parsing

        let handler = reader.nal_handler_ref();
        if handler.idr_frame_found && matches!(handler.coding_details_from_sps, Some(Ok(_))) {
            break;
        }
        sample_data = &sample_data[chunk_size..];
    }
    // ...
}
```

**Risk:** Low to moderate - parsing is chunked, but can still be slow for large samples.

### 7. Thread Cleanup (Currently Disabled)

**Location:** `crates/utils/re_video/src/decode/ffmpeg_cli/ffmpeg.rs:430-491`

```rust
impl Drop for FFmpegProcessAndListener {
    fn drop(&mut self) {
        re_tracing::profile_function!();

        // Unfortunately, even with the above measures, it can still happen
        // that the listen threads take occasionally 100ms and more to shut down.
        if false {  // <-- Currently disabled due to blocking concerns
            if let Some(write_thread) = self.write_thread.take()
                && write_thread.join().is_err()  // <-- Would BLOCK
            {
                re_log::error!("Failed to join ffmpeg listener thread.");
            }
            if let Some(listen_thread) = self.listen_thread.take()
                && listen_thread.join().is_err()  // <-- Would BLOCK
            {
                re_log::error!("Failed to join ffmpeg listener thread.");
            }
        }
    }
}
```

**Note:** Thread joins are currently disabled precisely because they can block for 100ms+.

---

## What Was Fixed in PR #10797

The merged PR addressed the initial manifestation by:

1. **Refactored ffmpeg version check** to prevent direct UI thread blocking
2. **Added profiling scopes** for better performance monitoring
3. **Enhanced documentation** with warnings about blocking behavior
4. **Moved version check off the UI thread** during application startup

**However, the issue persists because:**
- The global `VersionCache` mutex still blocks when multiple operations check ffmpeg version concurrently
- Opening `.mcap` files with videos triggers the same blocking path
- The `for_executable_poll` function still uses `for_executable_blocking` internally

---

## Suggested Improvements

### 1. **Adopt `_blocking` Naming Convention** (As proposed in issue)

Add a `_blocking` suffix to any function that directly or indirectly calls blocking operations. This makes blocking behavior visible during code review.

**Example refactoring:**
```rust
// Current:
pub fn file_modification_time(path: Option<&std::path::Path>) -> Result<...>

// Proposed:
pub fn file_modification_time_blocking(path: Option<&std::path::Path>) -> Result<...>
```

**Benefits:**
- Makes blocking behavior explicit in function signatures
- Helps reviewers catch UI thread violations
- Documents performance characteristics

### 2. **Lazy Async Version Checking**

Replace the global blocking version check with an async approach using tokio or similar:

```rust
pub async fn for_executable_async(path: Option<&std::path::Path>) -> FfmpegVersionResult {
    let modification_time = file_modification_time_async(path).await?;
    VersionCache::global_async(|cache| async move {
        cache.version_async(path, modification_time).await.clone()
    }).await
}
```

**Benefits:**
- UI thread never blocks on version checking
- Background threads can await completion without holding locks
- Natural fit for async Rust ecosystem

### 3. **Optimize Global Lock Strategy**

Replace the single global `Mutex<VersionCache>` with a more granular approach:

```rust
impl VersionCache {
    fn global<R>(f: impl FnOnce(&mut Self) -> R) -> R {
        // Use RwLock for read-heavy workloads
        static CACHE: std::sync::LazyLock<RwLock<VersionCache>> =
            std::sync::LazyLock::new(|| RwLock::new(VersionCache::default()));

        // Most operations only need read access
        let cache = CACHE.read();
        if let Some(result) = cache.try_get(path) {
            return result;
        }
        drop(cache);

        // Only lock for write if we need to compute
        f(&mut CACHE.write())
    }
}
```

**Benefits:**
- Multiple threads can read cached versions concurrently
- Only the first version check blocks subsequent checks
- Reduced contention on the global lock

### 4. **Pre-warm the Version Cache**

During application initialization (in a background thread), pre-populate the version cache:

```rust
pub fn warmup_version_cache() {
    std::thread::spawn(|| {
        // This blocks, but on a background thread during startup
        let _ = FFmpegVersion::for_executable_blocking(None);
    });
}
```

**Benefits:**
- First video load won't trigger expensive version check
- Amortizes the 7-second Mac delay across application startup
- User doesn't see delay when opening first video

### 5. **Lock-Free Atomics for Hot Paths**

Replace `Mutex<HashMap<...>>` with concurrent data structures for frequently accessed paths:

```rust
use dashmap::DashMap;

pub struct Video {
    debug_name: String,
    video_description: re_video::VideoDataDescription,
    players: Arc<DashMap<VideoPlayerStreamId, PlayerEntry>>,  // <-- Lock-free
    decode_settings: DecodeSettings,
}
```

**Benefits:**
- No blocking on player access
- Better performance under concurrent access
- Reduced risk of UI thread stalls

### 6. **Timeout-Protected Operations**

Add timeouts to all blocking operations that touch the UI thread:

```rust
pub fn for_executable_with_timeout(
    path: Option<&std::path::Path>,
    timeout: Duration,
) -> Result<FfmpegVersionResult, TimeoutError> {
    use std::sync::mpsc::channel;

    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let result = for_executable_blocking(path);
        let _ = tx.send(result);
    });

    rx.recv_timeout(timeout)
        .map_err(|_| TimeoutError::VersionCheckTimedOut)
}
```

**Benefits:**
- Prevents unbounded blocking even if underlying operation hangs
- Graceful degradation (can fall back to software decoding)
- Better user experience on slow systems

### 7. **Deferred Video Initialization**

Don't initialize ffmpeg until the video is actually visible in the viewport:

```rust
pub fn should_initialize_video(&self, video_bounds: Rect, viewport: Rect) -> bool {
    // Only initialize if video is in viewport or will be soon
    viewport.intersects(video_bounds) || viewport.distance_to(video_bounds) < PRELOAD_DISTANCE
}
```

**Benefits:**
- Videos scrolled past don't trigger initialization
- Reduces unnecessary ffmpeg process spawning
- Better performance when loading recordings with many videos

### 8. **Process Pool for FFmpeg**

Maintain a pool of pre-spawned ffmpeg processes to amortize startup cost:

```rust
pub struct FFmpegProcessPool {
    available: Arc<Mutex<Vec<FFmpegProcess>>>,
    max_size: usize,
}

impl FFmpegProcessPool {
    pub fn acquire(&self) -> FFmpegProcess {
        self.available.lock()
            .pop()
            .unwrap_or_else(|| self.spawn_new())
    }

    pub fn release(&self, process: FFmpegProcess) {
        let mut available = self.available.lock();
        if available.len() < self.max_size {
            available.push(process);
        }
    }
}
```

**Benefits:**
- Eliminates per-video process spawn overhead
- Reduces latency for subsequent video loads
- Better resource utilization

### 9. **Caching Strategy Improvements**

The current version cache could be improved:

**Current issues:**
- Checks file modification time synchronously
- No TTL or invalidation strategy
- Global lock for all operations

**Proposed improvements:**
```rust
struct VersionCacheEntry {
    version: FfmpegVersionResult,
    checked_at: Instant,
    modification_time: Option<SystemTime>,
    ttl: Duration,
}

impl VersionCacheEntry {
    fn is_stale(&self) -> bool {
        self.checked_at.elapsed() > self.ttl
    }

    fn needs_revalidation(&self, current_mtime: Option<SystemTime>) -> bool {
        self.modification_time != current_mtime || self.is_stale()
    }
}
```

**Benefits:**
- Time-based invalidation prevents indefinite stale data
- Modification time checks can be async
- Clearer cache semantics

### 10. **Profiling and Monitoring**

Add comprehensive profiling to all blocking operations:

```rust
pub fn for_executable_blocking(path: Option<&std::path::Path>) -> FfmpegVersionResult {
    re_tracing::profile_function!();

    // Add explicit timing metrics
    let start = Instant::now();
    let result = {
        let modification_time = {
            re_tracing::profile_scope!("file_modification_time");
            file_modification_time(path)?
        };

        re_tracing::profile_scope!("version_cache_access");
        VersionCache::global(|cache| {
            cache.version(path, modification_time).block_until_ready().clone()
        })
    };

    let elapsed = start.elapsed();
    if elapsed > Duration::from_millis(100) {
        re_log::warn!("FFmpeg version check took {:?} (path: {:?})", elapsed, path);
    }

    result
}
```

**Benefits:**
- Identifies which operations are actually slow in production
- Helps prioritize optimization efforts
- Provides data for regression detection

---

## Implementation Priority

### High Priority (Immediate Impact)

1. **Pre-warm version cache** - Easy win, addresses the 7-second Mac delay
2. **Adopt `_blocking` naming convention** - Prevents future regressions
3. **Add profiling to all blocking operations** - Provides data for further optimization

### Medium Priority (Significant Impact, More Work)

4. **Optimize global lock strategy** - Reduces contention
5. **Timeout-protected operations** - Prevents unbounded blocking
6. **Deferred video initialization** - Reduces unnecessary work

### Low Priority (Nice to Have)

7. **Lazy async version checking** - Requires async runtime integration
8. **Process pool for FFmpeg** - Complex, requires lifecycle management
9. **Lock-free atomics for hot paths** - Adds dependencies, requires careful design

---

## Testing Recommendations

### Performance Tests

1. **Cold start test**: Measure time from app launch to first video frame on macOS (fresh boot)
2. **Multi-video test**: Open `.mcap` file with 10+ videos, measure UI responsiveness
3. **Concurrent access test**: Trigger multiple video decodes simultaneously
4. **Lock contention test**: Measure time spent waiting on locks during video playback

### Regression Tests

1. **UI thread monitoring**: Assert that UI thread never blocks >16ms during video operations
2. **FFmpeg version caching**: Verify version is only checked once per executable/mtime combination
3. **Graceful degradation**: Ensure video playback fails gracefully if ffmpeg unavailable

### Platform-Specific Tests

1. **macOS cold boot**: Verify 7-second delay doesn't block UI
2. **Network filesystem**: Test with ffmpeg on NFS/SMB mount
3. **Windows dynamic libraries**: Test with different FFmpeg distributions

---

## Conclusion

The video performance hickup is a multi-faceted issue stemming from:

1. **Expensive FFmpeg version checking** (up to 7 seconds on macOS)
2. **Global lock contention** in the version cache
3. **Blocking file I/O** for metadata
4. **Synchronous process spawning and communication**

While PR #10797 addressed the most visible symptom, the underlying architectural issues remain. The recommended improvements focus on:

- **Prevention**: Naming conventions to catch blocking operations during code review
- **Mitigation**: Pre-warming caches, timeouts, and deferred initialization
- **Optimization**: Lock-free data structures and async operations
- **Monitoring**: Comprehensive profiling to catch regressions

The highest-impact improvements (pre-warming, naming conventions, profiling) are also the easiest to implement and should be prioritized.

---

## Implementation Status

The following high-priority improvements have been implemented:

### ✅ Completed

1. **Adopted `_blocking` naming convention**
   - Renamed `file_modification_time` → `file_modification_time_blocking`
   - Renamed `ffmpeg_version` → `ffmpeg_version_blocking`
   - Added `_blocking` suffix documentation to all blocking functions
   - Added explicit warnings about not calling on GUI thread

2. **Added comprehensive profiling to blocking operations**
   - Added timing instrumentation to `for_executable_blocking`
   - Added `re_tracing::profile_scope!` to all blocking I/O operations
   - Added warnings when operations take >100ms
   - Added profiling to version cache lock acquisition

3. **Implemented version cache pre-warming**
   - Created `warmup_version_cache()` function for background initialization
   - Exported for use during application startup
   - Provides graceful error handling (logs but doesn't fail app startup)

4. **Added timeout protection**
   - Created `for_executable_blocking_with_timeout()` function
   - New `TimeoutError` type for timeout scenarios
   - Enables graceful degradation when version checking is slow

5. **Improved lock profiling**
   - Added profiling scopes to version cache lock acquisition
   - Better visibility into lock contention issues

### 📝 Usage Instructions

**For pre-warming the cache during application startup:**

```rust
use re_video::decode::ffmpeg_cli::warmup_version_cache;

// Call this early in your application initialization,
// e.g., in viewer startup or SDK initialization
warmup_version_cache();
```

**For timeout-protected version checking:**

```rust
use re_video::decode::ffmpeg_cli::FFmpegVersion;
use std::time::Duration;

// Check with a 5-second timeout
match FFmpegVersion::for_executable_blocking_with_timeout(None, Duration::from_secs(5)) {
    Ok(Ok(version)) => println!("FFmpeg version: {}", version),
    Ok(Err(e)) => eprintln!("Version check failed: {}", e),
    Err(timeout_err) => eprintln!("Version check timed out: {}", timeout_err),
}
```

### 🔄 Remaining Improvements (Not Implemented)

The following improvements from the analysis were not implemented in this iteration:

- **RwLock optimization**: Cannot use RwLock due to `Promise` type not being `Sync`
- **Deferred video initialization**: Requires changes to viewer viewport logic
- **FFmpeg process pool**: Complex lifecycle management, needs more design work
- **Lock-free data structures**: Would require significant architectural changes

These can be addressed in future iterations if profiling shows they're necessary.

---

## References

- **Issue:** https://github.com/rerun-io/rerun/issues/10789
- **Fixed PR:** https://github.com/rerun-io/rerun/pull/10797
- **Related Code:**
  - `crates/utils/re_video/src/decode/ffmpeg_cli/version.rs` (✏️ modified)
  - `crates/utils/re_video/src/decode/ffmpeg_cli/mod.rs` (✏️ modified)
  - `crates/utils/re_video/src/decode/ffmpeg_cli/ffmpeg.rs`
  - `crates/viewer/re_viewer_context/src/cache/video_stream_cache.rs`
  - `crates/viewer/re_renderer/src/video/mod.rs`
  - `crates/utils/re_video/src/h264.rs`
