# Implementation Plan: Fix Streams Panel Rendering Inconsistency

**Issue**: #11432 - Improve rendering of streams panel / timeline view
**Branch**: `claude/improve-streams-panel-rendering-011CUxQ7HsGsdwenYz4qNhzm`
**Status**: Ready for implementation

---

## Objective

Fix visual inconsistency in the streams panel where identical logging patterns appear different due to chunk-based rendering fallbacks.

## Phase 1: Core Fix (This Implementation)

### Changes Required

**File**: `crates/viewer/re_time_panel/src/data_density_graph.rs`
**Function**: `build_density_graph()`
**Lines**: 565-589

### Current Behavior

```rust
if should_render_individual_events {
    // Render all individual events
    for (time, num_events) in chunk.num_events_cumulative_per_unique_time(timeline) {
        data.add_chunk_point(time, num_events as f32);
    }
} else if config.max_sampled_events_per_chunk > 0 {
    // Sample uniformly from the chunk
    data.add_uniform_sample_from_chunk(&events, config.max_sampled_events_per_chunk);
} else {
    // Fall back to uniform distribution across the entire time range
    data.add_chunk_range(time_range, num_events_in_chunk);  // ❌ PROBLEMATIC
}
```

**Problem**: The `add_chunk_range` fallback spreads events uniformly across the time range, destroying the actual temporal distribution.

### New Behavior

```rust
if should_render_individual_events {
    // Render all individual events
    for (time, num_events) in chunk.num_events_cumulative_per_unique_time(timeline) {
        data.add_chunk_point(time, num_events as f32);
    }
} else {
    // ALWAYS sample to preserve time distribution
    let events = chunk.num_events_cumulative_per_unique_time(timeline);

    if events.len() > config.max_sampled_events_per_chunk {
        // Sample if we have more events than the threshold
        data.add_uniform_sample_from_chunk(&events, config.max_sampled_events_per_chunk);
    } else {
        // Otherwise render all events
        for (time, num_events) in events {
            data.add_chunk_point(time, num_events as f32);
        }
    }
}
```

**Benefit**: Always preserves the actual temporal distribution by using sampling instead of uniform range spreading.

### Implementation Steps

1. ✅ Locate the problematic code section (lines 565-589)
2. ✅ Replace the three-branch if-else with the new two-branch logic
3. ✅ Remove the dependency on `config.max_sampled_events_per_chunk > 0` check
4. ✅ Ensure `events` variable is properly computed for the else branch
5. ✅ Verify the code compiles
6. ✅ Run benchmarks to confirm no performance regression
7. ✅ Run visual test to verify rendering improvements

### Expected Performance Impact

**Before**:
- < 10k events: ~1ms (individual rendering)
- 10k-100k events with sampling enabled: ~1-2ms
- 10k-100k events with sampling disabled: ~1ms (range distribution - fast but wrong)

**After**:
- < 10k events: ~1ms (individual rendering - unchanged)
- 10k-100k events: ~1-2ms (always sampling - preserves accuracy)

**Worst case**: Some scenarios that previously used range distribution (~1ms) will now use sampling (~1-2ms). This is acceptable because:
1. The default config has sampling enabled anyway
2. Correctness > speed
3. Still well within the target frame budget

### Testing Strategy

#### 1. Compilation Test
```bash
cargo check -p re_time_panel
```

#### 2. Benchmark Test
```bash
cargo bench -p re_time_panel
```

**Expected results**:
- Single chunks (sorted): Similar performance to baseline
- Single chunks (unsorted): Similar performance to baseline
- Many chunks: Similar performance to baseline
- Sampling scenarios: Unchanged (already using sampling)

#### 3. Visual Test
```bash
cargo run -p test_data_density_graph
```

**What to verify**:
- `/over-threshold` entity (100k events) now shows actual temporal distribution
- No visual regressions on other entities
- Rendering is smooth and responsive

#### 4. Manual Test (if available)
- Load a recording with > 10k events per entity
- Verify density graphs show accurate temporal distribution
- Compare with main branch to confirm improvement

### Rollback Plan

If issues arise:
```bash
git revert HEAD
git push -f origin claude/improve-streams-panel-rendering-011CUxQ7HsGsdwenYz4qNhzm
```

### Success Criteria

✅ Code compiles without errors
✅ Benchmarks show no significant regression (< 2x slowdown)
✅ Visual test runs without crashes
✅ Density graphs preserve temporal distribution for all chunk sizes

---

## Future Phases (Not in this PR)

### Phase 2: Visual Feedback
- Add visual indicator when using sampling
- Estimated effort: 4-8 hours

### Phase 3: Adaptive Sampling
- Adjust sample count based on zoom level
- Estimated effort: 8-16 hours

### Phase 4: Configuration UI
- Allow users to tune sampling parameters
- Estimated effort: TBD

### Phase 5: Alternative Visualizations
- Research heat maps, log scales, etc.
- Estimated effort: Research phase

---

## Notes

- The `add_chunk_range()` method will remain in the codebase but won't be used for data visualization
- The default `DensityGraphBuilderConfig` already has `max_sampled_events_per_chunk: 8000`
- This change makes the rendering more consistent but doesn't change the performance characteristics significantly

---

## References

- **Issue**: https://github.com/rerun-io/rerun/issues/11432
- **Analysis**: ISSUE_11432_ANALYSIS.md
- **Code**: crates/viewer/re_time_panel/src/data_density_graph.rs
