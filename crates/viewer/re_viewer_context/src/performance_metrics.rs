//! Global performance metrics for tracking bottlenecks across the viewer.
//!
//! These atomic counters are used by the performance panel to track various
//! operations that can become bottlenecks. They are reset at the beginning of
//! each frame by the performance panel.

use std::sync::atomic::AtomicU64;

// ============================================================================
// Bottleneck Metrics - Track operations that can slow down frame rendering
// ============================================================================

/// Number of times `AnnotationMap::load()` is called per frame.
///
/// **Target**: 1 per frame (should be cached)
pub static ANNOTATION_LOADS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);

/// Number of entity tree walks performed per frame.
///
/// **Target**: Minimize - ideally O(visible entities), not O(all entities)
pub static ENTITY_TREE_WALKS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);

/// Number of transform invalidations per frame.
///
/// **Target**: 0 when no data changes
pub static TRANSFORM_INVALIDATIONS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);

/// Number of blueprint tree rebuilds per frame.
///
/// **Target**: 0 when blueprint unchanged
pub static BLUEPRINT_TREE_REBUILDS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);

/// Number of query traversals per frame.
///
/// **Target**: Minimize through caching
pub static QUERY_TRAVERSALS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);

// ============================================================================
// Cache Statistics - Track cache hit/miss rates
// ============================================================================

/// Query cache hits this frame
pub static QUERY_CACHE_HITS: AtomicU64 = AtomicU64::new(0);

/// Query cache misses this frame
pub static QUERY_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

/// Transform cache hits this frame
pub static TRANSFORM_CACHE_HITS: AtomicU64 = AtomicU64::new(0);

/// Transform cache misses this frame
pub static TRANSFORM_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);

/// Blueprint tree cache hits this frame
pub static BLUEPRINT_TREE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);

/// Blueprint tree cache misses this frame
pub static BLUEPRINT_TREE_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
