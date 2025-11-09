# Rerun Query Cache - Detailed Code Reference

## File Locations & Key Implementations

### Primary Cache Implementation Files

```
/home/user/rerun/crates/store/re_query/src/
├── cache.rs              (454 lines) - QueryCache coordinator
├── latest_at.rs          (704 lines) - LatestAt cache and query logic
├── range.rs              (317 lines) - Range cache and query logic  
├── cache_stats.rs        (109 lines) - Cache statistics tracking
├── storage_engine.rs     - StorageEngine wrapper
├── clamped_zip/mod.rs    - Component batching utilities
└── lib.rs                - Public API exports
```

### Benchmark Files

```
/home/user/rerun/crates/store/re_query/
├── benches/latest_at.rs  (328 lines) - Query performance benchmarks
├── examples/query_latest_at.rs
└── examples/query_range.rs
```

---

## Critical Code Sections

### 1. QueryCache Structure and Initialization

**File**: `/home/user/rerun/crates/store/re_query/src/cache.rs` (lines 142-262)

```rust
pub struct QueryCache {
    /// Handle to the associated [`ChunkStoreHandle`].
    pub(crate) store: ChunkStoreHandle,

    /// The [`StoreId`] of the associated [`ChunkStoreHandle`].
    pub(crate) store_id: StoreId,

    /// Keeps track of which entities have had any `Clear`-related data
    /// This is used to optimized read-time clears, so that we don't 
    /// unnecessarily pay for the fixed overhead of all the query layers 
    /// when we know for a fact that there won't be any data there.
    /// This is a huge performance improvement in practice, especially in 
    /// recordings with many entities.
    pub(crate) might_require_clearing: RwLock<IntSet<EntityPath>>,

    // NOTE: `Arc` so we can cheaply free the top-level lock early when needed.
    pub(crate) latest_at_per_cache_key: 
        RwLock<HashMap<QueryCacheKey, Arc<RwLock<LatestAtCache>>>>,

    // NOTE: `Arc` so we can cheaply free the top-level lock early when needed.
    pub(crate) range_per_cache_key: 
        RwLock<HashMap<QueryCacheKey, Arc<RwLock<RangeCache>>>>,
}
```

**Key insight**: Arc wrapping allows release of top-level lock immediately after getting reference, enabling concurrent queries.

### 2. LatestAt Cache - Main Query API

**File**: `/home/user/rerun/crates/store/re_query/src/latest_at.rs` (lines 40-166)

```rust
pub fn latest_at(
    &self,
    query: &LatestAtQuery,
    entity_path: &EntityPath,
    components: impl IntoIterator<Item = ComponentIdentifier>,
) -> LatestAtResults {
    // This is called very frequently, don't put a profile scope here.
    let store = self.store.read();

    let mut results = LatestAtResults::empty(entity_path.clone(), query.clone());

    // NOTE: This pre-filtering is extremely important: going through all 
    // these query layers has non-negligible overhead even if the final 
    // result ends up being nothing, and our number of queries for a frame 
    // grows linearly with the number of entity paths.
    let components = components.into_iter().filter(|component| {
        store.entity_has_component_on_timeline(
            &query.timeline(), 
            entity_path, 
            *component
        )
    });

    // Query-time clears - check for Clear components up entity hierarchy
    let mut max_clear_index = (TimeInt::MIN, RowId::ZERO);
    {
        let potential_clears = self.might_require_clearing.read();
        // Walk up entity hierarchy checking for Clear components...
    }

    // Query each component independently
    for component in components {
        let key = QueryCacheKey::new(entity_path.clone(), query.timeline(), component);
        
        let cache = Arc::clone(
            self.latest_at_per_cache_key
                .write()
                .entry(key.clone())
                .or_insert_with(|| Arc::new(RwLock::new(LatestAtCache::new(key)))),
        );

        let mut cache = cache.write();
        cache.handle_pending_invalidation();
        if let Some(cached) = cache.latest_at(&store, query, entity_path, component) {
            if let Some(index) = cached.index(&query.timeline())
                && (component == archetypes::Clear::descriptor_is_recursive().component
                    || compare_indices(index, max_clear_index) == std::cmp::Ordering::Greater)
            {
                results.add(component, index, cached);
            }
        }
    }

    results
}
```

**Key optimizations**:
1. Pre-filtering avoids expensive cache miss on non-existent components
2. Per-component independent caches allow fine-grained invalidation
3. Compound index ensures temporal consistency across components

### 3. LatestAt Cache Implementation

**File**: `/home/user/rerun/crates/store/re_query/src/latest_at.rs` (lines 515-703)

```rust
/// Caches the results of `LatestAt` queries for a given [`QueryCacheKey`].
pub struct LatestAtCache {
    pub cache_key: QueryCacheKey,
    
    /// Organized by _query_ time.
    /// If the key is present but has a `None` value associated with it, 
    /// it means we cached the lack of result.
    /// This is important to do performance-wise: we run _a lot_ of queries 
    /// each frame to figure out what to render, and this scales linearly 
    /// with the number of entity.
    pub per_query_time: BTreeMap<TimeInt, LatestAtCachedChunk>,

    /// These timestamps have been invalidated asynchronously.
    /// The next time this cache gets queried, it must remove any 
    /// invalidated entries accordingly.
    /// Invalidation is deferred to query time because it is far more 
    /// efficient that way: the frame time effectively behaves as a 
    /// natural micro-batching mechanism.
    pub pending_invalidations: BTreeSet<TimeInt>,
}

impl LatestAtCache {
    /// Queries cached latest-at data for a single component.
    pub fn latest_at(
        &mut self,
        store: &ChunkStore,
        query: &LatestAtQuery,
        entity_path: &EntityPath,
        component: ComponentIdentifier,
    ) -> Option<UnitChunkShared> {
        // Don't do a profile scope here, this can have a lot of overhead 
        // when executing many small queries.

        let Self { cache_key: _, per_query_time, pending_invalidations: _ } = self;

        // CACHE HIT PATH (O(log n)):
        if let Some(cached) = per_query_time.get(&query.at()) {
            return Some(cached.unit.clone());
        }

        // CACHE MISS PATH:
        // Query store for relevant chunks and find maximum by index
        let ((data_time, _row_id), unit) = store
            .latest_at_relevant_chunks(query, entity_path, component)
            .into_iter()
            .filter_map(|chunk| {
                let chunk = chunk.latest_at(query, component).into_unit()?;
                chunk.index(&query.timeline()).map(|index| (index, chunk))
            })
            .max_by_key(|(index, _chunk)| *index)?;

        // Cache result by actual data time (not query time)
        let cached = per_query_time
            .entry(data_time)
            .or_insert_with(|| LatestAtCachedChunk {
                unit,
                is_reference: false,
            })
            .clone();

        // OPTIMIZATION: Don't cache query-time entries for static data
        // "Queries that return static data are much cheaper to run, and 
        // polluting the query-time cache just to point to the static 
        // tables again and again is very wasteful."
        if query.at() != data_time && !data_time.is_static() {
            per_query_time
                .entry(query.at())
                .or_insert_with(|| LatestAtCachedChunk {
                    unit: cached.unit.clone(),
                    is_reference: true,  // Reference deduplication
                });
        }

        Some(cached.unit)
    }

    /// Handle any pending invalidations from store updates
    pub fn handle_pending_invalidation(&mut self) {
        let Self {
            cache_key: _,
            per_query_time,
            pending_invalidations,
        } = self;

        if let Some(oldest_data_time) = pending_invalidations.first() {
            // Remove any data indexed by a _query time_ that's more recent 
            // than the oldest _data time_ that's been invalidated.
            let discarded = per_query_time.split_off(oldest_data_time);

            // TODO(#5974): Because of non-deterministic ordering, parallelism, 
            // and most importantly lack of centralized query layer, it can 
            // happen that we try to handle pending invalidations before we 
            // even cached the associated data.
            //
            // If that happens, the data will be cached after we've 
            // invalidated *nothing*, and will stay there indefinitely since 
            // the cache doesn't have a dedicated GC yet.
            //
            // TL;DR: make sure to keep track of pending invalidations 
            // indefinitely as long as we haven't had the opportunity to 
            // actually invalidate the associated data.
            pending_invalidations.retain(|data_time| {
                let is_reference = discarded
                    .get(data_time)
                    .is_none_or(|chunk| chunk.is_reference);
                !is_reference
            });
        }
    }
}
```

**Critical insights**:
- Cache misses scan all chunks, O(n) operation
- Static data bypass prevents unbounded cache growth
- Reference deduplication saves memory on repeated static data accesses
- Pending invalidations preserved until actually applied

### 4. Range Cache Implementation

**File**: `/home/user/rerun/crates/store/re_query/src/range.rs` (lines 125-316)

```rust
pub struct RangeCache {
    pub cache_key: QueryCacheKey,
    
    /// All the [`Chunk`]s currently cached.
    pub chunks: HashMap<ChunkId, RangeCachedChunk>,

    /// Every [`ChunkId`] present in this set has been asynchronously 
    /// invalidated. The next time this cache gets queried, it must remove 
    /// any entry matching any of these IDs.
    ///
    /// Invalidation is deferred to query time because it is far more 
    /// efficient that way: the frame time effectively behaves as a 
    /// natural micro-batching mechanism.
    pub pending_invalidations: BTreeSet<ChunkId>,
}

pub struct RangeCachedChunk {
    pub chunk: Chunk,

    /// When a `Chunk` gets cached, it is pre-processed according to the 
    /// current [`QueryCacheKey`], e.g. it is time-sorted on the 
    /// appropriate timeline.
    ///
    /// In the happy case, pre-processing a `Chunk` is a no-op, and the 
    /// cached `Chunk` is just a reference to the real one sitting in 
    /// the store.
    /// Otherwise, the cached `Chunk` is a full blown copy of the 
    /// original one.
    pub resorted: bool,
}

impl RangeCache {
    pub fn range(
        &mut self,
        store: &ChunkStore,
        query: &RangeQuery,
        entity_path: &EntityPath,
        component: ComponentIdentifier,
    ) -> Vec<Chunk> {
        // First, forward the query as-is to the store.
        // It's fine to run the query every time -- the index scan itself 
        // is not the costly part of a range query.
        //
        // For all relevant chunks that we find, we process them according 
        // to the [`QueryCacheKey`], and cache them.

        let raw_chunks = store.range_relevant_chunks(query, entity_path, component);
        for raw_chunk in &raw_chunks {
            self.chunks
                .entry(raw_chunk.id())
                .or_insert_with(|| RangeCachedChunk {
                    // TODO(#7008): avoid unnecessary sorting on the unhappy path
                    chunk: raw_chunk
                        // Densify the cached chunk according to the cache 
                        // key's component, which will speed up future arrow 
                        // operations on this chunk.
                        .densified(component)
                        // Pre-sort the cached chunk according to the cache 
                        // key's timeline.
                        .sorted_by_timeline_if_unsorted(&self.cache_key.timeline_name),
                    resorted: !raw_chunk.is_timeline_sorted(&self.cache_key.timeline_name),
                });
        }

        // Second, we simply retrieve from the cache all the relevant 
        // `Chunk`s. Since these `Chunk`s have already been pre-processed 
        // adequately, running a range filter on them will be quite cheap.

        raw_chunks
            .into_iter()
            .filter_map(|raw_chunk| self.chunks.get(&raw_chunk.id()))
            .map(|cached_sorted_chunk| {
                cached_sorted_chunk.chunk.range(query, component)
            })
            .filter(|chunk| !chunk.is_empty())
            .collect()
    }

    pub fn handle_pending_invalidation(&mut self) {
        let Self {
            cache_key: _,
            chunks,
            pending_invalidations,
        } = self;

        chunks.retain(|chunk_id, _chunk| !pending_invalidations.contains(chunk_id));
        pending_invalidations.clear();
    }
}
```

**Key observations**:
- Always scans store indices (not cached)
- Chunks are pre-processed (densified + sorted) on insertion
- `resorted` flag tracks memory cost
- TODO #7008 identifies sorting bottleneck

### 5. Invalidation Handling - Event Processing

**File**: `/home/user/rerun/crates/store/re_query/src/cache.rs` (lines 281-452)

```rust
fn on_events(&mut self, events: &[ChunkStoreEvent]) {
    re_tracing::profile_function!(format!("num_events={}", events.len()));

    #[derive(Default, Debug)]
    struct CompactedEvents {
        static_: HashMap<(EntityPath, ComponentIdentifier), BTreeSet<ChunkId>>,
        temporal_latest_at: HashMap<QueryCacheKey, TimeInt>,
        temporal_range: HashMap<QueryCacheKey, BTreeSet<ChunkId>>,
    }

    let mut compacted_events = CompactedEvents::default();

    // COMPACT EVENTS INTO GROUPS
    for event in events {
        // ... process events ...
        if chunk.is_static() {
            // Track static component invalidations
        }
        for (timeline, per_component) in chunk.time_range_per_component() {
            for (component_identifier, time_range) in per_component {
                let key = QueryCacheKey::new(...);
                
                // Latest-at invalidation: track minimum time affected
                let mut data_time_min = time_range.min();
                // ... handle compacted chunks ...
                compacted_events
                    .temporal_latest_at
                    .entry(key.clone())
                    .and_modify(|time| *time = TimeInt::min(*time, data_time_min))
                    .or_insert(data_time_min);

                // Range invalidation: track specific chunk IDs
                compacted_events
                    .temporal_range
                    .entry(key)
                    .or_default()
                    .insert(chunk.id());
            }
        }
    }

    // DEFERRED INVALIDATION APPLICATION
    let mut might_require_clearing = self.might_require_clearing.write();
    let caches_latest_at = self.latest_at_per_cache_key.write();
    let caches_range = self.range_per_cache_key.write();
    // NOTE: Don't release the top-level locks -- even though this cannot 
    // happen yet with our current macro-architecture, we want to prevent 
    // queries from concurrently running while we're updating the 
    // invalidation flags.

    // Static invalidations
    {
        re_tracing::profile_scope!("static");

        // TODO(cmc): This is horribly stupid and slow and can easily be 
        // made faster by adding yet another layer of caching indirection.
        // But since this pretty much never happens in practice, let's not 
        // go there until we have metrics showing that show we need to.
        for ((entity_path, component_identifier), chunk_ids) in compacted_events.static_ {
            if component_identifier == archetypes::Clear::descriptor_is_recursive().component {
                might_require_clearing.insert(entity_path.clone());
            }

            for (key, cache) in caches_latest_at.iter() {
                if key.entity_path == entity_path && key.component == component_identifier {
                    cache.write().pending_invalidations.insert(TimeInt::STATIC);
                }
            }

            for (key, cache) in caches_range.iter() {
                if key.entity_path == entity_path && key.component == component_identifier {
                    cache.write().pending_invalidations
                        .extend(chunk_ids.iter().copied());
                }
            }
        }
    }

    // Temporal invalidations
    {
        re_tracing::profile_scope!("temporal");

        for (key, time) in compacted_events.temporal_latest_at {
            if key.component == archetypes::Clear::descriptor_is_recursive().component {
                might_require_clearing.insert(key.entity_path.clone());
            }

            if let Some(cache) = caches_latest_at.get(&key) {
                cache.write().pending_invalidations.insert(time);
            }
        }

        for (key, chunk_ids) in compacted_events.temporal_range {
            if let Some(cache) = caches_range.get(&key) {
                cache.write().pending_invalidations
                    .extend(chunk_ids.iter().copied());
            }
        }
    }
}
```

**Critical insights**:
- Events are compacted to avoid duplicate invalidations
- Invalidations are deferred (pending_invalidations), not applied immediately
- Frame rendering naturally batches these deferred invalidations
- Static case is acknowledged as "horribly stupid and slow"

### 6. Cache Statistics

**File**: `/home/user/rerun/crates/store/re_query/src/cache_stats.rs` (lines 52-108)

```rust
#[derive(Default, Debug, Clone)]
pub struct QueryCachesStats {
    pub latest_at: BTreeMap<QueryCacheKey, QueryCacheStats>,
    pub range: BTreeMap<QueryCacheKey, QueryCacheStats>,
}

#[derive(Default, Debug, Clone)]
pub struct QueryCacheStats {
    /// How many chunks in the cache?
    pub total_chunks: u64,

    /// What would be the size of this cache in the worst case, i.e. if all 
    /// chunks had been fully copied?
    pub total_effective_size_bytes: u64,

    /// What is the actual size of this cache after deduplication?
    pub total_actual_size_bytes: u64,
}

impl QueryCache {
    /// Computes the stats for all primary caches.
    pub fn stats(&self) -> QueryCachesStats {
        re_tracing::profile_function!();
        
        // Gather stats on latest_at caches
        // Implicitly releasing top-level cache mappings -- concurrent 
        // queries can run once again
        // ...
    }
}
```

**Key metric**: Tracks both "effective" (if copied) and "actual" (with deduplication) memory costs.

### 7. Benchmark - Query & Visit Pattern

**File**: `/home/user/rerun/crates/store/re_query/benches/latest_at.rs` (lines 264-327)

```rust
fn query_and_visit_points(caches: &QueryCache, paths: &[EntityPath]) -> Vec<SavePoint> {
    let timeline_frame_nr = TimelineName::new("frame_nr");
    let query = LatestAtQuery::new(timeline_frame_nr, NUM_FRAMES_POINTS as i64 / 2);

    let mut ret = Vec::with_capacity(NUM_POINTS as _);

    for entity_path in paths {
        // Query latest-at data
        let results: LatestAtResults = 
            caches.latest_at(&query, entity_path, Points2D::all_component_identifiers());

        // Deserialize components
        let points = results
            .component_batch_quiet::<Position2D>(Points2D::descriptor_positions().component)
            .unwrap();
        let colors = results
            .component_batch_quiet::<Color>(Points2D::descriptor_colors().component)
            .unwrap_or_default();

        // Zip components safely with default values
        let color_default_fn = || Color::from(0xFF00FFFF);
        for (point, color) in clamped_zip_1x1(points, colors, color_default_fn) {
            ret.push(SavePoint {
                _pos: point,
                _color: Some(color),
            });
        }
    }
    assert_eq!(NUM_POINTS as usize, ret.len());
    ret
}
```

**Benchmark scenarios**:
- Mono: 1,000 entities × 1,000 frames
- Batch: 1 entity × 1,000 frames × 1,000 points per frame
- Both test insert and query performance

---

## Key Performance Characteristics Summary

### Time Complexity

| Operation | Time | Notes |
|-----------|------|-------|
| Cache Hit (latest-at) | O(log n) | BTreeMap lookup |
| Cache Hit (range) | O(1) | HashMap lookup |
| Cache Miss (latest-at) | O(m) | Scan m relevant chunks |
| Cache Miss (range) | O(m) | Scan m chunks + potential sort |
| Clear check | O(d * log n) | d = entity depth, cached |
| Pre-filtering | O(k) | k = num components |

### Memory Characteristics

| Strategy | Impact |
|----------|--------|
| Reference deduplication | Amortizes static data to 0 bytes |
| Resorted tracking | Only counts actual copied chunks |
| Pre-processing | One-time densify/sort cost |
| Pending invalidations | Could grow unbounded (TODO #5974) |

### Identified Optimizations & TODOs

1. **TODO #7008** (range.rs:270): Avoid unnecessary sorting on unhappy path
2. **TODO #5974** (latest_at.rs:686): Pending invalidation tracking can grow indefinitely
3. **TODO #403** (cache.rs:403): Static event handling is "horribly stupid and slow"
4. **TODO cmc** (latest_at.rs:24): DataIndex type could improve index handling

