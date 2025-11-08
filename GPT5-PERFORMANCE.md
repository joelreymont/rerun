```markdown
# Per-frame performance analysis (Issue #8233)

## Observed per-frame workload

### Blueprint tree rebuilds
- Every frame the blueprint panel calls `BlueprintTreeData::from_blueprint_and_filter`, eagerly rebuilding the full tree of containers and views before rendering, regardless of whether the UI actually needs all of the nodes.

- The builder walks the entire blueprint hierarchy and applies filtering on the fly even if nothing has changed, as noted by the module docs in `data.rs` and the recursive construction of `ContainerData`/`ViewData`. This means collapsed panes still trigger a full traversal of containers and their children each frame.


### Query result traversal per view
- For each view, `ViewData::from_blueprint_and_filter` fetches the `DataQueryResult`, then rebuilds origin and projection subtrees by traversing the query tree and repeatedly clearing temporary buffers, work that is repeated on every frame even if the query output is unchanged.

- `DataResultData::from_data_result_and_filter` recursively descends through every matching `DataResultNode`, cloning labels, merging highlight ranges, and sorting children because the source nodes store them in an unordered `SmallVec`, producing extra allocations and comparisons per frame.


### View system execution
- `execute_systems_for_view` rebuilds the `PerSystemDataResults` map by visiting every node in the `DataResultTree` and collecting visualizers before running the view systems each frame, so unchanged scenes still walk the entire result tree.

- Ahead of per-view execution, `execute_systems_for_all_views` runs `run_once_per_frame_context_systems` for every view class, even when their inputs have not changed, adding constant overhead proportional to the number of registered context systems.


## Improvement opportunities

1. **Cache blueprint tree construction** – Maintain a cached `BlueprintTreeData` keyed by blueprint change generation, query generation, and active filter string so `tree_ui` can reuse the structure when the UI is static. Only rebuild the affected branch when the user edits the layout, using the existing viewport blueprint accessors to walk just the dirty container or view.

2. **Persist filtered hierarchies** – Pool the `hierarchy`/`hierarchy_highlights` buffers per view and reuse the computed highlight ranges across frames. When the filter is inactive (`FilterMatcher::is_active` is false) skip recomputing matches entirely and return cached visibility flags.

3. **Avoid per-frame resorting** – Ensure `DataResultNode` children are sorted once when the tree is built (or cache a sorted handle list) so `DataResultData::from_data_result_and_filter` can iterate without calling `children.sort_by` on every visit.

4. **Short-circuit collapsed or invisible branches** – Reuse the existing collapse state (`collapse_scope` in the UI and `BlueprintTreeItem::visit`) to avoid generating data for subtrees that are not visible, and skip nodes where `DataResult::is_visible` is false before recursing.

5. **Throttle per-frame system work** – Associate generation counters with `run_once_per_frame_context_systems` outputs and `PerSystemDataResults` so views only recompute when the underlying `DataResultTree` or time query changes. Cache the `SystemExecutionOutput` per view keyed by query hash and invalidate it when stores advance.


## Measuring improvements with existing tooling

- **Profiling scopes**: Augment hot paths with additional `profile_scope!` / `profile_function!` markers to distinguish cache hits from misses. These macros already integrate with puffin tracing and cost virtually nothing when disabled.

- **Puffin viewer**: Use `re_tracing::Profiler::start` to spin up the puffin HTTP server locally and connect `puffin_viewer`, allowing before/after comparison of the blueprint tree rebuild cost and view execution time.

- **Frame-time HUD**: Track end-to-end impact via the existing CPU frame-time history and top-panel label, which smooths samples across frames and highlights regressions directly in the viewer UI.

- **Lightweight counters**: Log the number of nodes traversed or cache misses when building `BlueprintTreeData` or `PerSystemDataResults` using the existing traversal entry points (`BlueprintTreeData::visit`, `execute_systems_for_view`) so automated runs can flag regressions without interactive profiling.
```
