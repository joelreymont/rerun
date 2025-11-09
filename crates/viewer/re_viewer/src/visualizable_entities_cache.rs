//! Cache for visualizable entities determination
//!
//! Issue #8233: `determine_visualizable_entities()` is called every frame for every view,
//! which is expensive. This cache stores the results per view and only recomputes when
//! the view configuration or underlying data changes.

use std::collections::HashMap;

use re_chunk_store::ChunkStoreGeneration;
use re_entity_db::EntityDb;
use re_log_types::EntityPath;
use re_types::ViewClassIdentifier;
use re_viewer_context::{
    MaybeVisualizableEntities, PerVisualizer, ViewClassRegistry, ViewId, VisualizableEntities,
};
use re_viewport_blueprint::ViewBlueprint;

/// Cache key for visualizable entities determination.
///
/// The cache is valid as long as:
/// 1. The recording data hasn't changed (recording_generation)
/// 2. The view configuration is the same (space_origin, class_identifier)
/// 3. The maybe_visualizable entities haven't changed (subscriber_fingerprint)
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct VisualizableEntitiesCacheKey {
    /// Generation of the recording database
    recording_generation: ChunkStoreGeneration,

    /// The view's space origin
    space_origin: EntityPath,

    /// The view's class identifier
    class_identifier: ViewClassIdentifier,

    /// Fingerprint of maybe_visualizable_entities_per_visualizer
    /// This is a hash of which entities are maybe-visualizable for which visualizers
    subscriber_fingerprint: u64,
}

impl VisualizableEntitiesCacheKey {
    fn from_view(
        view: &ViewBlueprint,
        recording: &EntityDb,
        maybe_visualizable: &PerVisualizer<MaybeVisualizableEntities>,
    ) -> Self {
        // Create a fingerprint of the maybe_visualizable data
        // We use a hash of the entity sets to detect changes
        let subscriber_fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};

            let mut hasher = DefaultHasher::new();

            // Hash the visualizer IDs and all their entities
            // We must hash all entities to avoid collisions between different entity sets
            let mut visualizers: Vec<_> = maybe_visualizable.0.iter().collect();
            visualizers.sort_by_key(|(id, _)| *id);

            for (visualizer_id, entities) in visualizers {
                Hash::hash(visualizer_id, &mut hasher);
                entities.0.len().hash(&mut hasher);

                // Hash all entities for correctness
                let mut sorted_entities: Vec<_> = entities.0.iter().collect();
                sorted_entities.sort();
                for entity in sorted_entities {
                    Hash::hash(entity, &mut hasher);
                }
            }

            hasher.finish()
        };

        Self {
            recording_generation: recording.generation(),
            space_origin: view.space_origin.clone(),
            class_identifier: view.class_identifier(),
            subscriber_fingerprint,
        }
    }
}

/// Cached visualizable entities for a single view.
struct CachedVisualizableEntities {
    cache_key: VisualizableEntitiesCacheKey,
    visualizable_entities: PerVisualizer<VisualizableEntities>,
}

/// Cache for visualizable entities per view.
///
/// This cache eliminates redundant calls to `determine_visualizable_entities()`
/// which is expensive and was being called every frame for every view.
#[derive(Default)]
pub struct VisualizableEntitiesCache {
    cache: HashMap<ViewId, CachedVisualizableEntities>,
}

impl VisualizableEntitiesCache {
    /// Get or compute visualizable entities for a view.
    pub fn get_or_determine(
        &mut self,
        view: &ViewBlueprint,
        recording: &EntityDb,
        maybe_visualizable_entities: &PerVisualizer<MaybeVisualizableEntities>,
        view_class_registry: &ViewClassRegistry,
    ) -> PerVisualizer<VisualizableEntities> {
        re_tracing::profile_function!();

        let current_key = VisualizableEntitiesCacheKey::from_view(
            view,
            recording,
            maybe_visualizable_entities,
        );

        // Check if we have a valid cached entry
        if let Some(cached) = self.cache.get(&view.id) {
            if cached.cache_key == current_key {
                // Cache hit!
                re_tracing::profile_scope!("visualizable_entities_cache_hit");

                #[cfg(not(target_arch = "wasm32"))]
                re_viewer_context::performance_metrics::VISUALIZABLE_ENTITIES_CACHE_HITS
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                return PerVisualizer(cached.visualizable_entities.0.clone());
            }
        }

        // Cache miss - compute visualizable entities
        re_tracing::profile_scope!("visualizable_entities_cache_miss");

        #[cfg(not(target_arch = "wasm32"))]
        re_viewer_context::performance_metrics::VISUALIZABLE_ENTITIES_CACHE_MISSES
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        let view_class = view.class(view_class_registry);
        let visualizers = view_class_registry.new_visualizer_collection(view.class_identifier());

        let visualizable_entities = view_class.determine_visualizable_entities(
            maybe_visualizable_entities,
            recording,
            &visualizers,
            &view.space_origin,
        );

        // Update cache
        self.cache.insert(
            view.id,
            CachedVisualizableEntities {
                cache_key: current_key,
                visualizable_entities: PerVisualizer(visualizable_entities.0.clone()),
            },
        );

        visualizable_entities
    }

    /// Clear the entire cache.
    ///
    /// This can be useful for testing or when you know all views have changed.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Remove a specific view from the cache.
    ///
    /// Call this when a view is deleted.
    pub fn remove(&mut self, view_id: &ViewId) {
        self.cache.remove(view_id);
    }

    /// Get cache statistics for debugging/monitoring.
    pub fn stats(&self) -> VisualizableEntitiesCacheStats {
        VisualizableEntitiesCacheStats {
            cached_views: self.cache.len(),
        }
    }
}

/// Statistics about the visualizable entities cache.
#[derive(Debug, Clone, Copy)]
pub struct VisualizableEntitiesCacheStats {
    pub cached_views: usize,
}
