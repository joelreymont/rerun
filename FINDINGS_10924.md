# Issue #10924: Push Down Filter partition_id

## Summary

Issue #10924 requests implementing filter push-down optimization for `rerun_partition_id` to improve query performance. This is identified as a high-value, relatively low-effort enhancement.

## Requirements

The implementation should support four filter patterns:

1. **Equality**: `partition_id == value`
2. **Inequality**: `partition_id != value`
3. **Inclusion**: `partition_id IN [list]`
4. **Exclusion**: `partition_id NOT IN [list]`

## Current State

### Existing partition_id Filtering

Currently, partition filtering works at the API/request level:

**Python API** (`rerun_py/src/catalog/dataframe_query.rs:106-126`):
- `filter_partition_id()` method stores partition IDs in `PyDataframeQueryView` struct
- Partition IDs are passed to `DataframeQueryTableProvider::new()` (line 457)

**DataFusion Provider** (`crates/store/re_datafusion/src/dataframe_query_common.rs:111-115`):
- Partition IDs included in gRPC `QueryDatasetRequest`
- Sent to server which filters partitions server-side

**Server-Side** (`crates/store/re_server/src/store/dataset.rs:84-102`):
- `partitions_from_ids()` filters partitions based on provided IDs
- If empty, returns all partitions; otherwise only specified ones

### Existing Filter Push-Down Pattern

There's already a filter push-down mechanism for component columns:

**File**: `crates/store/re_datafusion/src/dataframe_query_common.rs`

**Lines 276-300**: `supports_filters_pushdown()` - Supports pushing down IS NOT NULL filters:
```rust
fn supports_filters_pushdown(
    &self,
    filters: &[&Expr],
) -> datafusion::common::Result<Vec<TableProviderFilterPushDown>> {
    let filter_columns = Self::compute_column_is_neq_null_filter(filters);
    // ... checks if filter can be pushed down as Exact or Unsupported
}
```

**Lines 216-223**: `compute_column_is_neq_null_filter()` - Identifies component column IS NOT NULL filters

**Lines 240-274**: `scan()` method - Applies pushed-down filters to `query_expression.filtered_is_not_null`

### Schema Information

**Lines 162-165** (`dataframe_query_common.rs`): `rerun_partition_id` column is prepended to schema:
```rust
let schema = Arc::new(Schema::new([Field::new(
    RERUN_PARTITION_ID,
    DataType::Utf8,
    false, // not nullable
)]));
```

This column is available for filtering but currently not optimized through DataFusion's push-down mechanism.

## Proposed Implementation

### High-Level Approach

Add support for `rerun_partition_id` column filtering in `supports_filters_pushdown()` similar to the existing component column filtering. This would allow DataFusion to optimize queries like:

```python
# Instead of (or in addition to) API-level:
df.filter_partition_id(["partition1", "partition2"])

# Support DataFusion filter push-down:
df.filter(col("rerun_partition_id") == "partition1")
df.filter(col("rerun_partition_id").is_in(["partition1", "partition2"]))
df.filter(col("rerun_partition_id") != "partition1")
df.filter(~col("rerun_partition_id").is_in(["partition1", "partition2"]))
```

### Implementation Steps

1. **Extend `supports_filters_pushdown()`** in `dataframe_query_common.rs`:
   - Detect filters on `rerun_partition_id` column
   - Support equality, inequality, IN, and NOT IN operations
   - Return `TableProviderFilterPushDown::Exact` for supported patterns

2. **Add filter extraction method**:
   - Create `compute_partition_id_filters()` to extract partition ID filter expressions
   - Handle the four required patterns: ==, !=, IN, NOT IN
   - Extract partition IDs from filter expressions

3. **Update `scan()` method**:
   - Apply extracted partition ID filters to the query request
   - Merge with any existing partition IDs from `filter_partition_id()` API calls
   - Pass filtered partition IDs to `DataframeQueryTableProvider::new()`

4. **Update data structures** (if needed):
   - Ensure `partition_ids` field can represent both inclusion and exclusion patterns
   - May need to add a flag or separate field for exclusion filters

### Key Files to Modify

- `crates/store/re_datafusion/src/dataframe_query_common.rs` - Main implementation
- `crates/store/re_datafusion/src/dataframe_query_provider.rs` - May need updates for execution
- `crates/store/re_server/src/store/dataset.rs` - Server-side filtering logic (if exclusion patterns added)

### Benefits

1. **Performance**: DataFusion can eliminate partitions early in query planning
2. **Consistency**: SQL-style filtering aligns with standard DataFusion patterns
3. **Flexibility**: Users can mix API-level and SQL-style filtering
4. **Optimization**: Reduces gRPC payload by fetching only relevant chunk metadata

## Testing Considerations

Existing test shows the pattern:

**File**: `rerun_py/tests/e2e_redap_tests/test_dataset_query.py:45-80`
- `test_partition_ordering()` uses `rerun_partition_id` column
- Could be extended to test filter push-down optimization

New tests should verify:
1. Equality filters: `col("rerun_partition_id") == "partition1"`
2. Inequality filters: `col("rerun_partition_id") != "partition1"`
3. IN filters: `col("rerun_partition_id").is_in(["p1", "p2"])`
4. NOT IN filters: `~col("rerun_partition_id").is_in(["p1", "p2"])`
5. Combination with existing `filter_partition_id()` API
6. Performance improvement verification

## Related Code Patterns

**Example Usage** (`examples/python/server_tables/server_tables.py:94`):
```python
.filter_partition_id()  # Current API-level approach
```

**Component Filter Test** (`rerun_py/tests/e2e_redap_tests/test_dataset_query.py:11-43`):
- `test_component_filtering()` demonstrates the push-down pattern for components
- Similar test structure should be created for partition_id filtering

## Potential Challenges

1. **Exclusion Logic**: NOT and NOT IN patterns may require server-side changes if current implementation only supports inclusion
2. **Filter Merging**: Need to handle cases where both API `filter_partition_id()` and DataFusion filters are used
3. **Expression Complexity**: Need to handle complex expressions like `(partition_id == "p1") OR (partition_id == "p2")`

## Next Steps

1. Implement filter detection for `rerun_partition_id` in `supports_filters_pushdown()`
2. Add extraction logic for the four filter patterns
3. Update `scan()` to apply extracted filters
4. Add comprehensive tests for all filter patterns
5. Verify performance improvements with benchmarks
