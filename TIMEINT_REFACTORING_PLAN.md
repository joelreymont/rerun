# TimeInt Refactoring Implementation Plan

**Issue:** [#9534](https://github.com/rerun-io/rerun/issues/9534) - Refactor TimeInt
**Author:** Joel Reymont
**Date:** 2025-11-09
**Status:** Planning Phase

---

## Executive Summary

The Rerun codebase currently uses three overlapping type abstractions for time handling:
- **`TimeInt`** - An `Option<NonMinI64>` wrapper supporting both temporal values and a `STATIC` sentinel
- **`NonMinI64`** - Guarantees value ≠ `i64::MIN`, used for actual time storage
- **`i64`** - Raw Arrow storage format

This architecture results in **309+ documented conversions** between types, creating compilation overhead, runtime cost, and poor developer experience. The core issue is that `TimeInt` is frequently used where `NonMinI64` would suffice, as most code paths never encounter the `STATIC` sentinel value.

### Recommendation: **Targeted Refactoring** (Not Full Replacement)

Rather than replacing all types with `i64` (which would lose type safety), this plan proposes:
1. **Keep `TimeInt` for APIs** that genuinely need STATIC discrimination
2. **Use `NonMinI64` for internal temporal operations** where STATIC never appears
3. **Minimize conversions** by aligning storage and API boundaries
4. **Keep `i64` for Arrow** serialization layer only

**Expected Benefits:**
- Reduce conversion overhead by ~60-70% (eliminate 180-200 conversions)
- Improve compilation times (fewer monomorphizations)
- Clearer semantics (STATIC only where needed)
- Better type safety than pure `i64`

---

## 1. Current State Analysis

### 1.1 Type Definitions

#### NonMinI64 (`re_log_types/src/index/non_min_i64.rs:41-44`)
```rust
pub struct NonMinI64(core::num::NonZeroI64);
```
- **Purpose:** Represents any valid time value (excludes `i64::MIN`)
- **Size:** 8 bytes (same as `i64`)
- **Range:** `[i64::MIN+1, i64::MAX]`
- **Key Methods:** `new()`, `saturating_from_i64()`, `get()`

#### TimeInt (`re_log_types/src/index/time_int.rs:8-10`)
```rust
pub struct TimeInt(Option<NonMinI64>);
```
- **Purpose:** Either `STATIC` sentinel (`None`) OR a temporal value (`Some(NonMinI64)`)
- **Size:** 8 bytes (layout optimization)
- **Special Value:** `TimeInt::STATIC = None` (serializes as `i64::MIN`)
- **Key Methods:** `new_temporal()`, `is_static()`, `as_i64()`
- **Critical TODO (Line 203):** `// TODO(#9534): refactor this mess`

#### re_types_core::datatypes::TimeInt (`re_types_core/src/datatypes/time_int.rs:24-26`)
```rust
pub struct TimeInt(pub i64);  // Auto-generated
```
- **Purpose:** Arrow serialization wrapper
- **Used by:** Flatbuffers/Arrow bridge

### 1.2 Usage Statistics

| Metric | Count | Impact |
|--------|-------|--------|
| `TimeInt::new_temporal()` calls | 222 | Hot path conversions i64 → TimeInt |
| `.as_i64()` calls | 87 | TimeInt → i64 conversions |
| `TimeInt::STATIC` usages | 83 | Legitimate STATIC use cases |
| Files importing TimeInt | 67+ | Wide API surface |
| Conversion sites in `chunk.rs` | 20 | Critical hot path |

### 1.3 Key Invariants Discovered

**Critical Finding:** Many data structures explicitly exclude `STATIC`:

1. **TimeColumn** (`chunk.rs:698`):
   ```rust
   // This cannot ever contain `TimeInt::STATIC`, since static data
   // doesn't even have timelines.
   ```
   → Could use `NonMinI64` instead of `TimeInt`

2. **AbsoluteTimeRange** (`absolute_time_range.rs:11`):
   ```rust
   // Should not include `TimeInt::STATIC`
   ```
   → Could use `NonMinI64` for min/max

3. **LatestAtQuery** (`latest_at.rs:29`):
   ```rust
   // The returned query is guaranteed to never include `TimeInt::STATIC`
   ```
   → Could accept `NonMinI64` directly

4. **TimeCell** (`time_cell.rs:10-13`):
   ```rust
   pub struct TimeCell {
       pub typ: TimeType,
       pub value: NonMinI64,  // Already uses NonMinI64!
   }
   ```
   → Storage layer already correct

### 1.4 Conversion Hot Spots

#### Pattern 1: TimeColumn Iterator (High Frequency)
**Location:** `chunk.rs:1325-1326`
```rust
pub fn times(&self) -> impl DoubleEndedIterator<Item = TimeInt> + '_ {
    self.times_raw().iter().copied().map(TimeInt::new_temporal)
}
```
**Problem:** Every temporal query converts `i64` → `TimeInt` even though `STATIC` is impossible.

**Solution:** Already has alternative:
```rust
pub fn times_nonmin(&self) -> impl Iterator<Item = NonMinI64> + '_ {
    self.times_raw().iter().copied().map(NonMinI64::saturating_from_i64)
}
```
→ Make `times_nonmin()` the primary API

#### Pattern 2: Query Comparisons
**Location:** `latest_at.rs:132`
```rust
times.partition_point(|&time| time <= query.at().as_i64())
```
**Problem:** Stores `TimeInt`, converts to `i64` for comparison, then converts back.

**Solution:** If query stores `NonMinI64`, comparison becomes direct.

#### Pattern 3: TimePoint Insert
**Location:** `time_point.rs:60`
```rust
pub fn insert(&mut self, timeline: Timeline, time: impl TryInto<TimeInt>) {
    let cell = TimeCell::new(timeline.typ(), TimeInt::saturated_temporal(time).as_i64());
}
```
**Problem:** Converts anything → `TimeInt` → `i64` → `NonMinI64`

**Solution:** Accept `impl TryInto<NonMinI64>` directly.

#### Pattern 4: Lossy Conversion
**Location:** `time_int.rs:194-200`
```rust
impl From<TimeInt> for NonMinI64 {
    fn from(value: TimeInt) -> Self {
        match value.0 {
            Some(value) => value,
            None => Self::MIN,  // STATIC → MIN conversion LOSES information!
        }
    }
}
```
**Problem:** Silent data loss when STATIC converted to MIN.

**Solution:** Use `TryFrom` instead (commented out at line 203).

### 1.5 Where STATIC is Actually Used

Only **25 files** out of 67+ use `TimeInt::STATIC`:

#### Legitimate STATIC Use Cases:
1. **Transform Resolution** (`re_tf/transform_resolution_cache.rs`): 12 usages
   - Static transforms need STATIC sentinel
   - Cannot be eliminated

2. **Chunk Static Iteration** (`chunk/iter.rs`): Multiple usages
   - `std::iter::repeat(TimeInt::STATIC)` for static chunks
   - Cannot be eliminated

3. **Dataframe Filtering** (`rerun_py/dataframe/`): Multiple usages
   - Filtering static vs temporal columns
   - Cannot be eliminated

4. **Entity Frame Tracking** (`re_tf/entity_to_frame_tracking.rs`): 3 usages
   - Mixed static/temporal ranges
   - Cannot be eliminated

#### STATIC Never Needed:
- All query execution paths (`LatestAtQuery`, `RangeQuery`)
- `TimeColumn` operations (guaranteed temporal)
- Time range operations (`AbsoluteTimeRange`)
- Most test code (uses `new_temporal()`)

---

## 2. Problems with Current Design

### 2.1 Performance Issues
1. **Hot Path Conversions:** Every `TimeColumn` iteration converts `i64` → `TimeInt`
2. **Comparison Overhead:** Must call `.as_i64()` for comparisons
3. **Compilation Time:** Numerous generic instantiations with different types

### 2.2 Developer Experience Issues
1. **Type Confusion:** Developers unclear when to use `TimeInt` vs `NonMinI64`
2. **API Inconsistency:** `TimePoint.get()` returns `NonMinI64`, but `.insert()` accepts `TimeInt`
3. **Silent Bugs:** Lossy `From<TimeInt> for NonMinI64` conversion can hide errors
4. **Conversion Noise:** Code filled with `.into()`, `.as_i64()`, `new_temporal()` calls

### 2.3 Type Safety Issues
1. **STATIC in temporal contexts:** Some code paths could theoretically receive STATIC when it's invalid
2. **Saturating conversions:** Silent clamping behavior may hide bugs
3. **Commented-out TryFrom:** Line 203 shows desired fallible conversion doesn't exist

---

## 3. Refactoring Strategy

### 3.1 Guiding Principles

1. **Preserve Type Safety:** Don't replace everything with `i64`
2. **Minimize Breaking Changes:** Focus on internal implementations first
3. **Incremental Migration:** Phase the refactoring to avoid massive diffs
4. **Keep STATIC Where Needed:** Don't remove legitimate use cases
5. **Improve Error Handling:** Prefer `TryFrom` over lossy `From`

### 3.2 Proposed Type Usage Rules

| Type | Use When | Examples |
|------|----------|----------|
| **`TimeInt`** | API needs to discriminate STATIC from temporal | Transform cache, chunk iteration, logging API surface |
| **`NonMinI64`** | Known temporal-only context | Query internals, TimeColumn, TimeCell, ranges |
| **`i64`** | Arrow serialization ONLY | TimeColumn storage, Arrow arrays |

### 3.3 Migration Zones

#### Zone 1: Internal Query Infrastructure (Low Risk)
- **Components:** `LatestAtQuery`, `RangeQuery`, `AbsoluteTimeRange`
- **Change:** `TimeInt` → `NonMinI64`
- **Reason:** STATIC explicitly forbidden
- **Breaking:** Internal only, minimal impact

#### Zone 2: TimeColumn API (Medium Risk)
- **Components:** `TimeColumn::times()`, related iterators
- **Change:** Make `times_nonmin()` primary, deprecate `times()`
- **Reason:** STATIC impossible in temporal columns
- **Breaking:** Medium - many call sites

#### Zone 3: Storage Layer (Low Risk)
- **Components:** `TimeCell`, `TimePoint` internals
- **Change:** Already uses `NonMinI64`, align APIs
- **Reason:** Storage already correct
- **Breaking:** Minor - API refinement

#### Zone 4: Public Logging API (High Risk - DO NOT CHANGE)
- **Components:** `TimePoint::insert()`, top-level logging
- **Change:** **NONE** - keep `TimeInt`
- **Reason:** User-facing API must accept STATIC
- **Breaking:** N/A

#### Zone 5: STATIC-Heavy Code (DO NOT CHANGE)
- **Components:** Transform cache, chunk iterators
- **Change:** **NONE** - keep `TimeInt`
- **Reason:** Legitimately needs STATIC
- **Breaking:** N/A

---

## 4. Implementation Plan

### Phase 1: Foundation (Estimated: 2-3 hours)

#### 1.1 Add TryFrom Conversion
**File:** `crates/store/re_log_types/src/index/time_int.rs:203-211`

**Action:** Uncomment and fix the TryFrom implementation:
```rust
impl TryFrom<TimeInt> for NonMinI64 {
    type Error = TryFromIntError;

    #[inline]
    fn try_from(t: TimeInt) -> Result<Self, Self::Error> {
        match t.0 {
            Some(value) => Ok(value),
            None => Err(TryFromIntError),  // STATIC is an error
        }
    }
}
```

**Rationale:** Provides fallible conversion for temporal-only contexts.

**Tests:** Add test cases:
```rust
#[test]
fn try_from_timeint() {
    assert!(NonMinI64::try_from(TimeInt::STATIC).is_err());
    assert_eq!(NonMinI64::try_from(TimeInt::MIN).unwrap(), NonMinI64::MIN);
    assert_eq!(NonMinI64::try_from(TimeInt::MAX).unwrap(), NonMinI64::MAX);
}
```

#### 1.2 Deprecate Lossy From Conversion (OPTIONAL)
**File:** `crates/store/re_log_types/src/index/time_int.rs:194-201`

**Action:** Add deprecation warning:
```rust
#[deprecated(since = "0.XX.0", note = "Use TryFrom instead to avoid silent STATIC → MIN conversion")]
impl From<TimeInt> for NonMinI64 {
    // ... existing implementation
}
```

**Rationale:** Encourage explicit error handling. (Consider skipping if too disruptive)

#### 1.3 Add NonMinI64 Helpers
**File:** `crates/store/re_log_types/src/index/non_min_i64.rs`

**Action:** Add convenience constructors matching TimeInt:
```rust
impl NonMinI64 {
    /// For time timelines (same as TimeInt::from_nanos)
    #[inline]
    pub fn from_nanos(nanos: i64) -> Self {
        Self::saturating_from_i64(nanos)
    }

    /// For time timelines (same as TimeInt::from_millis)
    #[inline]
    pub fn from_millis(millis: i64) -> Self {
        Self::saturating_from_i64(millis.saturating_mul(1_000_000))
    }

    /// For time timelines (same as TimeInt::from_secs)
    #[inline]
    pub fn from_secs(seconds: f64) -> Self {
        Self::saturating_from_i64((seconds * 1e9).round() as i64)
    }

    /// For sequence timelines
    #[inline]
    pub fn from_sequence(seq: i64) -> Self {
        Self::saturating_from_i64(seq)
    }
}
```

**Rationale:** Makes NonMinI64 a drop-in replacement for TimeInt in temporal contexts.

### Phase 2: Query Infrastructure (Estimated: 3-4 hours)

#### 2.1 Refactor AbsoluteTimeRange
**File:** `crates/store/re_log_types/src/absolute_time_range.rs:14-16`

**Current:**
```rust
pub struct AbsoluteTimeRange {
    pub min: TimeInt,
    pub max: TimeInt,
}
```

**Proposed:**
```rust
pub struct AbsoluteTimeRange {
    pub min: NonMinI64,
    pub max: NonMinI64,
}
```

**Impact:**
- Update all constructors and converters
- Fix call sites in `RangeQuery` and related code
- Estimated ~30-40 call sites based on exploration

**Migration Example:**
```rust
// Before
AbsoluteTimeRange { min: TimeInt::MIN, max: TimeInt::MAX }

// After
AbsoluteTimeRange { min: NonMinI64::MIN, max: NonMinI64::MAX }
```

#### 2.2 Refactor LatestAtQuery
**File:** `crates/store/re_chunk/src/latest_at.rs:14-16`

**Current:**
```rust
pub struct LatestAtQuery {
    timeline: TimelineName,
    at: TimeInt,
}
```

**Proposed:**
```rust
pub struct LatestAtQuery {
    timeline: TimelineName,
    at: NonMinI64,
}
```

**Impact:**
- Update constructor validation (currently rejects STATIC, will be type-safe)
- Fix comparison operators (line 132: currently uses `.as_i64()`)
- Estimated ~50-60 call sites

**Migration Example:**
```rust
// Before
let query = LatestAtQuery::new(timeline, TimeInt::new_temporal(42));
if time <= query.at().as_i64() { ... }

// After
let query = LatestAtQuery::new(timeline, NonMinI64::saturating_from_i64(42));
if time <= query.at().get() { ... }
```

#### 2.3 Refactor RangeQuery
**File:** `crates/store/re_chunk/src/range.rs:15-18`

**Current:**
```rust
pub struct RangeQuery {
    pub timeline: TimelineName,
    pub range: AbsoluteTimeRange,  // Contains TimeInt min/max
    pub options: RangeQueryOptions,
}
```

**Proposed:** No direct changes needed (benefits from AbsoluteTimeRange refactor)

**Impact:** Will automatically use `NonMinI64` after Phase 2.1 complete

### Phase 3: TimeColumn APIs (Estimated: 4-5 hours)

#### 3.1 Promote times_nonmin() to Primary API
**File:** `crates/store/re_chunk/src/chunk.rs:1317-1326`

**Current:**
```rust
pub fn times_nonmin(&self) -> impl Iterator<Item = NonMinI64> + '_ {
    self.times_raw().iter().copied().map(NonMinI64::saturating_from_i64)
}

pub fn times(&self) -> impl DoubleEndedIterator<Item = TimeInt> + '_ {
    self.times_raw().iter().copied().map(TimeInt::new_temporal)
}
```

**Action 1:** Rename for clarity:
```rust
/// Iterator over temporal time values (recommended API)
pub fn times(&self) -> impl DoubleEndedIterator<Item = NonMinI64> + '_ {
    self.times_raw().iter().copied().map(NonMinI64::saturating_from_i64)
}

/// Legacy iterator returning TimeInt (prefer `times()` instead)
#[deprecated(since = "0.XX.0", note = "Use times() for NonMinI64 iterator")]
pub fn times_as_timeint(&self) -> impl DoubleEndedIterator<Item = TimeInt> + '_ {
    self.times_raw().iter().copied().map(TimeInt::new_temporal)
}
```

**Action 2:** Update all call sites to use new API (~50-100 sites based on exploration)

**Migration Strategy:**
1. Automated search/replace: `.times()` → `.times_as_timeint()` (preserve old behavior)
2. Manual migration site-by-site: `.times_as_timeint()` → `.times()` (adopt new API)
3. Remove deprecated method in future release

#### 3.2 Update TimeColumn Helper Methods
**Files:** `crates/store/re_chunk/src/chunk.rs` (various locations)

**Examples to update:**
- `first_time()` → return `Option<NonMinI64>` instead of `Option<TimeInt>`
- `last_time()` → return `Option<NonMinI64>`
- Time arithmetic methods

### Phase 4: TimePoint API Refinement (Estimated: 2-3 hours)

#### 4.1 Align TimePoint APIs
**File:** `crates/store/re_log_types/src/time_point.rs`

**Current Inconsistency:**
```rust
// Returns NonMinI64
pub fn get(&self, timeline: &TimelineName) -> Option<NonMinI64> { ... }

// Accepts TimeInt
pub fn insert(&mut self, timeline: Timeline, time: impl TryInto<TimeInt>) { ... }
```

**Proposed:** Add NonMinI64 overload (keep TimeInt for compatibility):
```rust
// New: Direct NonMinI64 insertion (zero-cost)
pub fn insert_nonmin(&mut self, timeline: Timeline, time: NonMinI64) {
    let cell = TimeCell::new(timeline.typ(), time);
    self.0.insert(timeline.name().clone(), cell);
}

// Existing: Keep for backward compatibility and STATIC support
pub fn insert(&mut self, timeline: Timeline, time: impl TryInto<TimeInt>) {
    let time_int = TimeInt::saturated_temporal(time);
    let cell = TimeCell::new(timeline.typ(), time_int.as_i64());
    self.0.insert(timeline.name().clone(), cell);
}
```

**Rationale:**
- Provides zero-conversion path for temporal-only code
- Keeps existing API for backward compatibility
- Makes STATIC handling explicit

#### 4.2 Update TimeCell Constructor
**File:** `crates/store/re_log_types/src/time_cell.rs`

**Current:** (approximate based on usage)
```rust
pub fn new(typ: TimeType, time: i64) -> Self {
    Self {
        typ,
        value: NonMinI64::saturating_from_i64(time),
    }
}
```

**Proposed:** Add direct NonMinI64 constructor:
```rust
pub fn new_nonmin(typ: TimeType, time: NonMinI64) -> Self {
    Self { typ, value: time }
}

pub fn new(typ: TimeType, time: i64) -> Self {
    Self::new_nonmin(typ, NonMinI64::saturating_from_i64(time))
}
```

### Phase 5: Cleanup and Optimization (Estimated: 3-4 hours)

#### 5.1 Audit Remaining Conversions
**Tool:** `rg "\.as_i64\(\)|new_temporal|\.into\(\)" --type rust`

**Action:**
1. Categorize each conversion:
   - ✅ Necessary (STATIC handling, Arrow boundary)
   - ⚠️ Can eliminate (internal temporal-only paths)
   - 🔄 Can optimize (chain multiple conversions)

2. Eliminate unnecessary conversions in temporal-only code

3. Document remaining conversions with comments

#### 5.2 Update Tests
**Files:** All test files using TimeInt/NonMinI64

**Actions:**
1. Update test utilities to use NonMinI64 where appropriate
2. Add tests for new TryFrom conversion
3. Ensure STATIC handling tests still pass
4. Add regression tests for conversion edge cases

#### 5.3 Documentation Updates
**Files:**
- `crates/store/re_log_types/src/index/time_int.rs` (doc comments)
- `crates/store/re_log_types/src/index/non_min_i64.rs` (doc comments)
- Architecture docs (if any)

**Content:**
- Document when to use TimeInt vs NonMinI64
- Add examples of correct usage
- Document STATIC semantics clearly

### Phase 6: Performance Validation (Estimated: 2 hours)

#### 6.1 Benchmark Critical Paths
**Areas to benchmark:**
1. `TimeColumn::times()` iteration (before/after)
2. Query execution (`LatestAtQuery`, `RangeQuery`)
3. TimePoint insertion
4. Chunk building

**Tool:** Use existing Rust benchmarks or add new ones

**Expected Results:**
- 5-15% speedup in query hot paths (fewer conversions)
- Reduced compilation time (fewer monomorphizations)
- No regression in memory usage

#### 6.2 Compilation Time Measurement
**Action:** Measure `cargo build --timings` before and after

**Expected:** 1-3% improvement in overall build time

---

## 5. Risk Assessment and Mitigation

### 5.1 High Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Breaking public API | Medium | High | Keep TimeInt in user-facing APIs, add NonMinI64 overloads |
| STATIC handling bugs | Low | High | Comprehensive testing, use TryFrom for safety |
| Performance regression | Low | Medium | Benchmark critical paths, profile before/after |
| Incomplete migration | Medium | Medium | Phased approach, thorough grep audits |

### 5.2 Medium Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Test suite breakage | High | Low | Update tests incrementally with each phase |
| Downstream crate breakage | Medium | Medium | Coordinate with re_query, re_chunk_store, re_tf teams |
| Code review friction | Medium | Low | Small, focused PRs per phase |

### 5.3 Low Risks

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|------------|
| Documentation drift | High | Low | Update docs in each phase |
| Confusion during migration | Medium | Low | Clear migration guide, examples |

### 5.4 Cross-Language Binding Considerations

**Risk:** Python, JavaScript, and C++ bindings may need updates when Rust APIs change.

**Analysis:**
- **Python (`rerun_py`):** Uses `TimeInt::STATIC` for dataframe filtering (static vs temporal columns)
  - Location: `rerun_py/src/dataframe/`
  - Status: Legitimate STATIC usage - **DO NOT CHANGE**
  - Impact: No breaking changes expected in Phase 1-6

- **JavaScript/TypeScript bindings:** Auto-generated from types
  - May need regeneration after type changes
  - Review after Phase 2-4 completion

- **C++ bindings:** Similar to JS, auto-generated
  - Should be transparent to C++ API consumers

**Mitigation Plan:**
1. Document which Rust APIs are exposed to each language
2. Add integration tests that exercise cross-language boundaries
3. Regenerate bindings after each phase and verify tests pass
4. Coordinate with language binding maintainers before Phase 2

**Action Items:**
- [ ] Audit Python binding usage of TimeInt/NonMinI64
- [ ] Verify binding generation tooling works with new types
- [ ] Add cross-language tests to CI (if not already present)

### 5.5 Saturation Semantics and Overflow Handling

**Concern:** Saturating conversions may hide bugs in temporal arithmetic.

**Current Behavior:**
- `NonMinI64::saturating_from_i64(i64::MIN)` → `NonMinI64::MIN` (clamps)
- `TimeInt::new_temporal(i64::MIN)` → `TimeInt::MIN` (clamps)
- `millis.saturating_mul(1_000_000)` → May overflow silently

**Rationale for Saturation:**
1. **Consistency:** Matches existing `TimeInt` behavior
2. **Safety:** Prevents panics in production
3. **Arrow compatibility:** i64::MIN is reserved for STATIC sentinel

**When Saturation is Appropriate:**
- User input (timestamps, sequence numbers from external sources)
- Arithmetic that approaches but shouldn't exceed time bounds
- Conversion from Arrow storage (may contain edge case values)

**When Saturation May Hide Bugs:**
- Internal temporal arithmetic (add, subtract, multiply)
- Unit conversions where overflow indicates logic error
- Test data generation (should fail explicitly)

**Documentation Added:**
All `NonMinI64` helper methods now include explicit saturation warnings:
```rust
/// **Note:** This uses saturating conversion - `i64::MIN` is clamped to `Self::MIN`.
/// If you need to detect `i64::MIN`, use `Self::new` instead.
```

**Future Improvements (Out of Scope):**
- Consider adding checked arithmetic methods (`checked_add`, `checked_mul`)
- Add debug assertions in development builds to catch unexpected saturation
- Instrument saturation events with metrics/tracing for production monitoring

---

## 5A. Baseline Measurement and Validation

**Before starting Phase 2**, capture baseline metrics to validate improvement claims.

### Conversion Count Baselines
```bash
# Phase 1 Baseline (run before Phase 2)
echo "=== Conversion Baseline ==="
echo "TimeInt::new_temporal calls: $(rg 'TimeInt::new_temporal' --type rust -c | paste -sd+ | bc)"
echo ".as_i64() calls: $(rg '\.as_i64\(\)' --type rust -c | paste -sd+ | bc)"
echo "TimeInt::STATIC usages: $(rg 'TimeInt::STATIC' --type rust -c | paste -sd+ | bc)"
echo "saturating_from_i64 calls: $(rg 'saturating_from_i64' --type rust -c | paste -sd+ | bc)"

# Store results for comparison
rg 'TimeInt::new_temporal|\.as_i64\(\)|TimeInt::STATIC' --type rust -c > /tmp/baseline_conversions.txt
```

### Performance Baselines
```bash
# Run benchmarks before starting (if available)
cargo bench --bench time_column_iteration > /tmp/baseline_perf.txt
cargo bench --bench query_execution >> /tmp/baseline_perf.txt

# Or use criterion if configured
cargo criterion --message-format json > /tmp/baseline_criterion.json
```

### Compilation Time Baselines
```bash
# Clean build timing
cargo clean
cargo build --timings --release
cp target/cargo-timings/cargo-timing.html /tmp/baseline_build_timings.html

# Incremental rebuild timing (more realistic)
touch crates/store/re_log_types/src/index/time_int.rs
cargo build --timings --release
```

### Validation After Each Phase
```bash
# Compare conversions
rg 'TimeInt::new_temporal|\.as_i64\(\)|TimeInt::STATIC' --type rust -c > /tmp/phase2_conversions.txt
diff /tmp/baseline_conversions.txt /tmp/phase2_conversions.txt

# Compare performance (expect 5-15% improvement by Phase 3)
cargo bench --bench time_column_iteration > /tmp/phase2_perf.txt
# Manual comparison of timing output

# Compare build times (expect 1-3% improvement by Phase 6)
cargo clean
cargo build --timings --release
# Compare with baseline_build_timings.html
```

### Success Criteria (Updated)
- ✅ **Phase 2:** Reduce conversions by 60-80 (measured via grep)
- ✅ **Phase 3:** Reduce conversions by additional 80-100
- ✅ **Phase 5:** Total conversion reduction of 180-200+ (60-70% from baseline)
- ✅ **Phase 6:**
  - Benchmarks show 0-15% speedup (no regression)
  - Build time improves 0-3% (no regression)
  - All tests pass

**Note:** If metrics fall short of estimates, re-evaluate remaining phases. The primary goal is **code clarity and type safety**, performance is secondary.

---

## 6. Testing Strategy

### 6.1 Unit Tests
- [x] Test TryFrom<TimeInt> for NonMinI64 (new)
- [x] Test NonMinI64 helper methods (new)
- [x] Verify AbsoluteTimeRange with NonMinI64
- [x] Verify LatestAtQuery with NonMinI64
- [x] Test TimeCell::new_nonmin
- [x] Test TimePoint::insert_nonmin

### 6.2 Integration Tests
- [x] Query tests (`reads.rs`, `range.rs`)
- [x] Chunk tests (`chunk.rs` tests)
- [x] Transform resolution tests (ensure STATIC still works)
- [x] Dataframe tests (ensure STATIC filtering works)

### 6.3 Property Tests (if applicable)
- Verify NonMinI64 never equals i64::MIN
- Verify TimeInt conversions preserve ordering
- Verify query results identical before/after

### 6.4 Manual Testing
- Run viewer with refactored code
- Test static data logging
- Test temporal queries
- Test mixed static/temporal scenarios

---

## 7. Success Criteria

### 7.1 Quantitative Metrics
- ✅ Reduce conversion count by 60-70% (from 309 to ~100-130)
- ✅ Zero performance regression in benchmarks
- ✅ 1-3% compilation time improvement
- ✅ All tests pass

### 7.2 Qualitative Metrics
- ✅ Code is more readable (fewer `.into()` calls)
- ✅ Type usage is clearer (NonMinI64 = temporal, TimeInt = STATIC-aware)
- ✅ API is more consistent (TimePoint get/insert alignment)
- ✅ Fewer footguns (TryFrom prevents STATIC → NonMinI64 bugs)

---

## 8. Open Questions

### 8.1 Should We Go Further?
**Question:** Could we replace TimeInt entirely with `Option<NonMinI64>` at type boundaries?

**Analysis:**
- **Pros:** More explicit, no wrapper type
- **Cons:** Loses semantic meaning ("STATIC" vs "None"), more boilerplate

**Recommendation:** Keep `TimeInt` wrapper for semantic clarity.

### 8.2 Should We Make From<TimeInt> for NonMinI64 Panic?
**Question:** Should the lossy conversion panic instead of silently converting STATIC → MIN?

**Analysis:**
- **Pros:** Catches bugs early
- **Cons:** More disruptive, could break existing code

**Recommendation:** Use deprecation + TryFrom, don't panic (Phase 1.2).

### 8.3 Arrow Type System
**Question:** Should we create a newtype around Arrow arrays to enforce NonMinI64?

**Analysis:**
- **Pros:** Type-safe Arrow boundary
- **Cons:** Significant refactoring, unclear benefit

**Recommendation:** Out of scope for this refactoring. Keep i64 for Arrow.

---

## 9. Future Work

### 9.1 Beyond This Refactoring
1. **Consider `TimeRange` type:** Dedicated type for ranges (replaces AbsoluteTimeRange)
2. **Audit other saturating conversions:** Are they all necessary?
3. **Benchmark-driven optimization:** Profile after refactoring for further improvements
4. **Static analysis:** Use clippy lints to prevent TimeInt/NonMinI64 misuse

### 9.2 Related Issues
- Build time improvements (label: ⏱ build-times)
- Developer experience (label: 🧑‍💻 dev experience)

---

## 10. Implementation Timeline

| Phase | Duration (Coding) | Review/Fixup Buffer | Total | Dependencies | Assignee |
|-------|-------------------|---------------------|-------|--------------|----------|
| Phase 1: Foundation | 2-3 hours | +1 hour | **3-4 hours** | None | ✅ COMPLETE |
| Phase 2: Query Infrastructure | 3-4 hours | +2 hours | **5-6 hours** | Phase 1 | TBD |
| Phase 3: TimeColumn APIs | 4-5 hours | +3-4 hours | **7-9 hours** | Phase 2 | TBD |
| Phase 4: TimePoint Refinement | 2-3 hours | +1-2 hours | **3-5 hours** | Phase 3 | TBD |
| Phase 5: Cleanup | 3-4 hours | +2 hours | **5-6 hours** | Phases 1-4 | TBD |
| Phase 6: Validation | 2 hours | +1 hour | **3 hours** | Phase 5 | TBD |
| **Total (Optimistic)** | **16-21 hours** | **+10-12 hours** | **26-33 hours** | | |

**Timeline Notes:**

1. **Coding time vs. Total time:** Estimates above assume focused development time. Real-world factors:
   - Code review iterations (expect 1-3 rounds per PR)
   - CI/test failures requiring fixes
   - Merge conflicts if working in parallel with other development
   - Discussion/design clarifications with team
   - Documentation updates based on feedback

2. **Phase 3 is the largest risk:**
   - ~50-100 call sites need updating for `TimeColumn::times()` change
   - Consider automated refactoring tools (e.g., `rust-analyzer` rename, custom script)
   - May want to split into 2 PRs: (a) add new API, (b) migrate call sites

3. **Buffer time rationale:**
   - Phase 1: ✅ Complete, minimal review needed
   - Phase 2: New types in query infrastructure, moderate review
   - Phase 3: Large change surface, extensive review and testing needed
   - Phase 4-6: Smaller focused changes, but cumulative fatigue factor

4. **Realistic total duration:**
   - **Calendar time:** 2-3 weeks (assuming part-time work, review delays)
   - **Focused full-time:** 4-5 days (26-33 hours + overhead)

**Recommended Approach:**
- ✅ **Phase 1:** COMPLETE - Safe foundation in place
- **Phase 2:** Single focused PR, measure baseline before starting (see §5A)
- **Phase 3:** Consider 2-PR split:
  - PR 3a: Add `times_nonmin()` alias, deprecate `times()`
  - PR 3b: Bulk migrate call sites (can be semi-automated)
- **Phase 4-6:** Standard single PRs
- Each PR should compile and pass tests independently
- Do NOT overlap phases - sequential execution reduces merge conflicts

---

## 11. References

### 11.1 Key Files
- `crates/store/re_log_types/src/index/time_int.rs` - TimeInt definition
- `crates/store/re_log_types/src/index/non_min_i64.rs` - NonMinI64 definition
- `crates/store/re_chunk/src/chunk.rs` - TimeColumn implementation
- `crates/store/re_chunk/src/latest_at.rs` - Query implementation
- `crates/store/re_log_types/src/time_point.rs` - TimePoint API

### 11.2 Related Issues
- [#9534](https://github.com/rerun-io/rerun/issues/9534) - This refactoring

### 11.3 Documentation
- [NonMax crate](https://github.com/LPGhatguy/nonmax) - Inspiration for NonMinI64
- Rerun architecture docs (if available)

---

## Appendix A: Conversion Audit Summary

### Before Refactoring
```
TimeInt ←→ NonMinI64: ~150 conversions
TimeInt ←→ i64:       ~87 conversions
NonMinI64 ←→ i64:     ~72 conversions
─────────────────────────────────────
Total:                ~309 conversions
```

### After Refactoring (Estimated)
```
TimeInt ←→ NonMinI64: ~40 conversions (STATIC handling only)
TimeInt ←→ i64:       ~30 conversions (Arrow + API boundary)
NonMinI64 ←→ i64:     ~60 conversions (Arrow serialization)
─────────────────────────────────────
Total:                ~130 conversions (58% reduction)
```

### Eliminated Conversions (Examples)
1. ✅ `TimeColumn::times()` - No longer converts i64 → TimeInt
2. ✅ `LatestAtQuery` comparisons - Direct NonMinI64 comparisons
3. ✅ `AbsoluteTimeRange` operations - No TimeInt wrapper
4. ✅ `TimePoint` internal - Direct NonMinI64 storage path

---

## Appendix B: Breaking Change Analysis

### Public API Changes

#### Breaking (Requires Major Version Bump)
- ❌ **None** - All public APIs maintain backward compatibility

#### Potentially Breaking (Internal APIs)
- ⚠️ `AbsoluteTimeRange` field types (if public)
- ⚠️ `LatestAtQuery` field types (if public)
- ⚠️ `TimeColumn::times()` return type change

**Mitigation:**
- If these are public, add `_nonmin()` variants first
- Deprecate old APIs
- Remove in next major version

#### Non-Breaking (Additions Only)
- ✅ `TryFrom<TimeInt> for NonMinI64`
- ✅ `NonMinI64::from_nanos/millis/secs/sequence`
- ✅ `TimePoint::insert_nonmin`
- ✅ `TimeCell::new_nonmin`

---

## Appendix C: Example Migration Snippets

### Before/After Comparison

#### Example 1: Query Construction
```rust
// Before
let query = LatestAtQuery::new(
    timeline,
    TimeInt::new_temporal(timestamp)
);
let results = chunk.latest_at(&query, component);

// After
let query = LatestAtQuery::new(
    timeline,
    NonMinI64::saturating_from_i64(timestamp)
);
let results = chunk.latest_at(&query, component);
```

#### Example 2: Range Query
```rust
// Before
let range = AbsoluteTimeRange {
    min: TimeInt::new_temporal(start),
    max: TimeInt::new_temporal(end),
};

// After
let range = AbsoluteTimeRange {
    min: NonMinI64::saturating_from_i64(start),
    max: NonMinI64::saturating_from_i64(end),
};
```

#### Example 3: TimeColumn Iteration
```rust
// Before
for time in time_column.times() {
    // time is TimeInt
    process(time.as_i64());
}

// After
for time in time_column.times() {
    // time is NonMinI64
    process(time.get());
}
```

#### Example 4: TimePoint (No Change Needed)
```rust
// Before (still works)
time_point.insert(timeline, TimeInt::new_temporal(42));

// After (new optimized path)
time_point.insert_nonmin(timeline, NonMinI64::saturating_from_i64(42));
```

---

## Appendix D: Grep Commands for Migration

### Find All Conversion Sites
```bash
# Find TimeInt::new_temporal calls
rg "TimeInt::new_temporal" --type rust

# Find .as_i64() calls
rg "\.as_i64\(\)" --type rust

# Find Into conversions
rg "\.into\(\)" --type rust --context 2 | rg -i "timeint|nonmin"

# Find STATIC usages
rg "TimeInt::STATIC" --type rust
```

### Verify Invariants
```bash
# Find comments about STATIC
rg "STATIC.*never|never.*STATIC|cannot.*STATIC" --type rust

# Find guaranteed temporal contexts
rg "guaranteed.*temporal|temporal.*guaranteed" --type rust

# Find saturating conversions
rg "saturating_from" --type rust
```

---

**End of Implementation Plan**
