use std::sync::Arc;

use re_chunk_store::{ChunkStoreEvent, LatestAtQuery};
use re_entity_db::EntityDb;
use re_types::archetypes;

use crate::{AnnotationMap, Cache, CacheMemoryReport, ViewerContext};

/// Cache for annotation maps to avoid redundant loading within a frame.
///
/// This cache stores a single annotation map per frame and is cleared at the beginning
/// of each frame. The first access in a frame loads the annotation map, and all subsequent
/// accesses reuse the cached version.
#[derive(Default)]
pub struct AnnotationMapCache {
    /// The cached annotation map for the current frame.
    /// None means it hasn't been loaded yet this frame.
    cached: Option<Arc<AnnotationMap>>,
}

impl AnnotationMapCache {
    /// Get or load the annotation map for the current frame.
    pub fn get(&mut self, ctx: &ViewerContext<'_>, query: &LatestAtQuery) -> Arc<AnnotationMap> {
        re_tracing::profile_function!();

        if let Some(cached) = &self.cached {
            // Already loaded this frame, return cached version
            cached.clone()
        } else {
            // First access this frame, load and cache
            let mut annotation_map = AnnotationMap::default();
            annotation_map.load(ctx, query);
            let arc = Arc::new(annotation_map);
            self.cached = Some(arc.clone());
            arc
        }
    }
}

impl Cache for AnnotationMapCache {
    fn begin_frame(&mut self) {
        re_tracing::profile_function!();
        // Clear the cache at the beginning of each frame to force a reload
        self.cached = None;
    }

    fn purge_memory(&mut self) {
        self.cached = None;
    }

    fn name(&self) -> &'static str {
        "Annotation Map Cache"
    }

    fn memory_report(&self) -> CacheMemoryReport {
        CacheMemoryReport {
            bytes_cpu: std::mem::size_of::<Self>() as u64,
            bytes_gpu: None,
            per_cache_item_info: Vec::new(),
        }
    }

    fn on_store_events(&mut self, events: &[&ChunkStoreEvent], _entity_db: &EntityDb) {
        re_tracing::profile_function!();

        // Invalidate cache if any annotation context was added or removed
        let has_annotation_context_changes = events.iter().any(|event| {
            event
                .chunk
                .components()
                .contains_key(&archetypes::AnnotationContext::descriptor_context().component)
        });

        if has_annotation_context_changes {
            self.cached = None;
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
