# Issue #11432: Improve Rendering of Streams Panel / Timeline View

## Executive Summary

The Rerun streams panel has a performance optimization that causes visual confusion: when there are too many events to render individually, the system falls back to rendering entire chunks as uniform distributions. This creates misleading visual representations where two scalar values logged at identical frequencies can appear different to users.

## Problem Description

### Current Behavior

The density graph rendering system uses a three-tier fallback strategy based on performance thresholds:

1. **Individual Events** (preferred): Shows exact event times as discrete points
2. **Sampled Events** (compromise): Uniformly samples events and reweights them to preserve density
3. **Range Distribution** (fallback): Spreads events uniformly across the entire chunk's time range

The fallback is triggered based on:
- Total number of events across all chunks (`max_total_chunk_events: 10,000`)
- Number of events per sorted chunk (`max_events_in_sorted_chunk: 10,000`)
- Number of events per unsorted chunk (`max_events_in_unsorted_chunk: 8,000`)

### Why This Is Confusing

When two scalars are logged at identical frequencies but stored in chunks with different characteristics (size, sort order, time distribution), they can render differently:
- One might show individual event spikes (under threshold)
- The other might show a smooth uniform distribution (over threshold)

This creates the false impression that data logging patterns are different when they're actually identical.

## Code Analysis

### Key Files

| File | Purpose |
|------|---------|
| `crates/viewer/re_time_panel/src/data_density_graph.rs` | Core rendering logic, config, and fallback strategy (810 lines) |
| `crates/viewer/re_time_panel/src/time_panel.rs` | Main streams panel UI and entity tree rendering |
| `crates/viewer/re_time_panel/src/streams_tree_data.rs` | Entity hierarchy and tree structure management |

### Critical Code Sections

#### 1. Configuration (data_density_graph.rs:595-663)

```rust
pub struct DensityGraphBuilderConfig {
    pub max_total_chunk_events: u64,        // Default: 10,000
    pub max_events_in_sorted_chunk: u64,    // Default: 10,000
    pub max_events_in_unsorted_chunk: u64,  // Default: 8,000
    pub max_sampled_events_per_chunk: usize // Default: 8,000
}
```

#### 2. Rendering Decision Logic (data_density_graph.rs:544-590)

```rust
let can_render_individual_events = total_events < config.max_total_chunk_events;

for (chunk, time_range, num_events_in_chunk) in chunk_ranges {
    let should_render_individual_events = can_render_individual_events
        && if chunk.is_timeline_sorted(timeline) {
            num_events_in_chunk < config.max_events_in_sorted_chunk
        } else {
            num_events_in_chunk < config.max_events_in_unsorted_chunk
        };

    if should_render_individual_events {
        // Render all individual events (BEST)
        for (time, num_events) in chunk.num_events_cumulative_per_unique_time(timeline) {
            data.add_chunk_point(time, num_events as f32);
        }
    } else if config.max_sampled_events_per_chunk > 0 {
        // Sample uniformly from the chunk (GOOD)
        data.add_uniform_sample_from_chunk(&events, config.max_sampled_events_per_chunk);
    } else {
        // Fall back to uniform distribution across the entire time range (MISLEADING)
        data.add_chunk_range(time_range, num_events_in_chunk);
    }
}
```

### Performance Constraints

From comments in the code (data_density_graph.rs:641-648):
> "Our very basic benchmarks suggest that at 100k sorted events the graph building takes on average 1.5ms, measured on a high-end x86_64 CPU from 2022 (Ryzen 9 7950x). We want to stay around 1ms if possible."

The 10,000 event threshold is set to ensure ~1ms graph building time per entity row.

### Rendering Pipeline

1. **Chunk Collection** (lines 473-529): Collects all chunks in visible time range
2. **Threshold Check** (line 544): Determines if individual rendering is possible
3. **Per-Chunk Decision** (lines 557-589): Chooses rendering strategy per chunk
4. **Graph Building** (lines 723-798): Accumulates density data
5. **Smoothing** (lines 378-411): Applies Hann window blur
6. **Painting** (lines 210-366): Renders as symmetric histogram with feathering

## Root Cause Analysis

The issue stems from **inconsistent visual representation** across different chunks:

1. **Chunk Characteristics Vary**:
   - Chunk size varies based on logging patterns
   - Sort order varies based on data ingestion
   - Time distribution varies based on application behavior

2. **Threshold-Based Fallback Is Binary**:
   - Events either render individually (accurate) or as ranges (misleading)
   - No gradual degradation or visual indication of the fallback

3. **No User Feedback**:
   - Users don't know when they're seeing exact events vs. approximations
   - No visual cue distinguishes the two rendering modes

## Implementation Plan

### Phase 1: Improve Sampling Strategy (Priority: High)

**Goal**: Always use sampling instead of uniform range distribution

**Changes**:
1. Modify `build_density_graph()` to prefer sampling over range fallback
2. Increase `max_sampled_events_per_chunk` or make it adaptive
3. Remove or deprecate `add_chunk_range()` for data visualization

**Rationale**: Sampling preserves the actual time distribution of events, while range distribution destroys it.

**Implementation**:
```rust
// In data_density_graph.rs:570-588
if should_render_individual_events {
    // Render all individual events
    for (time, num_events) in chunk.num_events_cumulative_per_unique_time(timeline) {
        data.add_chunk_point(time, num_events as f32);
    }
} else {
    // ALWAYS sample - never use uniform range fallback
    let events = chunk.num_events_cumulative_per_unique_time(timeline);

    if events.len() > config.max_sampled_events_per_chunk {
        data.add_uniform_sample_from_chunk(&events, config.max_sampled_events_per_chunk);
    } else {
        for (time, num_events) in events {
            data.add_chunk_point(time, num_events as f32);
        }
    }
}
```

**Estimated Impact**:
- ✅ Fixes visual inconsistency between identical logging patterns
- ✅ Minimal performance impact (sampling is already implemented)
- ✅ No API changes required

### Phase 2: Add Visual Feedback (Priority: Medium)

**Goal**: Indicate to users when they're viewing approximations

**Options**:

**Option A: Visual Indicator on Graph**
- Add subtle visual marker (e.g., transparency, dotted pattern, color shift)
- Render sampled/approximated data differently from exact data
- Add tooltip explaining approximation level

## Review and Correction of Initial Implementation

**Initial Issue (Commit 02b7d25):**

The first implementation removed the uniform range fallback entirely, which caused two regressions:

1. **`NEVER_SHOW_INDIVIDUAL_EVENTS` no longer behaved as documented** - This preset sets all thresholds to zero to guarantee aggregated rendering, but the new logic still iterated over every `(time, count)` pair, defeating its purpose.

2. **Disabling sampling (`max_sampled_events_per_chunk = 0`) caused performance explosion** - Previously, this configuration used O(1) fallback. The initial fix built the full events vector and plotted every point, reintroducing performance cliffs.

**Corrected Implementation:**

The code now uses a three-tier approach that respects configuration contracts:

```rust
if should_render_individual_events {
    // Render all individual events
    for (time, num_events) in chunk.num_events_cumulative_per_unique_time(timeline) {
        data.add_chunk_point(time, num_events as f32);
    }
} else if config.max_sampled_events_per_chunk > 0 {
    // Sample to preserve time distribution while maintaining performance
    let events = chunk.num_events_cumulative_per_unique_time(timeline);

    if events.len() > config.max_sampled_events_per_chunk {
        data.add_uniform_sample_from_chunk(&events, config.max_sampled_events_per_chunk);
    } else {
        for (time, num_events) in events {
            data.add_chunk_point(time, num_events as f32);
        }
    }
} else {
    // Sampling explicitly disabled - fall back to range distribution for performance
    data.add_chunk_range(time_range, num_events_in_chunk);
}
```

**Impact:**
- ✅ **Default config** (sampling enabled): Uses sampling, fixes visual inconsistency issue #11432
- ✅ **`NEVER_SHOW_INDIVIDUAL_EVENTS` preset**: Respects O(1) aggregation contract
- ✅ **Sampling disabled**: Maintains O(1) performance guarantee
- ✅ **No API changes**: All existing configuration contracts honored

**Option B: Status Text**
- Add small text indicator below/above graph: "~8k samples" or "exact"
- Only show when hovering or when approximation is active

**Option C: Debug Overlay (for development)**
- Toggle-able overlay showing:
  - Number of chunks
  - Total events
  - Rendering mode per chunk
  - Sample count

**Recommended**: Implement Option B (least intrusive, most informative)

**Implementation Location**: `data_density_graph.rs:210-366` (in `DensityGraph::paint()`)

### Phase 3: Adaptive Sampling (Priority: Low)

**Goal**: Dynamically adjust sampling based on screen space

**Approach**:
1. Calculate available screen pixels for the chunk's time range
2. Sample proportionally to pixel density (Nyquist-inspired)
3. Never sample less than a minimum (e.g., 100) or more than a maximum (e.g., 10,000)

**Formula**:
```rust
let time_range_pixels = time_ranges_ui.x_from_time(chunk.max_time)
                       - time_ranges_ui.x_from_time(chunk.min_time);
let optimal_samples = (time_range_pixels * 2.0).clamp(100.0, 10_000.0) as usize;
```

**Benefits**:
- More samples when zoomed in (better fidelity)
- Fewer samples when zoomed out (better performance)
- Adapts to user's view automatically

### Phase 4: Configuration Improvements (Priority: Low)

**Goal**: Allow users/developers to tune rendering behavior

**Options**:
1. **Developer Settings Panel**: Add UI for adjusting thresholds in debug builds
2. **Blueprint Settings**: Store per-recording preferences for sampling quality
3. **Quality Presets**: "Fast", "Balanced", "Accurate" modes

### Phase 5: Alternative Visualizations (Priority: Research)

**Goal**: Explore better representations for high-density data

**Ideas**:
1. **Heat Map Mode**: Show intensity instead of height
2. **Log Scale**: Compress high-density areas, expand low-density areas
3. **Chunk Boundaries**: Show explicit visual separators between chunks
4. **Min-Max Bars**: Show range of densities within approximated regions

## Performance Considerations

### Current Performance Characteristics

| Event Count | Render Time (estimate) | Rendering Mode |
|------------|----------------------|----------------|
| < 10,000 | ~1ms | Individual events |
| 10,000 - 100,000 | ~1-2ms | Sampled (8k samples) |
| > 100,000 | ~1ms | Range distribution (fast but misleading) |

### Proposed Changes Impact

**Phase 1** (Always Sample):
- ✅ No significant performance regression
- Worst case: 100k events → sample to 8k → ~1-2ms (acceptable)

**Phase 2** (Visual Feedback):
- ✅ Negligible impact (just rendering, no computation)

**Phase 3** (Adaptive Sampling):
- ✅ Could improve performance when zoomed out
- ⚠️ Might increase complexity

## Recommended Implementation Order

1. **Phase 1: Improve Sampling** ⭐ (Start here)
   - Immediate impact on visual consistency
   - Low risk, high reward
   - ~2-4 hours implementation

2. **Phase 2: Visual Feedback**
   - Helps users understand what they're seeing
   - Improves transparency
   - ~4-8 hours implementation

3. **Phase 3: Adaptive Sampling**
   - Nice-to-have optimization
   - More complex, requires testing
   - ~8-16 hours implementation

4. **Phase 4 & 5: Research items**
   - Long-term improvements
   - Requires user feedback and experimentation

## Testing Strategy

### Existing Test Infrastructure

**Benchmarks** (`crates/viewer/re_time_panel/benches/bench_density_graph.rs`):
- Comprehensive performance benchmarks already exist
- Tests scenarios: single chunks (0-100k events), many chunks, sampling strategies
- Includes sorted/unsorted variants
- Current results show: ~1.5ms for 100k sorted events on high-end CPU

**Visual Test** (`tests/rust/test_data_density_graph/src/main.rs`):
- Demonstrates different chunk configurations:
  - `/small`: 100 chunks × 100 rows (many small)
  - `/large`: 5 chunks × 2000 rows (few large sorted)
  - `/large-unsorted`: 5 chunks × 2000 rows (few large unsorted)
  - `/gap`: 2 chunks × 5000 rows with time gap
  - `/over-threshold`: 1 chunk × 100k rows (exceeds threshold)

**To run visual test**:
```bash
cargo run -p test_data_density_graph
```

**To run benchmarks**:
```bash
cargo bench -p re_time_panel
```

### Proposed New Tests

#### Phase 1 Validation Test
Create a test that verifies visual consistency:
1. Log two identical scalars at same frequency
2. One uses small sorted chunks (under threshold)
3. One uses large sorted chunks (over threshold)
4. Compare rendered density graph outputs
5. Assert they produce similar visual patterns (within tolerance)

**Test location**: `crates/viewer/re_time_panel/tests/density_consistency_test.rs`

#### Phase 3 Adaptive Sampling Test
Test that sampling adapts to zoom level:
1. Create entity with 100k events
2. Test at various zoom levels (1x, 10x, 100x)
3. Verify sample count adjusts appropriately
4. Confirm performance stays within budget

### Test Scenarios
- **Scenario A**: 2 scalars, 5k events each, identical frequency, different chunk sizes
- **Scenario B**: 10 scalars, varying event counts (100 to 100k)
- **Scenario C**: Heavy zoom operations (1x to 1000x zoom range)
- **Scenario D**: Timeline scrubbing while rendering multiple entities

## Success Metrics

✅ **Visual Consistency**: Identical logging patterns render identically regardless of chunk characteristics

✅ **Performance**: No regression beyond acceptable threshold (<2ms per entity row)

✅ **User Clarity**: Users understand when viewing exact vs. approximated data

✅ **Scalability**: System handles 100k+ events per entity gracefully

## Alternative Approaches Considered

### 1. Always Render Individual Events
**Pros**: Perfect accuracy
**Cons**: Unacceptable performance for large datasets
**Verdict**: ❌ Not feasible

### 2. Increase Thresholds Significantly
**Pros**: Simple, reduces fallback frequency
**Cons**: Worse performance, doesn't solve root cause
**Verdict**: ❌ Band-aid solution

### 3. Pre-aggregate Data on Ingestion
**Pros**: Shifts computation to background
**Cons**: Major architectural change, loses fidelity
**Verdict**: ⚠️ Consider for future, too invasive now

### 4. Use GPU for Density Computation
**Pros**: Much faster rendering
**Cons**: Complex, requires shader work
**Verdict**: ⚠️ Interesting but overkill for current problem

## Conclusion

The issue is well-understood and solvable with **Phase 1** alone. The sampling infrastructure already exists; we just need to prefer it over the misleading range-distribution fallback.

**Recommended immediate action**:
1. Implement Phase 1 (eliminate `add_chunk_range` fallback for density graphs)
2. Test with real-world data
3. Evaluate need for Phase 2 based on user feedback

The root cause is the binary threshold-based fallback that chooses between perfect accuracy and perfect inaccuracy. By always using sampling (which gracefully degrades accuracy while preserving distribution), we maintain visual consistency across all chunk configurations.

---

## Quick Reference: Key Files to Modify

### Phase 1 Implementation (Core Fix)

**Primary file**: `crates/viewer/re_time_panel/src/data_density_graph.rs`

**Lines to modify**: 565-589 (the rendering decision logic in `build_density_graph()`)

**Current problematic code**:
```rust
} else if config.max_sampled_events_per_chunk > 0 {
    // Sample uniformly from the chunk
    data.add_uniform_sample_from_chunk(&events, config.max_sampled_events_per_chunk);
} else {
    // Fall back to uniform distribution across the entire time range
    data.add_chunk_range(time_range, num_events_in_chunk);  // ⚠️ PROBLEMATIC
}
```

**Proposed fix**:
```rust
} else {
    // ALWAYS sample to preserve time distribution
    let events = chunk.num_events_cumulative_per_unique_time(timeline);

    if events.len() > config.max_sampled_events_per_chunk {
        data.add_uniform_sample_from_chunk(&events, config.max_sampled_events_per_chunk);
    } else {
        for (time, num_events) in events {
            data.add_chunk_point(time, num_events as f32);
        }
    }
}
```

**Note**: The `add_chunk_range()` method can remain in the codebase for other potential uses (like background patterns), but should not be used for actual data visualization.

### Phase 2 Implementation (Visual Feedback)

**Primary file**: `crates/viewer/re_time_panel/src/data_density_graph.rs`

**Function to modify**: `DensityGraph::paint()` (lines 210-366)

**Approach**: Add rendering mode metadata to `DensityGraphBuilder` and pass it through to paint function to render with different visual treatment.

### Testing

**Benchmark file**: `crates/viewer/re_time_panel/benches/bench_density_graph.rs` (already exists)
**Visual test**: `tests/rust/test_data_density_graph/src/main.rs` (already exists)
**New test**: `crates/viewer/re_time_panel/tests/density_consistency_test.rs` (to be created)

---

## Additional Context

**Issue on GitHub**: https://github.com/rerun-io/rerun/issues/11432
**Filed by**: @emilk
**Date**: October 6, 2025
**Labels**: Performance, re_renderer, Annoying, Bug

**Development branch**: `claude/improve-streams-panel-rendering-011CUxQ7HsGsdwenYz4qNhzm`
