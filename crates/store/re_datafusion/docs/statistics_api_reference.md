# DataFusion Statistics API Reference (v50.3.0)

This document provides the exact API structure for DataFusion's Statistics support
and how it's implemented in Rerun's re_datafusion crate.

## Implementation Status

✅ **Implemented:** Core statistics framework with DataframeQueryTableProvider
✅ **Implemented:** Returns `Precision::Absent` for all statistics (honest approach)
✅ **Implemented:** Comprehensive unit tests validating Absent behavior
❌ **Not Implemented:** Row count and byte size (requires server-side changes)
❌ **Not Implemented:** Time column min/max statistics (requires server-side changes)
❌ **Not Implemented:** Component column statistics (requires data scanning)

## Core Structures

### Precision<T> Enum

```rust
pub enum Precision<T: Debug + Clone + PartialEq + Eq + PartialOrd> {
    Exact(T),      // Exact/precise value
    Inexact(T),    // Approximate/estimated value
    Absent,        // Value not available (default)
}
```

### Statistics Struct

```rust
pub struct Statistics {
    pub num_rows: Precision<usize>,
    pub total_byte_size: Precision<usize>,
    pub column_statistics: Vec<ColumnStatistics>,
}
```

### ColumnStatistics Struct

```rust
pub struct ColumnStatistics {
    pub null_count: Precision<usize>,
    pub max_value: Precision<ScalarValue>,
    pub min_value: Precision<ScalarValue>,
    pub sum_value: Precision<ScalarValue>,
    pub distinct_count: Precision<usize>,
}
```

## Usage in Rerun

### Available Exact Statistics

From chunk metadata, we can provide **exact** statistics for:

1. **Table-level:**
   - `num_rows`: Sum of `num_rows()` across all chunks
   - `total_byte_size`: Sum of `heap_size_bytes()` across all chunks

2. **Column-level (time columns):**
   - `min_value`: From `TimeColumn::time_range.min()`
   - `max_value`: From `TimeColumn::time_range.max()`
   - These are pre-computed and cached!

3. **Column-level (component columns):**
   - `null_count`: Can be computed from validity bitmaps
   - Min/max: Would require scanning actual data (skip for now)

### What to Return as Absent

- `distinct_count`: Not available without scanning
- `sum_value`: Not relevant for most Rerun data types
- Component min/max: Would require data scan

## Import Path

```rust
use datafusion::common::stats::{Precision, Statistics};
use datafusion::common::stats::ColumnStatistics;
use datafusion::scalar::ScalarValue;
```

Or alternatively:
```rust
use datafusion_common::stats::{Precision, Statistics, ColumnStatistics};
use datafusion_common::ScalarValue;
```

## Example Usage

```rust
use datafusion_common::stats::{Precision, Statistics, ColumnStatistics};
use datafusion_common::ScalarValue;

let stats = Statistics {
    num_rows: Precision::Exact(1000),
    total_byte_size: Precision::Exact(50_000),
    column_statistics: vec![
        ColumnStatistics {
            null_count: Precision::Exact(0),
            max_value: Precision::Exact(ScalarValue::Int64(Some(100))),
            min_value: Precision::Exact(ScalarValue::Int64(Some(1))),
            sum_value: Precision::Absent,
            distinct_count: Precision::Absent,
        },
    ],
};
```
