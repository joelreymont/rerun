# Lazy Timeline Indexing Analysis
## Task 1.3: Investigation Report

### Problem Statement

**Current Behavior:** When a static transform chunk is added via `add_static_chunk()` at `transform_resolution_cache.rs:1009`, the code eagerly iterates over ALL timelines in `self.per_timeline` (line 1034-1050) to invalidate transforms.

**Performance Impact:**
- For recordings with N timelines, every static chunk addition touches all N timeline caches
- Air traffic 2h dataset: Multiple timelines but users typically view 1-2
- Wasted CPU time: Processing timelines that are never queried

**Code Location:**
```
crates/store/re_tf/src/transform_resolution_cache.rs:1034
```

### Current Implementation Analysis

#### Key Data Structures

```rust
pub struct TransformResolutionCache {
    frame_id_registry: FrameIdRegistry,
    per_timeline: HashMap<TimelineName, CachedTransformsForTimeline>,  // ← Eagerly populated
    static_timeline: CachedTransformsForTimeline,
}
```

#### Eager Behavior in add_static_chunk()

**Lines 1034-1050:**
```rust
for per_timeline_transforms in &mut self.per_timeline.values_mut() {
    // Propagate the new static child frames to all timelines
    per_timeline_transforms
        .per_entity_affected_child_frames
        .get_or_create_for(entity_path)
        .insert_range_start(TimeInt::STATIC, child_frames.clone());

    // Invalidate the static status on previous child frames
    for previous_child_frame in &previous_frames {
        if let Some(frame_transform) = per_timeline_transforms
            .per_child_frame_transforms
            .get_mut(previous_child_frame)
        {
            frame_transform.events.get_mut().remove_at(TimeInt::STATIC);
        }
    }
}
```

**Lines 1075-1096:**
```rust
for (timeline, per_timeline_transforms) in &mut self.per_timeline {
    let entity_transforms = per_timeline_transforms
        .per_child_frame_transforms
        .entry(child_frame)
        .or_insert_with(|| {
            TransformsForChildFrame::new(
                child_frame,
                *timeline,
                &mut self.static_timeline,
            )
        });

    entity_transforms.insert_invalidated_transform_events(
        aspects,
        TimeInt::STATIC,
        || std::iter::once(TimeInt::STATIC),
        entity_path,
    );
}
```

#### Access Patterns

**Timeline creation (lazy - already optimized):**
```rust
// Line 841 - Only creates timeline entry when accessed
let per_timeline = self.per_timeline.entry(*timeline).or_insert_with(|| {
    CachedTransformsForTimeline::new_empty_with_static_transforms(&self.static_timeline)
});
```

**Timeline query (good - uses specific timeline):**
```rust
// Line 1108 - Gets specific timeline for query
let Some(per_timeline) = self.per_timeline.get_mut(timeline) else {
    // ...
};
```

### Optimization Challenges

#### Challenge 1: Transform Invalidation Correctness

Static transforms affect ALL timelines because:
1. Static data is "always present" across time
2. Adding/changing static transform invalidates all temporal queries
3. The system needs to re-evaluate tree transforms when static data changes

**Risk:** Making this lazy could break transform resolution if invalidation is missed.

#### Challenge 2: Two Separate Loops

The eager behavior happens in TWO places in `add_static_chunk()`:
1. Lines 1034-1050: Propagate child frames + invalidate previous frames
2. Lines 1075-1096: Insert invalidated events for all child frames

Both would need to be made lazy while maintaining correctness.

#### Challenge 3: Incremental Invalidation

Current approach: Invalidate everything immediately when static data changes.

Lazy approach would need:
- Track which timelines have "pending" static invalidations
- Apply invalidations on first access to each timeline
- Ensure no stale data is returned

### Proposed Lazy Implementation Strategy

#### Option A: Deferred Invalidation (Recommended)

**Concept:** Track pending static changes, apply to timeline on first query.

```rust
pub struct TransformResolutionCache {
    frame_id_registry: FrameIdRegistry,
    per_timeline: HashMap<TimelineName, CachedTransformsForTimeline>,
    static_timeline: CachedTransformsForTimeline,

    // NEW: Track pending static changes
    pending_static_invalidations: Vec<PendingStaticInvalidation>,
}

struct PendingStaticInvalidation {
    child_frames: SmallVec1<TransformFrameIdHash>,
    previous_frames: SmallVec1<TransformFrameIdHash>,
    aspects: TransformAspect,
    entity_path: EntityPath,
}

impl TransformResolutionCache {
    fn add_static_chunk(&mut self, chunk: &Chunk, aspects: TransformAspect) {
        re_tracing::profile_function!();

        // ... existing static_timeline updates ...

        // INSTEAD of iterating all timelines, just record the invalidation
        self.pending_static_invalidations.push(PendingStaticInvalidation {
            child_frames,
            previous_frames,
            aspects,
            entity_path: entity_path.clone(),
        });
    }

    fn ensure_timeline_updated(&mut self, timeline: &TimelineName) {
        // Get or create timeline
        let per_timeline = self.per_timeline.entry(*timeline).or_insert_with(|| {
            CachedTransformsForTimeline::new_empty_with_static_transforms(&self.static_timeline)
        });

        // Apply any pending static invalidations
        if !self.pending_static_invalidations.is_empty() {
            for invalidation in &self.pending_static_invalidations {
                // Apply the invalidation logic that was previously in add_static_chunk()
                per_timeline.apply_static_invalidation(invalidation);
            }
            // Only clear if this was the last timeline needing updates
            // (or use a per-timeline tracking mechanism)
        }
    }

    // Call ensure_timeline_updated() before any query
    pub fn query_and_resolve(..., timeline: &TimelineName, ...) {
        self.ensure_timeline_updated(timeline);
        // ... rest of query logic ...
    }
}
```

**Pros:**
- Maintains correctness - invalidations are applied before queries
- Reduces work for unused timelines
- Clear separation of concerns

**Cons:**
- Adds complexity - need to track pending invalidations
- Memory overhead for pending invalidation queue
- Need to ensure ensure_timeline_updated() is called before ALL query paths

#### Option B: Lazy Timeline Creation Only (Safer, Limited Impact)

**Concept:** Keep invalidation eager but make timeline CREATION lazy.

Currently line 841 already does this! The real issue is lines 1034 and 1075 which iterate over existing timelines.

**Simpler approach:**
```rust
fn add_static_chunk(&mut self, chunk: &Chunk, aspects: TransformAspect) {
    // ... existing static_timeline updates ...

    // CHANGE: Only iterate over EXISTING timelines
    // NEW timelines will pick up static data via new_empty_with_static_transforms()

    for per_timeline_transforms in &mut self.per_timeline.values_mut() {
        // ... existing invalidation logic ...
    }
}
```

**Analysis:** This doesn't help! If timelines have already been created (which they have for active views), we still iterate all of them.

The win would only come if we can avoid creating timeline entries for timelines that are never viewed.

### Recommendation

**Status: DEFER - Requires Deeper Analysis**

**Reasoning:**
1. **Correctness Risk:** Transform invalidation across timelines is subtle. Making it lazy could introduce hard-to-debug incorrect transform results.

2. **Complexity:** Two separate invalidation loops need to be refactored, plus all query paths need to call ensure_timeline_updated().

3. **Expected Impact:** 1-3ms improvement is modest compared to annotation loading fix (5-15ms).

4. **Testing Challenge:** Need to verify correctness across:
   - Multiple timelines
   - Static + temporal transforms
   - Transform tree invalidation
   - Various entity hierarchies

**Suggested Next Steps:**
1. Add instrumentation to measure current timeline iteration cost
2. Verify the 1-3ms estimate with real profiling data
3. If confirmed, implement Option A with comprehensive testing
4. Consider alternative optimizations with better risk/reward ratio

### Alternative: Instrumentation First

Before implementing lazy indexing, add metrics to validate the optimization value:

```rust
// In add_static_chunk()
for (timeline, per_timeline_transforms) in &mut self.per_timeline {
    performance_metrics::TIMELINE_INVALIDATIONS_THIS_FRAME
        .fetch_add(1, Ordering::Relaxed);
    // ... existing logic ...
}

// In performance panel
if bottleneck_metrics.timeline_invalidations_per_frame > active_timeline_count {
    ui.label(format!(
        "⚠ Invalidating {} timelines but only {} active",
        timeline_invalidations, active_timelines
    ));
}
```

This would confirm whether lazy timeline indexing is worth the complexity.

### Conclusion

Lazy timeline indexing is a valid optimization but requires:
- Careful correctness analysis
- Comprehensive testing infrastructure
- Profiling to confirm expected benefits

**Recommendation:** Implement instrumentation first to validate optimization value, then proceed with Option A (Deferred Invalidation) if confirmed worthwhile.

**Current Priority:** Focus on safer, high-impact optimizations first (annotation caching ✅). Return to timeline indexing after measuring real-world impact.
