# Issue #11254: Column Statistics in Table Provider

**Date:** 2025-11-09
**Issue:** https://github.com/rerun-io/rerun/issues/11254
**Status:** Implementation Planning

---

## Executive Summary

This document provides findings and an implementation plan for adding column statistics to Rerun's TableProvider implementations. Column statistics enable DataFusion's query optimizer to make better decisions, particularly for aggregate functions like MIN/MAX, by utilizing pre-computed metadata from chunk information rather than scanning entire datasets.

---

## Issue Overview

### Problem Statement
Currently, Rerun's TableProvider implementations don't expose column statistics to DataFusion's query optimizer. This means that queries involving aggregate functions (MIN, MAX, COUNT, etc.) must scan entire datasets even when the required information is already available in chunk metadata.

### Proposed Solution
Implement DataFusion's `Statistics` and `ColumnStatistics` for TableProvider implementations, leveraging the existing chunk metadata that Rerun already computes and maintains.

### Expected Benefits
- **Query Performance**: Significant speedup for aggregate queries (MIN/MAX/COUNT)
- **Resource Efficiency**: Reduced CPU and memory usage by avoiding full table scans
- **Better Query Plans**: DataFusion can make smarter decisions about join ordering and execution strategies
- **Zero Runtime Cost**: Statistics are computed from existing metadata

---

## Technical Findings

### 1. Current TableProvider Architecture

**Location:** `/home/user/rerun/crates/store/re_datafusion/src/`

Rerun implements multiple TableProviders using a two-layer architecture:

#### Layer 1: Trait Implementation (`GrpcStreamToTable`)
Generic trait for converting gRPC streams to Arrow RecordBatches:
```rust
pub trait GrpcStreamToTable {
    async fn fetch_schema(&mut self) -> DataFusionResult<SchemaRef>;
    async fn send_streaming_request(&mut self) -> ...;
    fn process_response(&mut self, response: ...) -> ...;
    fn supports_filters_pushdown(&self, filters: &[&Expr]) -> ...;
}
```

#### Layer 2: Generic Wrapper (`GrpcStreamProvider<T>`)
Wraps any `GrpcStreamToTable` implementation to satisfy DataFusion's `TableProvider` trait:
```rust
pub struct GrpcStreamProvider<T: GrpcStreamToTable> {
    schema: SchemaRef,
    client: T,
}

impl<T> TableProvider for GrpcStreamProvider<T> {
    fn schema(&self) -> SchemaRef;
    async fn scan(...) -> Arc<dyn ExecutionPlan>;
    // statistics() method not implemented - uses default
}
```

#### Concrete Implementations
- **`TableEntryTableProvider`** (`table_entry_provider.rs`) - For table entries
- **`PartitionTableProvider`** (`partition_table.rs`) - For partition metadata
- **`SearchResultsTableProvider`** (`search_provider.rs`) - For search results
- **`DatasetManifestProvider`** (`dataset_manifest.rs`) - For dataset manifests
- **`DataframeQueryTableProvider`** (`dataframe_query_common.rs`) - For dataframe queries (most complex)

**Key Finding:** None of these implementations currently override the `statistics()` method from the `TableProvider` trait, meaning they return default (empty) statistics.

### 2. Available Chunk Metadata

**Location:** `/home/user/rerun/crates/store/re_chunk/src/chunk.rs`

The `Chunk` structure contains rich metadata suitable for computing statistics:

```rust
pub struct Chunk {
    pub(crate) id: ChunkId,
    pub(crate) entity_path: EntityPath,
    pub(crate) heap_size_bytes: AtomicU64,  // Cached heap size
    pub(crate) is_sorted: bool,
    pub(crate) row_ids: FixedSizeBinaryArray,
    pub(crate) timelines: IntMap<TimelineName, TimeColumn>,
    pub(crate) components: ChunkComponents,
}
```

**Available Statistics Methods:**
- `num_rows() -> usize` - Row count (exact)
- `heap_size_bytes() -> u64` - Total bytes (exact)
- `num_events_cumulative() -> u64` - Total component batches
- `time_range_per_component() -> IntMap<...>` - Min/max time ranges per component
- `num_events_for_component(ComponentIdentifier) -> Option<u64>` - Component-level counts

**TimeColumn Statistics:**
```rust
pub struct TimeColumn {
    pub(crate) timeline: Timeline,
    pub(crate) times: ArrowScalarBuffer<i64>,
    pub(crate) is_sorted: bool,
    pub(crate) time_range: AbsoluteTimeRange,  // Min/max pre-computed!
}
```

**Key Finding:** Time ranges are already computed and cached in the `TimeColumn` structure. This provides exact MIN/MAX values for temporal columns at zero cost.

### 3. gRPC Chunk Metadata

**Location:** `/home/user/rerun/crates/store/re_datafusion/src/dataframe_query_common.rs:46-52`

The `DataframeQueryTableProvider` stores chunk metadata from `QueryDatasetResponse`:

```rust
pub struct DataframeQueryTableProvider {
    pub schema: SchemaRef,
    query_expression: QueryExpression,
    sort_index: Option<Index>,
    chunk_info_batches: Arc<Vec<RecordBatch>>,  // ← Chunk metadata!
    client: ConnectionClient,
}
```

**Chunk Metadata Fields** (from `re_protos`):
- `chunk_id` - Unique chunk identifier (FixedSizeBinary(16))
- `chunk_partition_id` - Partition identifier (Utf8)
- `chunk_layer_name` - Layer name (Utf8)
- `chunk_key` - Storage location (Binary)
- `chunk_entity_path` - Entity path (Utf8)
- `chunk_is_static` - Static vs temporal flag (Boolean)
- Plus: component columns, timeline columns, index columns

**Key Finding:** The `chunk_info_batches` contains metadata for all chunks that will be fetched. We can aggregate this metadata to compute table-level statistics without fetching actual chunk data.

### 4. FFI Integration

**Location:** `/home/user/rerun/rerun_py/src/catalog/datafusion_table.rs`

The Python FFI layer uses `datafusion_ffi::FFI_TableProvider`:

```rust
use datafusion_ffi::table_provider::FFI_TableProvider;

let provider = FFI_TableProvider::new(
    Arc::clone(&self.provider),  // Wraps our TableProvider
    false,                        // is_unitary
    Some(runtime)                 // Tokio runtime
);
```

**Key Finding:** The FFI layer is a thin wrapper. If our Rust `TableProvider::statistics()` method returns proper statistics, they should automatically be available to Python through the FFI boundary.

### 5. DataFusion Statistics API

**DataFusion Version:** 50.1.0

Based on DataFusion documentation and common patterns:

The `TableProvider` trait includes:
```rust
fn statistics(&self) -> Result<Statistics> {
    Ok(Statistics::new_unknown(&self.schema()))
}
```

The `Statistics` structure typically includes:
- `num_rows: Option<usize>` - Total row count
- `total_byte_size: Option<usize>` - Total size in bytes
- `column_statistics: Vec<ColumnStatistics>` - Per-column statistics

The `ColumnStatistics` structure typically includes:
- `null_count: Option<usize>` - Number of null values
- `max_value: Option<ScalarValue>` - Maximum value
- `min_value: Option<ScalarValue>` - Minimum value
- `distinct_count: Option<usize>` - Number of distinct values

**Note:** Exact API structure needs verification in DataFusion 50.1.0 source code during implementation.

---

## Implementation Plan

### Phase 1: Research and Preparation

#### Task 1.1: Verify DataFusion Statistics API
**File:** Research task
**Action:**
- Locate DataFusion 50.1.0 source in Cargo cache or download
- Document exact structure of `Statistics` and `ColumnStatistics`
- Identify all fields and their types
- Check for any version-specific quirks or requirements
- Document expected behavior when statistics are unavailable

**Deliverable:** API documentation snippet for reference

#### Task 1.2: Analyze Chunk Metadata Aggregation
**File:** `crates/store/re_datafusion/src/dataframe_query_common.rs`
**Action:**
- Study how `chunk_info_batches` is populated in `DataframeQueryTableProvider::new()`
- Identify which fields are guaranteed to be present
- Determine how to aggregate across multiple chunks
- Understand partition-based grouping logic (see `group_chunk_infos_by_partition_id()`)

**Deliverable:** Design document for aggregation strategy

### Phase 2: Core Implementation

#### Task 2.1: Implement Statistics Aggregation Helper
**File:** `crates/store/re_datafusion/src/statistics.rs` (new file)
**Action:**
Create a new module for statistics computation:

```rust
use arrow::datatypes::SchemaRef;
use arrow::array::RecordBatch;
use datafusion::physical_plan::Statistics;
use std::sync::Arc;

/// Compute table statistics from chunk metadata batches.
///
/// This aggregates chunk-level metadata (row counts, byte sizes, time ranges)
/// to produce table-level statistics for DataFusion's query optimizer.
pub fn compute_statistics_from_chunks(
    schema: &SchemaRef,
    chunk_info_batches: &[RecordBatch],
) -> datafusion::common::Result<Statistics> {
    // 1. Aggregate row counts across all chunks
    // 2. Aggregate byte sizes across all chunks
    // 3. For each column in schema:
    //    - If it's a time column, extract min/max from chunk metadata
    //    - If it's a component column, compute statistics from chunk info
    //    - Handle null counts if available
    // 4. Return Statistics struct
}

/// Compute column statistics for a specific column from chunk metadata.
fn compute_column_statistics(
    column_name: &str,
    column_type: &arrow::datatypes::DataType,
    chunk_info_batches: &[RecordBatch],
) -> datafusion::common::Result<datafusion::physical_plan::ColumnStatistics> {
    // Extract min/max/null_count for this specific column
}
```

**Deliverable:** Working statistics computation module with tests

#### Task 2.2: Add Statistics to `DataframeQueryTableProvider`
**File:** `crates/store/re_datafusion/src/dataframe_query_common.rs`
**Action:**

1. Add statistics field to struct:
```rust
pub struct DataframeQueryTableProvider {
    pub schema: SchemaRef,
    query_expression: QueryExpression,
    sort_index: Option<Index>,
    chunk_info_batches: Arc<Vec<RecordBatch>>,
    client: ConnectionClient,
    statistics: Statistics,  // ← Add this
}
```

2. Compute statistics in `new()`:
```rust
let statistics = crate::statistics::compute_statistics_from_chunks(
    &schema,
    &chunk_info_batches,
)?;

Ok(Self {
    schema,
    query_expression: query_expression.to_owned(),
    sort_index: query_expression.filtered_index,
    chunk_info_batches,
    client,
    statistics,  // ← Store computed statistics
})
```

3. Implement `statistics()` method:
```rust
impl TableProvider for DataframeQueryTableProvider {
    // ... existing methods ...

    fn statistics(&self) -> datafusion::common::Result<Statistics> {
        Ok(self.statistics.clone())
    }
}
```

**Deliverable:** Updated `DataframeQueryTableProvider` with statistics support

#### Task 2.3: Add Statistics to `GrpcStreamProvider`
**File:** `crates/store/re_datafusion/src/grpc_streaming_provider.rs`
**Action:**

This is more challenging because the streaming providers don't have chunk metadata upfront. Options:

**Option A:** Return unknown statistics (no change)
- Justification: Streaming data sources don't have metadata until streaming begins
- Minimal implementation impact

**Option B:** Extend `GrpcStreamToTable` trait with statistics method
- Add `fn statistics(&self) -> Result<Statistics>` to trait
- Each implementation provides appropriate statistics
- Default implementation returns unknown

**Recommendation:** Start with Option A for Phase 2, revisit in Phase 3 if needed.

**Deliverable:** Decision document and implementation (if Option B chosen)

### Phase 3: Extended Implementation

#### Task 3.1: Add Statistics to Specialized Providers
**Files:**
- `crates/store/re_datafusion/src/partition_table.rs`
- `crates/store/re_datafusion/src/search_provider.rs`
- `crates/store/re_datafusion/src/dataset_manifest.rs`

**Action:**
For each provider:
1. Analyze what metadata is available
2. Determine if statistics can be computed cheaply
3. If yes, implement `statistics()` method
4. If no, document why and return unknown statistics

**Examples:**
- `PartitionTableProvider`: Can compute row count from partition metadata
- `SearchResultsTableProvider`: May need to stream first to know statistics
- `DatasetManifestProvider`: Can compute from manifest metadata

**Deliverable:** Statistics support for all appropriate providers

#### Task 3.2: Optimize Time Column Statistics
**File:** `crates/store/re_datafusion/src/statistics.rs`
**Action:**

Time columns have pre-computed ranges in `TimeColumn::time_range`. Optimize statistics computation:

1. Detect time columns in schema (use `ColumnKind::try_from()`)
2. For time columns, extract min/max directly from chunk metadata
3. Convert `AbsoluteTimeRange` to appropriate `ScalarValue` type
4. Ensure timezone handling is correct

**Deliverable:** Optimized time column statistics with validation tests

#### Task 3.3: Component-Level Statistics
**File:** `crates/store/re_datafusion/src/statistics.rs`
**Action:**

Component columns may have useful statistics:
1. Null counts (from validity bitmaps)
2. Row counts (from `num_events_for_component()`)
3. Potentially min/max for numeric components

**Deliverable:** Component statistics support where applicable

### Phase 4: Testing and Validation

#### Task 4.1: Unit Tests
**File:** `crates/store/re_datafusion/src/statistics.rs`
**Action:**

Create comprehensive unit tests:
- Test with empty chunks
- Test with single chunk
- Test with multiple chunks
- Test with different column types
- Test with missing metadata
- Test with static vs temporal data
- Test aggregation correctness

**Deliverable:** Test suite with >90% coverage of statistics module

#### Task 4.2: Integration Tests
**File:** `crates/store/re_datafusion/tests/statistics_integration.rs` (new)
**Action:**

Test end-to-end statistics flow:
1. Create a `DataframeQueryTableProvider` with known data
2. Verify `statistics()` returns expected values
3. Execute queries that should use statistics (MIN/MAX/COUNT)
4. Verify query plans show optimization (use `EXPLAIN`)
5. Verify results are correct

**Deliverable:** Integration test suite

#### Task 4.3: FFI Validation
**File:** `rerun_py/tests/` (new Python test)
**Action:**

Verify statistics propagate through FFI:
1. Create a Python test using `PyDataFusionTable`
2. Access table statistics from Python
3. Verify values match Rust-side expectations
4. Test query optimization from Python side

**Deliverable:** Python FFI test

#### Task 4.4: Performance Benchmarking
**File:** `crates/store/re_datafusion/benches/statistics.rs` (new)
**Action:**

Benchmark statistics impact:
1. Measure time to compute statistics for various dataset sizes
2. Compare query execution time with/without statistics
3. Verify statistics computation doesn't significantly slow down table creation
4. Document performance characteristics

**Deliverable:** Benchmark results and analysis

### Phase 5: Documentation and Polish

#### Task 5.1: Code Documentation
**Files:** All modified files
**Action:**
- Add rustdoc comments to all public functions
- Document statistics computation strategy
- Add examples showing how statistics improve queries
- Document limitations and edge cases

**Deliverable:** Complete API documentation

#### Task 5.2: User Documentation
**File:** `crates/store/re_datafusion/README.md`
**Action:**
- Update README with statistics support information
- Add examples of queries that benefit from statistics
- Document any user-visible changes

**Deliverable:** Updated README

#### Task 5.3: Migration Guide
**File:** `CHANGELOG.md` or migration guide
**Action:**
- Document changes in behavior (if any)
- Note performance improvements
- Mention FFI compatibility

**Deliverable:** Migration/changelog entry

---

## Technical Challenges and Considerations

### Challenge 1: Aggregating Statistics Across Chunks
**Problem:** Chunk metadata is distributed across multiple `RecordBatch`es in `chunk_info_batches`.

**Solution:**
- Iterate through all batches
- Use Arrow compute kernels for efficient aggregation
- Handle missing/null metadata gracefully

**Example Strategy:**
```rust
let mut total_rows = 0usize;
for batch in chunk_info_batches {
    if let Some(row_count_col) = batch.column_by_name("row_count") {
        // Aggregate...
    }
}
```

### Challenge 2: Column Type Detection
**Problem:** Need to distinguish time columns, component columns, and other column types for appropriate statistics.

**Solution:**
- Use `re_sorbet::ColumnKind::try_from()` to detect column types
- Different statistics strategies for different column kinds
- Gracefully handle unknown column types

### Challenge 3: MIN/MAX Value Type Conversion
**Problem:** Converting chunk metadata (i64 for times) to appropriate `ScalarValue` for statistics.

**Solution:**
- Use Arrow's type system to ensure correct conversion
- Handle different timestamp units (seconds, milliseconds, microseconds, nanoseconds)
- Reference: `time_array_ref_to_i64()` in `dataframe_query_common.rs:456-487`

### Challenge 4: Null Handling
**Problem:** Chunk metadata might not include null counts for all columns.

**Solution:**
- Return `None` for statistics we can't compute
- DataFusion handles partial statistics gracefully
- Document what statistics are and aren't available

### Challenge 5: Static vs Temporal Data
**Problem:** Static data doesn't have time ranges, temporal data does.

**Solution:**
- Check `chunk_is_static` flag in metadata
- Handle static data appropriately (time = `TimeInt::STATIC`)
- Ensure statistics reflect the correct data type

### Challenge 6: FFI Compatibility
**Problem:** Need to ensure statistics work correctly through the Python FFI layer.

**Solution:**
- `datafusion_ffi` should handle this automatically
- Verify with integration tests
- Check `datafusion_ffi` version (50.1.0) for compatibility

### Challenge 7: Performance Impact
**Problem:** Computing statistics shouldn't significantly slow down table creation.

**Solution:**
- Statistics computation is O(num_chunks), not O(num_rows)
- Use efficient Arrow compute kernels
- Cache statistics (already done - stored in struct)
- Benchmark to verify acceptable overhead

---

## Success Criteria

### Functional Requirements
- ✅ `DataframeQueryTableProvider::statistics()` returns accurate statistics
- ✅ Row count matches actual data
- ✅ Byte size matches actual data
- ✅ Time column MIN/MAX values are correct
- ✅ Statistics work through Python FFI
- ✅ All tests pass

### Performance Requirements
- ✅ Statistics computation adds <5% overhead to table creation
- ✅ Queries using MIN/MAX on time columns are >10x faster (for large datasets)
- ✅ No regression in non-statistical query performance

### Code Quality Requirements
- ✅ All public APIs documented with rustdoc
- ✅ Test coverage >80% for new code
- ✅ No clippy warnings
- ✅ Follows existing code style and patterns

---

## Testing Strategy

### 1. Unit Tests
**Location:** `crates/store/re_datafusion/src/statistics.rs`

Test individual functions:
- `compute_statistics_from_chunks()` with various inputs
- `compute_column_statistics()` for different column types
- Edge cases (empty data, null metadata, etc.)

### 2. Integration Tests
**Location:** `crates/store/re_datafusion/tests/`

Test full workflow:
- Create table provider with known data
- Verify statistics are correct
- Execute queries and verify results
- Check query plans for optimization

### 3. FFI Tests
**Location:** `rerun_py/tests/`

Test Python integration:
- Access statistics from Python
- Verify values propagate correctly
- Test query optimization from Python

### 4. End-to-End Tests
**Location:** Rerun e2e test suite

Test realistic scenarios:
- Large datasets with multiple chunks
- Mixed static and temporal data
- Various query patterns

---

## Timeline and Milestones

### Week 1: Research and Design
- [ ] Task 1.1: Verify DataFusion Statistics API
- [ ] Task 1.2: Analyze Chunk Metadata Aggregation
- [ ] Review and approve design

### Week 2: Core Implementation
- [ ] Task 2.1: Implement Statistics Aggregation Helper
- [ ] Task 2.2: Add Statistics to `DataframeQueryTableProvider`
- [ ] Task 2.3: Evaluate `GrpcStreamProvider` statistics
- [ ] Initial testing

### Week 3: Extended Implementation
- [ ] Task 3.1: Add Statistics to Specialized Providers
- [ ] Task 3.2: Optimize Time Column Statistics
- [ ] Task 3.3: Component-Level Statistics

### Week 4: Testing and Validation
- [ ] Task 4.1: Unit Tests
- [ ] Task 4.2: Integration Tests
- [ ] Task 4.3: FFI Validation
- [ ] Task 4.4: Performance Benchmarking

### Week 5: Documentation and Polish
- [ ] Task 5.1: Code Documentation
- [ ] Task 5.2: User Documentation
- [ ] Task 5.3: Migration Guide
- [ ] Final review and merge

---

## Open Questions

1. **DataFusion Statistics API:** What is the exact structure of `Statistics` and `ColumnStatistics` in version 50.1.0?
   - **Action:** Verify during Task 1.1

2. **Chunk Metadata Completeness:** Are row counts and byte sizes always available in `chunk_info_batches`?
   - **Action:** Investigate during Task 1.2

3. **Streaming Providers:** Should we implement statistics for `GrpcStreamProvider` or leave as unknown?
   - **Action:** Decide during Task 2.3

4. **Component Statistics:** Which component-level statistics are most valuable for query optimization?
   - **Action:** Analyze during Task 3.3

5. **FFI Behavior:** Does `datafusion_ffi` automatically propagate statistics, or do we need special handling?
   - **Action:** Verify during Task 4.3

---

## References

### Issue and Discussion
- GitHub Issue: https://github.com/rerun-io/rerun/issues/11254
- Issue Author: timsaucer
- Labels: dataplatform, enhancement, 📉 performance

### Code Locations
- **TableProvider Implementations:**
  - `crates/store/re_datafusion/src/grpc_streaming_provider.rs`
  - `crates/store/re_datafusion/src/dataframe_query_common.rs`
  - `crates/store/re_datafusion/src/table_entry_provider.rs`
  - `crates/store/re_datafusion/src/partition_table.rs`
  - `crates/store/re_datafusion/src/search_provider.rs`

- **Chunk Structures:**
  - `crates/store/re_chunk/src/chunk.rs`
  - `crates/store/re_protos/src/v1alpha1/rerun.cloud.v1alpha1.ext.rs`

- **FFI Layer:**
  - `rerun_py/src/catalog/datafusion_table.rs`

### External Documentation
- DataFusion TableProvider: https://datafusion.apache.org/library-user-guide/custom-table-providers.html
- DataFusion Statistics Blog: https://xebia.com/blog/making-joins-faster-in-datafusion-based-on-table-statistics/
- DataFusion API Docs: https://docs.rs/datafusion/50.1.0/datafusion/

---

## Appendix A: Example Statistics Computation

Here's a conceptual example of how statistics might be computed:

```rust
fn compute_statistics_from_chunks(
    schema: &SchemaRef,
    chunk_info_batches: &[RecordBatch],
) -> Result<Statistics> {
    // Aggregate total row count
    let mut total_rows = 0usize;
    let mut total_bytes = 0usize;

    for batch in chunk_info_batches {
        // Assuming chunk metadata includes row_count column
        if let Some(row_count_col) = batch.column_by_name("num_rows") {
            let counts = row_count_col.as_primitive::<Int64Type>();
            for count in counts.values() {
                total_rows += *count as usize;
            }
        }

        // Aggregate byte sizes
        if let Some(byte_size_col) = batch.column_by_name("heap_size_bytes") {
            let sizes = byte_size_col.as_primitive::<UInt64Type>();
            for size in sizes.values() {
                total_bytes += *size as usize;
            }
        }
    }

    // Compute per-column statistics
    let mut column_statistics = Vec::new();
    for field in schema.fields() {
        let col_stats = compute_column_statistics(
            field.name(),
            field.data_type(),
            chunk_info_batches,
        )?;
        column_statistics.push(col_stats);
    }

    Ok(Statistics {
        num_rows: Some(total_rows),
        total_byte_size: Some(total_bytes),
        column_statistics,
    })
}
```

---

## Appendix B: Example Query Optimization

**Without Statistics:**
```sql
SELECT MAX(timestamp) FROM my_table;
```
Query plan: Full table scan, compute max across all rows.

**With Statistics:**
```sql
SELECT MAX(timestamp) FROM my_table;
```
Query plan: Return pre-computed max from statistics (no scan needed!).

**Performance Impact:** 1000x+ speedup for large tables.

---

## Appendix C: Risk Assessment

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| DataFusion API changes between versions | Low | High | Verify API in Phase 1, pin version |
| Statistics computation is too slow | Low | Medium | Benchmark in Phase 4, optimize if needed |
| FFI doesn't propagate statistics | Low | Medium | Validate in Phase 4, add custom handling if needed |
| Chunk metadata is incomplete | Medium | High | Handle gracefully, return `None` for missing stats |
| Statistics are incorrect | Low | High | Comprehensive testing in Phase 4 |
| Performance regression | Low | Medium | Benchmark in Phase 4, optimize critical paths |

---

**Document Version:** 1.0
**Last Updated:** 2025-11-09
**Author:** Claude (AI Assistant)
**Reviewer:** TBD
