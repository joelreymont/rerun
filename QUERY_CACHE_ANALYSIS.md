# Rerun Query & Caching System Analysis

## Overview
The Rerun query system is built around two primary caching mechanisms: **LatestAtCache** and **RangeCache**, managed by the **QueryCache** coordinator. These are lazy-loading caches that avoid unnecessary computation while managing memory efficiency.

---

## 1. How QueryCache Works

### Architecture
The `QueryCache` is a ref-counted, inner-mutable handle to a shared cache serving as a `ChunkStoreSubscriber`. It coordinates two independent caching layers:

```rust
pub struct QueryCache {
    pub(crate) store: ChunkStoreHandle,                          // Reference to underlying data store
    pub(crate) store_id: StoreId,                               // Store identifier
    pub(crate) might_require_clearing: RwLock<IntSet<EntityPath>>,  // Optimization: tracks entities with Clear components
    pub(crate) latest_at_per_cache_key: RwLock<HashMap<QueryCacheKey, Arc<RwLock<LatestAtCache>>>>,
    pub(crate) range_per_cache_key: RwLock<HashMap<QueryCacheKey, Arc<RwLock<RangeCache>>>>,
}
```

### Key Design Principles

#### 1. **Lazy Caching on First Access**
- Data is NOT cached proactively
- Cache entries are created only when queries first request them
- This reduces memory overhead for unused query patterns

#### 2. **Deferred Invalidation (Micro-batching)**
```rust
// From cache.rs line 532-533:
// "Invalidation is deferred to query time because it is far more efficient that way: 
// the frame time effectively behaves as a natural micro-batching mechanism."
```

- Invalidations triggered by store events are NOT applied immediately
- Instead, they're collected in `pending_invalidations` sets
- Applied at query time when cache locks are acquired anyway
- This reduces lock contention and batches invalidation work

#### 3. **Arc-based Lock Strategy**
```rust
pub(crate) latest_at_per_cache_key: RwLock<HashMap<QueryCacheKey, Arc<RwLock<LatestAtCache>>>>,
```

- Top-level HashMap is locked only briefly to get Arc reference
- Individual cache locks are held during query execution
- Allows concurrent queries on different caches without blocking lock acquisitions

### Cache Key Structure
```rust
pub struct QueryCacheKey {
    pub entity_path: EntityPath,
    pub timeline_name: TimelineName,
    pub component: ComponentIdentifier,
}
```

- Each unique (entity, timeline, component) combination has its own cache
- Enables fine-grained cache organization and independent invalidation

---

## 2. Caching Strategies

### 2.1 LatestAtCache Strategy

#### Data Structure
```rust
pub struct LatestAtCache {
    pub cache_key: QueryCacheKey,
    pub per_query_time: BTreeMap<TimeInt, LatestAtCachedChunk>,  // Keyed by query time
    pub pending_invalidations: BTreeSet<TimeInt>,
}

pub struct LatestAtCachedChunk {
    pub unit: UnitChunkShared,
    pub is_reference: bool,  // true if this is just a pointer to another cache entry
}
```

#### Cache Hit Path (Ideal Case)
```rust
if let Some(cached) = per_query_time.get(&query.at()) {
    return Some(cached.unit.clone());  // O(log n) lookup in BTreeMap
}
```

#### Cache Miss Handling
1. Query goes to ChunkStore for relevant chunks
2. Find highest indexed result from all returned chunks
3. Cache result indexed by **actual data time** (not query time)
4. If query time != data time, add cache entry with `is_reference: true`

#### Reference Deduplication Optimization
```rust
// From latest_at.rs lines 657-666:
// For non-static queries hitting static data, cache entries are marked as references
// These references count as 0 bytes in heap_size_bytes() calculation
// Prevents query-time cache from growing unnecessarily
```

**Cost savings**: Eliminates redundant cache entries for queries that return the same static data at different times.

#### Negative Caching
- If a query returns no data, still caches this fact
- Prevents re-running expensive store queries that would yield nothing
- Essential performance optimization given "we run _a lot_ of queries each frame to figure out what to render"

### 2.2 RangeCache Strategy

#### Data Structure
```rust
pub struct RangeCache {
    pub cache_key: QueryCacheKey,
    pub chunks: HashMap<ChunkId, RangeCachedChunk>,  // HashMap for O(1) access
    pub pending_invalidations: BTreeSet<ChunkId>,
}

pub struct RangeCachedChunk {
    pub chunk: Chunk,
    pub resorted: bool,  // Was chunk modified during caching?
}
```

#### Pre-processing Strategy
Chunks are pre-processed on cache insertion to optimize future queries:

```rust
chunk
    .densified(component)  // Dense layout for this component
    .sorted_by_timeline_if_unsorted(&timeline)  // Pre-sort by timeline
```

**Cost tracking**:
- `resorted: true` → chunk was copied and modified (counts as real memory)
- `resorted: false` → just a reference to store chunk (0 memory cost)

#### Query Execution on Range Cache
1. Forward query to store to find relevant chunks (index scan always happens)
2. Cache chunks with preprocessing
3. Run range filter on cached, pre-processed chunks
4. Return non-empty results

### 2.3 Clearing Optimization (might_require_clearing)

```rust
pub(crate) might_require_clearing: RwLock<IntSet<EntityPath>>
```

**Problem solved**: Every latest-at query must check for `Clear` components up the entity hierarchy. This is expensive overhead.

**Solution**: 
- Track which entities have ever had Clear-related data
- Skip Clear checks for entities not in this set (most entities in typical datasets)
- Massive performance improvement in recordings with many entities

**From cache.rs lines 149-154**:
```
// "This is used to optimized read-time clears, so that we don't unnecessarily 
// pay for the fixed overhead of all the query layers when we know for a fact 
// that there won't be any data there. This is a huge performance improvement 
// in practice, especially in recordings with many entities."
```

---

## 3. Latest-At Query Optimization

### Query Flow

#### Step 1: Pre-filtering
```rust
let components = components.into_iter().filter(|component| {
    store.entity_has_component_on_timeline(&query.timeline(), entity_path, *component)
});
```

**Critical optimization**: Avoid query layer overhead for non-existent components. This alone prevents massive overhead in frame rendering loops.

#### Step 2: Clear Component Checking
```rust
// Walks up entity hierarchy, checking:
// 1. Self clears (any Clear component)
// 2. Recursive parent clears (only if ClearIsRecursive flag is set)
// Uses cache with pending_invalidations for efficient re-checking
```

**Cost**: O(entity_depth) cache lookups, but mostly hits after first query.

#### Step 3: Data Component Queries
Each component is independently cached and queried:
```rust
let cache = self.latest_at_per_cache_key
    .write()
    .entry(key.clone())
    .or_insert_with(|| Arc::new(RwLock::new(LatestAtCache::new(key))))
    .clone();  // Arc clone is cheap

let mut cache = cache.write();
cache.handle_pending_invalidation();  // Apply any deferred invalidations

if let Some(cached) = cache.latest_at(&store, query, entity_path, component) {
    // Process result...
}
```

### Compound Index Handling
```rust
pub compound_index: (TimeInt, RowId)  // Most recent index across ALL components
```

When multiple components are queried together, the result index is the maximum:
```rust
if index > self.compound_index {
    self.compound_index = index;
}
```

This ensures temporal consistency when components are sourced from different rows.

### Optimization: Static Data Bypass
```rust
// From latest_at.rs lines 657-666:
if query.at() != data_time && !data_time.is_static() {
    per_query_time.entry(query.at())
        .or_insert_with(|| LatestAtCachedChunk {
            unit: cached.unit.clone(),
            is_reference: true,
        });
}
```

- Static queries are cached directly (very cheap to recompute)
- Don't pollute cache with query-time entries for static data
- Prevents unbounded cache growth for static data accessed at many different time points

---

## 4. Cost of Cache Misses

### Cache Miss Impacts

#### LatestAtCache Miss Cost:
```
1. Lock acquisition on HashMap and individual cache
2. ChunkStore::latest_at_relevant_chunks() call → index scan across all chunks
3. Filter returned chunks (find max by TimeInt+RowId)
4. Chunk access and extraction of unit chunk  
5. Optional cache insertion (if not static data)
```

**Relative cost**: HIGH - requires store query which scans indices

**From latest_at.rs line 51**: "This is called very frequently, don't put a profile scope here."

The comment indicates cache misses are expensive enough that even profiling overhead would be noticeable.

#### RangeCache Miss Cost:
```
1. Store.range_relevant_chunks() → range query against indices
2. For each returned chunk:
   - Optional densification by component
   - Optional timeline sorting (potentially O(n log n) per chunk)
3. Apply range filter to each chunk
4. Collect non-empty results
```

**Relative cost**: VERY HIGH - includes chunk reorganization

**From range.rs line 270**: `TODO(#7008): avoid unnecessary sorting on the unhappy path`

Indicates developers recognize sorting is expensive; there's a planned optimization.

### Cache Miss Avoidance Strategies

#### 1. **Pre-query Filtering** (lines 58-62 in latest_at.rs)
```rust
// "This pre-filtering is extremely important: going through all these query 
// layers has non-negligible overhead even if the final result ends up being 
// nothing, and our number of queries for a frame grows linearly with the 
// number of entity paths."
```

Prevents calling expensive cache miss logic for components that don't exist.

#### 2. **Negative Caching**
Caching "no result" prevents re-running expensive queries.

#### 3. **Deferred Invalidation**
```rust
// From cache.rs line 533:
// "the frame time effectively behaves as a natural micro-batching mechanism"
```

Batches invalidations together, reducing cache-clearing work.

#### 4. **Might-Require-Clearing Optimization**
Most cache lookups for clears hit the "entity not in might_require_clearing" path - instant return.

### Memory vs Speed Tradeoff

**Frame-time cache growth management**:
```rust
pub fn purge_fraction_of_ram(&self, fraction_to_purge: f32) {
    // Split off older entries by time, keeping only recent data
    cache.per_query_time = cache.per_query_time.split_off(&split_time);
}
```

- Purges oldest cache entries to manage RAM
- Must preserve `pending_invalidations` to avoid future cache corruptions
- (line 180-184): Accepting potential over-invalidation is safer than under-invalidation

---

## 5. Query Performance Benchmarks

### Benchmark Setup (benches/latest_at.rs)

#### Test Scenarios:

**Mono Points**: Each point at separate entity path
- 1,000 entity paths × 1,000 frames = 1M data points
- Tests: insert + query performance

**Batch Points**: All points at one entity path  
- 1 entity path × 1,000 frames × 1,000 points per frame = 1M data points
- Tests: batch vs. mono performance

**String Labels**: Similar structure but with variable-length strings

#### Benchmark Configuration:
```rust
#[cfg(debug_assertions)]
const NUM_FRAMES_POINTS: u32 = 1;     // 1 for quick testing
const NUM_POINTS: u32 = 1;

#[cfg(not(debug_assertions))]
const NUM_FRAMES_POINTS: u32 = 1_000; // Production benchmarks
const NUM_POINTS: u32 = 1_000;
```

Uses Criterion with configurable sample sizes (mono-insert: 10 samples due to slowness).

### Benchmarked Operations:

1. **Insert performance**: `cargo bench -- arrow_mono_points2/insert`
   - Measures chunk storage and cache population
   - Expected: High overhead for mono insertions

2. **Query performance**: `cargo bench -- arrow_mono_points2/query`
   - Measures latestAtCache hits/misses
   - Runs query_and_visit_points: iterates entities, queries latest-at, extracts components
   - Uses clamped_zip for safe component batching

### Query Benchmark Logic:
```rust
let query = LatestAtQuery::new(timeline_frame_nr, NUM_FRAMES_POINTS as i64 / 2);
for entity_path in paths {
    let results = caches.latest_at(&query, entity_path, Points2D::all_component_identifiers());
    let points = results.component_batch_quiet::<Position2D>(...)?;
    let colors = results.component_batch_quiet::<Color>(...)?;
    for (point, color) in clamped_zip_1x1(points, colors, color_default_fn) {
        // Process data
    }
}
```

### Expected Performance Characteristics:

1. **First query for (entity, timeline, component)**: Cache miss - expensive
2. **Repeated queries at same time**: Cache hit - O(log n) BTreeMap lookup
3. **Same query time, different components**: Independent cache entries per component
4. **Mono data (separate paths)**: More cache entries, more overhead, but simpler zip logic
5. **Batch data (one path)**: Fewer cache entries, but more complex per-query processing

---

## Summary of Performance Characteristics

### Strengths:
1. ✓ Lazy caching prevents wasted computation
2. ✓ Deferred invalidation reduces contention
3. ✓ Reference deduplication saves memory
4. ✓ `might_require_clearing` eliminates unnecessary Clear checks (huge win)
5. ✓ Pre-filtering avoids expensive store queries for missing components
6. ✓ Negative caching prevents re-running failed queries

### Weaknesses:
1. ✗ Range queries require chunk pre-processing (sorting/densifying) - expensive miss cost
2. ✗ TODO #7008: Unnecessary sorting on unhappy path (identified but not yet fixed)
3. ✗ TODO #5974: Cache invalidation tracking can grow indefinitely if data never inserted
4. ✗ TODO #403: Static cache population is "horribly stupid and slow" (acknowledged by developers)

### Critical Insight:
The system is optimized for the **common case** (repeated queries at the same times with clearing), where most operations hit cache and are O(1) to O(log n). Cache misses are expensive but rare in practice due to frame-based temporal locality.

