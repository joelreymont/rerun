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
#[cfg(test)]
use re_viewer_context::ViewSystemIdentifier;
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
        let subscriber_fingerprint = Self::subscriber_fingerprint(maybe_visualizable);

        Self {
            recording_generation: recording.generation(),
            space_origin: view.space_origin.clone(),
            class_identifier: view.class_identifier(),
            subscriber_fingerprint,
        }
    }

    /// Computes a fingerprint of the maybe-visualizable entities per visualizer.
    ///
    /// The fingerprint accounts for both which visualizers are present and the
    /// exact set of entities associated with each of them.
    fn subscriber_fingerprint(
        maybe_visualizable: &PerVisualizer<MaybeVisualizableEntities>,
    ) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();

        // Hash the visualizer IDs deterministically.
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

        let current_key =
            VisualizableEntitiesCacheKey::from_view(view, recording, maybe_visualizable_entities);

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

#[cfg(test)]
mod tests {
    use super::*;
    use re_log_types::EntityPath;

    fn per_visualizer_with_entities(
        visualizer: &str,
        entities: &[&str],
    ) -> PerVisualizer<MaybeVisualizableEntities> {
        let mut per_visualizer = PerVisualizer::default();
        let mut maybe_entities = MaybeVisualizableEntities::default();

        for entity in entities {
            maybe_entities.0.insert(EntityPath::from(*entity));
        }

        per_visualizer
            .0
            .insert(ViewSystemIdentifier::from(visualizer), maybe_entities);

        per_visualizer
    }

    #[test]
    fn fingerprint_differs_for_different_entities() {
        let set_a = per_visualizer_with_entities("viz", &["/a", "/b"]);
        let set_b = per_visualizer_with_entities("viz", &["/a", "/c"]);

        let fingerprint_a = VisualizableEntitiesCacheKey::subscriber_fingerprint(&set_a);
        let fingerprint_b = VisualizableEntitiesCacheKey::subscriber_fingerprint(&set_b);

        assert_ne!(fingerprint_a, fingerprint_b);
    }

    #[test]
    fn fingerprint_stable_across_iteration_order() {
        let set_a = per_visualizer_with_entities("viz", &["/a", "/b"]);
        let set_b = per_visualizer_with_entities("viz", &["/b", "/a"]);

        let fingerprint_a = VisualizableEntitiesCacheKey::subscriber_fingerprint(&set_a);
        let fingerprint_b = VisualizableEntitiesCacheKey::subscriber_fingerprint(&set_b);

        assert_eq!(fingerprint_a, fingerprint_b);
    }
}
