# Rerun Query Cache System - Quick Reference

## File Structure & Absolute Paths

```
/home/user/rerun/crates/store/re_query/
├── src/
│   ├── lib.rs                              # Public API exports
│   ├── cache.rs                      (454) # QueryCache coordinator, invalidation handling
│   ├── latest_at.rs                  (704) # LatestAt cache implementation & queries
│   ├── range.rs                      (317) # Range cache implementation & queries
│   ├── cache_stats.rs                (109) # Cache statistics utilities
│   ├── storage_engine.rs                   # StorageEngine wrapper for concurrent access
│   ├── clamped_zip/
│   │   ├── mod.rs                         # Component batching (handles variable length data)
│   │   └── generated.rs                   # Generated clamped_zip functions
│   ├── range_zip/
│   │   ├── mod.rs
│   │   └── generated.rs
│   └── bin/
│       ├── clamped_zip.rs
│       └── range_zip.rs
├── benches/
│   └── latest_at.rs                  (328) # Performance benchmarks (1M data points)
├── examples/
│   ├── query_latest_at.rs
│   ├── query_range.rs
│   └── range_zip.rs
├── tests/
│   ├── latest_at.rs
│   └── range.rs
└── README.md                                # Crate description

Total lines of code: ~2,500 (excluding tests/benches)
```

## Quick Reference: Key Data Structures

### Main Cache Coordinator
```rust
QueryCache {
    store: ChunkStoreHandle,
    store_id: StoreId,
    might_require_clearing: RwLock<IntSet<EntityPath>>,      // Clearing optimization
    latest_at_per_cache_key: RwLock<HashMap<QueryCacheKey, Arc<RwLock<LatestAtCache>>>>,
    range_per_cache_key: RwLock<HashMap<QueryCacheKey, Arc<RwLock<RangeCache>>>>,
}
```

### Cache Key (identifies unique cache)
```rust
QueryCacheKey {
    entity_path: EntityPath,
    timeline_name: TimelineName,
    component: ComponentIdentifier,
}
```

### LatestAtCache (per-component query time-series cache)
```rust
LatestAtCache {
    cache_key: QueryCacheKey,
    per_query_time: BTreeMap<TimeInt, LatestAtCachedChunk>,    // O(log n) lookup
    pending_invalidations: BTreeSet<TimeInt>,                  // Deferred invalidations
}

LatestAtCachedChunk {
    unit: UnitChunkShared,                      // The cached data chunk
    is_reference: bool,                         // true = amortized (0 bytes)
}
```

### RangeCache (chunk-based cache for range queries)
```rust
RangeCache {
    cache_key: QueryCacheKey,
    chunks: HashMap<ChunkId, RangeCachedChunk>,               // O(1) lookup
    pending_invalidations: BTreeSet<ChunkId>,                 // Deferred invalidations
}

RangeCachedChunk {
    chunk: Chunk,
    resorted: bool,    // true = was copied; false = reference (0 bytes)
}
```

## Query Flow Diagram

```
QueryCache::latest_at(query, entity_path, components)
│
├─ STEP 1: Pre-filtering (lines 60-62 latest_at.rs)
│  └─ Filter components that actually exist on timeline
│     (avoids expensive cache misses for non-existent data)
│
├─ STEP 2: Clear component checking (lines 82-138)
│  ├─ Check might_require_clearing set
│  ├─ Walk entity hierarchy for Clear components
│  └─ Cache clear queries (most hits, very cheap)
│
└─ STEP 3: Query each component (lines 140-163)
   ├─ Get or create cache for (entity, timeline, component)
   ├─ Handle pending invalidations
   ├─ Check cache hit (O(log n) BTreeMap lookup)
   └─ If miss: Query store (expensive, O(m) where m=relevant chunks)
```

## Cache Hit vs Miss Performance

### Cache Hit Path (Fast)
```
latest_at(entity, timeline, component, time)
  ├─ Lock top-level HashMap (milliseconds)
  ├─ Get Arc clone to individual cache (instant)
  ├─ Lock individual cache (milliseconds)
  ├─ per_query_time.get(&query_time) → BTreeMap O(log n)
  └─ Return cached data (instant)
```

### Cache Miss Path (Slow)
```
latest_at(entity, timeline, component, time) [CACHE MISS]
  ├─ Everything above, but miss at BTreeMap
  ├─ store.latest_at_relevant_chunks() → Index scan O(n)
  ├─ filter_map over results (max by TimeInt+RowId)
  ├─ Extract unit chunk
  ├─ Cache result at actual data_time (O(log n) insert)
  └─ If query_time != data_time: add reference entry
```

**Cost insight**: Cache misses require full store query, which is why pre-filtering is essential.

## Memory Management

### Reference Deduplication Example
```
Query results accessing static data at different times:

Time 100: latest_at(time=100, component=Color) → hits static data
  → Cache[100] = {unit: StaticChunk, is_reference: false}  (costs bytes)

Time 150: latest_at(time=150, component=Color) → hits same static data
  → Cache[150] = {unit: StaticChunk, is_reference: true}   (costs 0 bytes!)
  → heap_size_bytes() counts this as 0 due to is_reference flag

Time 200: latest_at(time=200, component=Color) → hits same static data
  → Cache[200] = {unit: StaticChunk, is_reference: true}   (costs 0 bytes!)
```

**Memory savings**: Without this, every query at different times would duplicate the chunk.

### Static Data Cache Bypass
```
Latest static data lookup stores entry at actual data_time, not query_time.
Static data recomputation is cheap, so avoid polluting cache with
per-query-time entries for static data.

This prevents unbounded growth from:
  for time in 0..1_000_000 {
      latest_at(time, Color)  // All hit static data
  }

Without bypass: 1 million cache entries!
With bypass: 1 cache entry
```

## Invalidation Strategy

### Deferred Invalidation (Frame Batching)
```
Store event occurs (data inserted/deleted)
  → on_events() callback
    ├─ Compact events to avoid duplicates
    ├─ Store invalidation in pending_invalidations set
    ├─ Release locks
    └─ Return immediately (O(1) per cache key)

Next query on that cache:
  → cache.handle_pending_invalidation()
    ├─ Apply all accumulated invalidations
    └─ Clear pending_invalidations set

Benefit: Frame rendering naturally batches invalidations
         Single event loop processes many invalidations together
```

## Performance Optimization Checklist

When debugging query performance, check:

1. **Pre-filtering working?** (removes non-existent components)
   - `store.entity_has_component_on_timeline()` calls should be fast
   - Check component doesn't exist → no store query

2. **might_require_clearing helping?** (skips clear checks for most entities)
   - Typical: Most entities skip clear logic entirely
   - Problematic: Every entity does full clear traversal

3. **Cache hit rate?** (should be high for stable queries)
   - Same (entity, timeline, component, time) → cache hit
   - Different times → might need reference deduplication

4. **Range cache sorting?** (potential O(n log n) cost)
   - Unhappy path: chunks not pre-sorted → need sorting
   - Happy path: chunks already sorted → no cost
   - TODO #7008: This is a known optimization target

5. **Memory pressure?** (cache getting too large?)
   - Check `purge_fraction_of_ram()` is being called
   - Monitor ratio of effective vs actual size bytes

## Known Limitations & TODOs

| Issue | Location | Severity | Notes |
|-------|----------|----------|-------|
| Static event handling slow | cache.rs:403 | Low | Acknowledged as "horribly stupid and slow" but rarely happens |
| Range sorting unhappy path | range.rs:270 | Medium | TODO #7008 - unnecessary sorting on cache miss |
| Pending invalidations unbounded | latest_at.rs:686 | Low | TODO #5974 - could grow if data never gets cached |
| DataIndex abstraction needed | latest_at.rs:24 | Low | Could improve index comparison operations |

## Running Benchmarks

```bash
# Full benchmark suite
cargo bench -p re_query --bench latest_at

# Specific test
cargo bench -p re_query --bench latest_at -- arrow_mono_points2/query

# With test setup (fast)
cargo test -p re_query --bench latest_at

# Release mode (slow but accurate)
cargo bench --release -p re_query --bench latest_at
```

### Benchmark Metrics
- **NUM_FRAMES_POINTS**: 1 (debug) vs 1,000 (release) frames
- **NUM_POINTS**: 1 (debug) vs 1,000 (release) points
- **Throughput**: Elements/second (higher is better)

## Debugging Tips

### Enable Profiling
```rust
// These calls already have profiling:
cache.handle_pending_invalidation();      // Deferred, batched
cache.latest_at();                        // Per-query (no scope to reduce overhead)
cache.on_events();                        // Event processing
  ├─ "compact events"
  ├─ "static"
  └─ "temporal"
```

### Check Cache State
```rust
// Get cache statistics
let stats = cache.stats();
// Returns: total_chunks, total_effective_size_bytes, total_actual_size_bytes

// Check clearing optimization
let might_require_clearing = cache.might_require_clearing.read();
// Should be small set (only entities with Clear components)
```

### Memory Analysis
```
effective_size = what memory WOULD be used if all chunks copied
actual_size = real memory WITH deduplication & reference tracking

Ratio = actual_size / effective_size (should be << 1.0)
```

## Integration Points

### ChunkStore Integration
- `QueryCache` implements `ChunkStoreSubscriber` trait
- Registered to receive all store events via `on_events()`
- Events are compacted before cache invalidation

### Entity Path Hierarchy
- Clear components affect entities and their children
- `might_require_clearing` tracks all potentially affected entities
- Traversal happens in `latest_at()` for each query

### Timeline Management
- Each timeline has independent cache entries
- `QueryCacheKey` includes timeline name
- Pre-filters by timeline to avoid cross-timeline queries

## Summary: The Core Insight

Rerun's query cache is optimized for **frame-based rendering workloads** where:
- Same queries repeat frame-to-frame (cache hits dominate)
- Temporal locality is high (queries cluster around frame times)
- Most entities have no Clear data (might_require_clearing helps)
- Memory matters but speed matters more

Cache misses are expensive but rare in steady state, making the design
simple and efficient for the common case while accepting higher miss costs.

