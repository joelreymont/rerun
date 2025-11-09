use std::collections::HashMap;
use std::sync::Arc;

use re_chunk_store::{ChunkStoreEvent, LatestAtQuery};
use re_entity_db::EntityDb;
use re_types::archetypes;

use crate::{AnnotationMap, Cache, CacheMemoryReport, ViewerContext};

/// Cache for annotation maps to avoid redundant loading within a frame.
///
/// This cache stores annotation maps per query (timeline + time) and is cleared at the
/// beginning of each frame. Multiple views querying at different times will each get
/// the correct annotations for their query.
#[derive(Default)]
pub struct AnnotationMapCache {
    /// Cached annotation maps, keyed by query.
    /// Cleared at the beginning of each frame.
    cached: HashMap<LatestAtQuery, Arc<AnnotationMap>>,
}

impl AnnotationMapCache {
    /// Get or load the annotation map for the given query.
    pub fn get(&mut self, ctx: &ViewerContext<'_>, query: &LatestAtQuery) -> Arc<AnnotationMap> {
        re_tracing::profile_function!();

        if let Some(cached) = self.cached.get(query) {
            // Already loaded for this query, return cached version
            cached.clone()
        } else {
            // First access for this query, load and cache
            let mut annotation_map = AnnotationMap::default();
            annotation_map.load(ctx, query);
            let arc = Arc::new(annotation_map);
            self.cached.insert(query.clone(), arc.clone());
            arc
        }
    }
}

impl Cache for AnnotationMapCache {
    fn begin_frame(&mut self) {
        re_tracing::profile_function!();
        // Clear all cached queries at the beginning of each frame
        self.cached.clear();
    }

    fn purge_memory(&mut self) {
        self.cached.clear();
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
            self.cached.clear();
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
