//! Performance metrics panel for issue #8233
//!
//! Provides real-time bottleneck tracking and optimization progress monitoring.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::Duration;

use egui::{Color32, RichText, Ui};
use web_time::Instant;

use re_viewer_context::performance_metrics;

// ============================================================================
// Main Panel Structure
// ============================================================================

/// Performance metrics collector and display
pub struct PerformancePanel {
    /// Whether panel is visible
    pub enabled: bool,

    /// Data collection state
    pub paused: bool,

    /// Rolling window of frame times
    frame_times: VecDeque<Duration>,

    /// Start time of current frame
    frame_start: Option<Instant>,

    /// Total frames collected
    total_frames: u64,

    /// Session start time
    session_start: Instant,

    /// Per-phase timings (updated each frame)
    pub phase_timings: PhaseTimings,

    /// Bottleneck-specific metrics
    pub bottleneck_metrics: BottleneckMetrics,

    /// Cache statistics
    pub cache_stats: CacheStatistics,

    /// Memory usage tracking
    pub memory_stats: MemoryStatistics,

    /// Baseline for comparison (optional)
    baseline: Option<PerformanceBaseline>,
}

// ============================================================================
// Metrics Structures
// ============================================================================

#[derive(Default, Clone, Copy)]
pub struct PhaseTimings {
    pub blueprint_query: Duration,
    pub query_results: Duration,
    pub update_overrides: Duration,
    pub execute_systems: Duration,
    pub ui_rendering: Duration,
    pub gc: Duration,
}

impl PhaseTimings {
    fn total(&self) -> Duration {
        self.blueprint_query
            + self.query_results
            + self.update_overrides
            + self.execute_systems
            + self.ui_rendering
            + self.gc
    }

    fn bottleneck_phase(&self) -> &'static str {
        let mut max_duration = Duration::ZERO;
        let mut phase_name = "None";

        let phases = [
            ("Blueprint Query", self.blueprint_query),
            ("Query Results", self.query_results),
            ("Update Overrides", self.update_overrides),
            ("Execute Systems", self.execute_systems),
            ("UI Rendering", self.ui_rendering),
            ("GC", self.gc),
        ];

        for (name, duration) in phases {
            if duration > max_duration {
                max_duration = duration;
                phase_name = name;
            }
        }

        phase_name
    }
}

/// Tracks metrics for the 8 identified bottlenecks
#[derive(Default)]
pub struct BottleneckMetrics {
    // Bottleneck 1: Redundant annotation loading
    pub annotation_loads_per_frame: u64,

    // Bottleneck 2: Per-view entity tree walk
    pub entity_tree_walks_per_frame: u64,

    // Bottleneck 3: Conservative transform invalidation
    pub transform_invalidations_per_frame: u64,

    // Bottleneck 4: Eager timeline indexing
    pub timelines_indexed_per_frame: u64,
    pub timelines_total: u64,

    // Bottleneck 5: Blueprint tree rebuilds
    pub blueprint_tree_rebuilds_per_frame: u64,

    // Bottleneck 6: Query result tree traversal
    pub query_traversals_per_frame: u64,

    // Bottleneck 7: Per-frame system execution overhead
    pub system_overhead_us: u64,

    // Bottleneck 8: Time series tessellation
    pub time_series_tessellation_count: u64,
}

#[derive(Default)]
pub struct CacheStatistics {
    // Query cache
    pub query_cache_hits: u64,
    pub query_cache_misses: u64,

    // Transform cache
    pub transform_cache_hits: u64,
    pub transform_cache_misses: u64,

    // Blueprint tree cache
    pub blueprint_tree_cache_hits: u64,
    pub blueprint_tree_cache_misses: u64,
}

#[derive(Default)]
pub struct MemoryStatistics {
    pub rss_bytes: u64,
    pub counted_bytes: u64,
}

/// Baseline metrics for comparison
#[derive(Clone)]
struct PerformanceBaseline {
    p50: Duration,
    p95: Duration,
    p99: Duration,
    timestamp: Instant,
}

// ============================================================================
// Implementation
// ============================================================================

impl PerformancePanel {
    const WINDOW_SIZE: usize = 60; // 60 frames = ~1 second at 60 FPS

    pub fn new() -> Self {
        Self {
            enabled: false,
            paused: false,
            frame_times: VecDeque::with_capacity(Self::WINDOW_SIZE),
            frame_start: None,
            total_frames: 0,
            session_start: Instant::now(),
            phase_timings: Default::default(),
            bottleneck_metrics: Default::default(),
            cache_stats: Default::default(),
            memory_stats: Default::default(),
            baseline: None,
        }
    }

    /// Call at start of frame
    pub fn begin_frame(&mut self) {
        if !self.enabled || self.paused {
            return;
        }
        self.frame_start = Some(Instant::now());
    }

    /// Call at end of frame
    pub fn end_frame(&mut self) {
        if !self.enabled || self.paused {
            return;
        }

        if let Some(start) = self.frame_start.take() {
            let frame_time = start.elapsed();
            self.frame_times.push_back(frame_time);

            // Keep only last N frames
            while self.frame_times.len() > Self::WINDOW_SIZE {
                self.frame_times.pop_front();
            }

            self.total_frames += 1;

            // Collect bottleneck metrics from atomics
            self.bottleneck_metrics.annotation_loads_per_frame =
                performance_metrics::ANNOTATION_LOADS_THIS_FRAME.swap(0, Ordering::Relaxed);
            self.bottleneck_metrics.entity_tree_walks_per_frame =
                performance_metrics::ENTITY_TREE_WALKS_THIS_FRAME.swap(0, Ordering::Relaxed);
            self.bottleneck_metrics.transform_invalidations_per_frame =
                performance_metrics::TRANSFORM_INVALIDATIONS_THIS_FRAME.swap(0, Ordering::Relaxed);
            self.bottleneck_metrics.blueprint_tree_rebuilds_per_frame =
                performance_metrics::BLUEPRINT_TREE_REBUILDS_THIS_FRAME.swap(0, Ordering::Relaxed);
            self.bottleneck_metrics.query_traversals_per_frame =
                performance_metrics::QUERY_TRAVERSALS_THIS_FRAME.swap(0, Ordering::Relaxed);

            // Collect cache statistics
            self.cache_stats.query_cache_hits =
                performance_metrics::QUERY_CACHE_HITS.swap(0, Ordering::Relaxed);
            self.cache_stats.query_cache_misses =
                performance_metrics::QUERY_CACHE_MISSES.swap(0, Ordering::Relaxed);

            self.cache_stats.transform_cache_hits =
                performance_metrics::TRANSFORM_CACHE_HITS.swap(0, Ordering::Relaxed);
            self.cache_stats.transform_cache_misses =
                performance_metrics::TRANSFORM_CACHE_MISSES.swap(0, Ordering::Relaxed);

            self.cache_stats.blueprint_tree_cache_hits =
                performance_metrics::BLUEPRINT_TREE_CACHE_HITS.swap(0, Ordering::Relaxed);
            self.cache_stats.blueprint_tree_cache_misses =
                performance_metrics::BLUEPRINT_TREE_CACHE_MISSES.swap(0, Ordering::Relaxed);
        }
    }

    /// Calculate percentile from frame times
    fn percentile(&self, p: f64) -> Duration {
        if self.frame_times.is_empty() {
            return Duration::ZERO;
        }

        let mut sorted: Vec<_> = self.frame_times.iter().copied().collect();
        sorted.sort();

        let index = ((sorted.len() as f64) * p) as usize;
        sorted[index.min(sorted.len() - 1)]
    }

    /// Set current metrics as baseline for comparison
    pub fn set_baseline(&mut self) {
        self.baseline = Some(PerformanceBaseline {
            p50: self.percentile(0.5),
            p95: self.percentile(0.95),
            p99: self.percentile(0.99),
            timestamp: Instant::now(),
        });
    }

    /// Clear baseline
    pub fn clear_baseline(&mut self) {
        self.baseline = None;
    }

    /// Reset all statistics
    pub fn reset(&mut self) {
        self.frame_times.clear();
        self.cache_stats = Default::default();
        self.bottleneck_metrics = Default::default();
        self.total_frames = 0;
        self.session_start = Instant::now();
    }

    /// Show the performance panel
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.enabled {
            return;
        }

        egui::Window::new("⚡ Performance Metrics (Issue #8233)")
            .default_pos([20.0, 100.0])
            .default_size([480.0, 700.0])
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                self.ui_impl(ui);
            });
    }

    fn ui_impl(&mut self, ui: &mut Ui) {
        // Control bar
        ui.horizontal(|ui| {
            if ui
                .button(if self.paused {
                    "▶ Resume"
                } else {
                    "⏸ Pause"
                })
                .clicked()
            {
                self.paused = !self.paused;
            }

            if ui.button("🔄 Reset").clicked() {
                self.reset();
            }

            if self.baseline.is_some() {
                if ui.button("Clear Baseline").clicked() {
                    self.clear_baseline();
                }
            } else if ui.button("📊 Set Baseline").clicked() {
                self.set_baseline();
            }
        });

        ui.add_space(5.0);

        // Session info
        ui.horizontal(|ui| {
            ui.label(format!(
                "Session: {:.1}s",
                self.session_start.elapsed().as_secs_f64()
            ));
            ui.label(format!("Frames: {}", self.total_frames));
            if self.paused {
                ui.colored_label(Color32::YELLOW, "⏸ PAUSED");
            }
        });

        ui.separator();

        // Main sections
        ui.heading("Frame Time");
        self.show_frame_times(ui);

        ui.add_space(10.0);
        ui.separator();

        ui.heading("Phase Breakdown");
        self.show_phase_breakdown(ui);

        ui.add_space(10.0);
        ui.separator();

        ui.heading("Bottleneck Metrics");
        self.show_bottleneck_metrics(ui);

        ui.add_space(10.0);
        ui.separator();

        ui.heading("Cache Effectiveness");
        self.show_cache_stats(ui);

        ui.add_space(10.0);
        ui.separator();

        ui.heading("Memory Usage");
        self.show_memory_stats(ui);

        ui.add_space(10.0);
        ui.separator();

        ui.heading("Optimization Status");
        self.show_optimization_status(ui);
    }

    fn show_frame_times(&self, ui: &mut Ui) {
        if self.frame_times.is_empty() {
            ui.label("No data yet...");
            return;
        }

        let p50 = self.percentile(0.5);
        let p95 = self.percentile(0.95);
        let p99 = self.percentile(0.99);

        // Color coding based on performance
        let p95_ms = p95.as_secs_f64() * 1000.0;
        let p95_color = if p95_ms < 16.0 {
            Color32::GREEN
        } else if p95_ms < 33.0 {
            Color32::YELLOW
        } else {
            Color32::RED
        };

        ui.horizontal(|ui| {
            ui.label("P50:");
            ui.label(format!("{:.1}ms", p50.as_secs_f64() * 1000.0));
            if let Some(baseline) = &self.baseline {
                let delta = (p50.as_secs_f64() - baseline.p50.as_secs_f64()) * 1000.0;
                self.show_delta(ui, delta);
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("P95:").strong());
            ui.colored_label(
                p95_color,
                RichText::new(format!("{:.1}ms", p95_ms)).strong(),
            );
            if let Some(baseline) = &self.baseline {
                let delta = (p95.as_secs_f64() - baseline.p95.as_secs_f64()) * 1000.0;
                self.show_delta(ui, delta);
            }
        });

        ui.horizontal(|ui| {
            ui.label("P99:");
            ui.label(format!("{:.1}ms", p99.as_secs_f64() * 1000.0));
            if let Some(baseline) = &self.baseline {
                let delta = (p99.as_secs_f64() - baseline.p99.as_secs_f64()) * 1000.0;
                self.show_delta(ui, delta);
            }
        });

        // FPS calculation
        let fps = if p50 > Duration::ZERO {
            1.0 / p50.as_secs_f64()
        } else {
            0.0
        };

        ui.horizontal(|ui| {
            ui.label("FPS:");
            ui.label(format!("{:.1}", fps));
        });

        ui.add_space(5.0);

        // Target indicator
        ui.horizontal(|ui| {
            ui.label("Target: P95 < 16ms");
            if p95_ms < 16.0 {
                ui.colored_label(Color32::GREEN, "✓ TARGET MET");
            } else {
                let over = p95_ms - 16.0;
                ui.colored_label(Color32::RED, format!("({:.1}ms over)", over));
            }
        });

        ui.add_space(5.0);

        // Mini timeline graph
        self.show_frame_time_graph(ui);
    }

    fn show_delta(&self, ui: &mut Ui, delta_ms: f64) {
        let (color, sign) = if delta_ms.abs() < 0.01 {
            (Color32::GRAY, "±")
        } else if delta_ms < 0.0 {
            (Color32::GREEN, "")
        } else {
            (Color32::RED, "+")
        };

        ui.colored_label(color, format!("({}{:.1}ms)", sign, delta_ms));
    }

    fn show_frame_time_graph(&self, ui: &mut Ui) {
        use egui_plot::{Line, LineStyle, Plot, PlotPoints};

        let points: PlotPoints<'_> = self
            .frame_times
            .iter()
            .enumerate()
            .map(|(i, &duration)| [i as f64, duration.as_secs_f64() * 1000.0])
            .collect();

        let line = Line::new("frame_time", points)
            .color(Color32::LIGHT_BLUE)
            .width(2.0);

        Plot::new("frame_time_plot")
            .height(120.0)
            .show_axes([false, true])
            .allow_zoom(false)
            .allow_drag(false)
            .allow_scroll(false)
            .show(ui, |plot_ui| {
                plot_ui.line(line);

                // 60 FPS target line (16ms)
                let target_60fps: PlotPoints<'_> =
                    vec![[0.0, 16.0], [Self::WINDOW_SIZE as f64, 16.0]].into();
                let target_line_60 = Line::new("60fps_target", target_60fps)
                    .color(Color32::GREEN)
                    .width(1.5)
                    .style(LineStyle::Dashed { length: 5.0 });
                plot_ui.line(target_line_60);

                // 30 FPS reference line (33ms)
                let target_30fps: PlotPoints<'_> =
                    vec![[0.0, 33.0], [Self::WINDOW_SIZE as f64, 33.0]].into();
                let target_line_30 = Line::new("30fps_ref", target_30fps)
                    .color(Color32::YELLOW)
                    .width(1.0)
                    .style(LineStyle::Dashed { length: 3.0 });
                plot_ui.line(target_line_30);
            });
    }

    fn show_phase_breakdown(&self, ui: &mut Ui) {
        let total = self.phase_timings.total();

        if total == Duration::ZERO {
            ui.label("No timing data yet...");
            return;
        }

        let bottleneck = self.phase_timings.bottleneck_phase();

        ui.horizontal(|ui| {
            ui.label(format!("Total: {:.1}ms", total.as_secs_f64() * 1000.0));
            ui.label("•");
            ui.label(format!("Bottleneck: {}", bottleneck));
        });

        ui.add_space(5.0);

        let phases = [
            (
                "Blueprint Query",
                self.phase_timings.blueprint_query,
                Color32::from_rgb(100, 150, 200),
            ),
            (
                "Query Results",
                self.phase_timings.query_results,
                Color32::from_rgb(200, 100, 100),
            ),
            (
                "Update Overrides",
                self.phase_timings.update_overrides,
                Color32::from_rgb(100, 200, 100),
            ),
            (
                "Execute Systems",
                self.phase_timings.execute_systems,
                Color32::from_rgb(200, 200, 100),
            ),
            (
                "UI Rendering",
                self.phase_timings.ui_rendering,
                Color32::from_rgb(150, 100, 200),
            ),
            (
                "GC",
                self.phase_timings.gc,
                Color32::from_rgb(200, 100, 200),
            ),
        ];

        for (name, duration, color) in phases {
            let ms = duration.as_secs_f64() * 1000.0;
            let percentage = if total > Duration::ZERO {
                (duration.as_secs_f64() / total.as_secs_f64()) * 100.0
            } else {
                0.0
            };

            ui.horizontal(|ui| {
                ui.colored_label(color, "█");
                ui.label(format!("{:18}", name));
                ui.label(format!("{:5.1}ms", ms));
                ui.label(format!("({:4.1}%)", percentage));

                if name == bottleneck {
                    ui.colored_label(Color32::RED, "← BOTTLENECK");
                }
            });
        }
    }

    fn show_bottleneck_metrics(&self, ui: &mut Ui) {
        let bm = &self.bottleneck_metrics;

        ui.horizontal(|ui| {
            ui.label("1. Annotation Loads:");
            let color = if bm.annotation_loads_per_frame <= 1 {
                Color32::GREEN
            } else if bm.annotation_loads_per_frame < 10 {
                Color32::YELLOW
            } else {
                Color32::RED
            };
            ui.colored_label(color, format!("{}/frame", bm.annotation_loads_per_frame));
            ui.label("(target: 1)");
        });

        ui.horizontal(|ui| {
            ui.label("2. Entity Tree Walks:");
            let color = if bm.entity_tree_walks_per_frame <= 1 {
                Color32::GREEN
            } else {
                Color32::YELLOW
            };
            ui.colored_label(color, format!("{}/frame", bm.entity_tree_walks_per_frame));
            ui.label("(target: 1)");
        });

        ui.horizontal(|ui| {
            ui.label("3. Transform Invalidations:");
            ui.label(format!("{}/frame", bm.transform_invalidations_per_frame));
        });

        ui.horizontal(|ui| {
            ui.label("4. Timelines Indexed:");
            let ratio = if bm.timelines_total > 0 {
                bm.timelines_indexed_per_frame as f64 / bm.timelines_total as f64
            } else {
                0.0
            };
            let color = if ratio < 0.5 {
                Color32::GREEN
            } else {
                Color32::YELLOW
            };
            ui.colored_label(
                color,
                format!("{}/{}", bm.timelines_indexed_per_frame, bm.timelines_total),
            );
        });

        ui.horizontal(|ui| {
            ui.label("5. Blueprint Tree Rebuilds:");
            let color = if bm.blueprint_tree_rebuilds_per_frame == 0 {
                Color32::GREEN
            } else {
                Color32::RED
            };
            ui.colored_label(
                color,
                format!("{}/frame", bm.blueprint_tree_rebuilds_per_frame),
            );
            ui.label("(target: 0)");
        });

        ui.horizontal(|ui| {
            ui.label("6. Query Traversals:");
            ui.label(format!("{}/frame", bm.query_traversals_per_frame));
        });

        ui.horizontal(|ui| {
            ui.label("7. System Overhead:");
            ui.label(format!("{}µs", bm.system_overhead_us));
        });

        ui.horizontal(|ui| {
            ui.label("8. Time Series Tessellation:");
            ui.label(format!("{}", bm.time_series_tessellation_count));
        });
    }

    fn show_cache_stats(&self, ui: &mut Ui) {
        let cs = &self.cache_stats;

        // Query cache
        let query_hit_rate = self.cache_hit_rate(cs.query_cache_hits, cs.query_cache_misses);
        self.show_cache_row(ui, "Query Cache", query_hit_rate, 90.0);

        // Transform cache
        let transform_hit_rate =
            self.cache_hit_rate(cs.transform_cache_hits, cs.transform_cache_misses);
        self.show_cache_row(ui, "Transform Cache", transform_hit_rate, 85.0);

        // Blueprint tree cache
        let blueprint_hit_rate =
            self.cache_hit_rate(cs.blueprint_tree_cache_hits, cs.blueprint_tree_cache_misses);
        self.show_cache_row(ui, "Blueprint Tree", blueprint_hit_rate, 95.0);
    }

    fn cache_hit_rate(&self, hits: u64, misses: u64) -> f64 {
        let total = hits + misses;
        if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }

    fn show_cache_row(&self, ui: &mut Ui, name: &str, hit_rate: f64, target: f64) {
        let color = if hit_rate >= target {
            Color32::GREEN
        } else if hit_rate >= target - 10.0 {
            Color32::YELLOW
        } else {
            Color32::RED
        };

        ui.horizontal(|ui| {
            ui.label(format!("{:18}", name));
            ui.colored_label(color, format!("{:5.1}%", hit_rate));
            ui.label(format!("(target: >{:.0}%)", target));
        });
    }

    fn show_memory_stats(&self, ui: &mut Ui) {
        let ms = &self.memory_stats;

        ui.horizontal(|ui| {
            ui.label("RSS:");
            ui.label(format!("{:.1} MB", ms.rss_bytes as f64 / 1_000_000.0));
        });

        ui.horizontal(|ui| {
            ui.label("Counted:");
            ui.label(format!("{:.1} MB", ms.counted_bytes as f64 / 1_000_000.0));
        });
    }

    fn show_optimization_status(&self, ui: &mut Ui) {
        ui.label(RichText::new("Issue #8233 Optimizations").strong());

        let bm = &self.bottleneck_metrics;
        let cs = &self.cache_stats;

        let optimizations = [
            (
                "1. Annotation Loading",
                bm.annotation_loads_per_frame <= 1,
                "Task 1.2",
            ),
            (
                "2. Lazy Timeline Indexing",
                bm.timelines_indexed_per_frame < bm.timelines_total,
                "Task 1.3",
            ),
            (
                "3. Blueprint Tree Caching",
                cs.blueprint_tree_cache_hits > 0,
                "Task 2.1",
            ),
            (
                "4. Shared Entity Walk",
                bm.entity_tree_walks_per_frame <= 1,
                "Task 2.2",
            ),
            (
                "5. Transform Invalidation",
                bm.transform_invalidations_per_frame < 10,
                "Task 2.3",
            ),
            (
                "6. Incremental UI",
                bm.time_series_tessellation_count == 0,
                "Task 3.1",
            ),
            ("7. Viewport Culling", false, "Task 3.2"),
            ("8. Performance Tests", false, "Task 3.3"),
        ];

        for (name, status, task) in optimizations {
            ui.horizontal(|ui| {
                let (icon, color) = if status {
                    ("✓", Color32::GREEN)
                } else {
                    ("○", Color32::GRAY)
                };
                ui.colored_label(color, icon);
                ui.label(name);
                ui.label(RichText::new(task).weak());
            });
        }
    }
}

// ============================================================================
// Global Metrics Collection (Thread-safe atomics)
// ============================================================================
// Note: All atomic counters are now defined in re_viewer_context::performance_metrics
// and imported above.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_panel_frame_tracking() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        // Simulate frames
        for _ in 0..10 {
            panel.begin_frame();
            std::thread::sleep(Duration::from_micros(15_000));
            panel.end_frame();
        }

        // Check we have data
        assert_eq!(panel.frame_times.len(), 10);
        assert_eq!(panel.total_frames, 10);
    }

    #[test]
    fn test_percentile_calculation() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        // Add known frame times
        panel.frame_times.push_back(Duration::from_millis(10));
        panel.frame_times.push_back(Duration::from_millis(15));
        panel.frame_times.push_back(Duration::from_millis(20));

        let p50 = panel.percentile(0.5);
        assert!(p50.as_millis() >= 14 && p50.as_millis() <= 16);
    }

    #[test]
    fn test_baseline_comparison() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        // Add some frame times
        for _ in 0..10 {
            panel.frame_times.push_back(Duration::from_millis(20));
        }

        // Set baseline
        panel.set_baseline();
        assert!(panel.baseline.is_some());

        // Clear baseline
        panel.clear_baseline();
        assert!(panel.baseline.is_none());
    }

    #[test]
    fn test_pause_resume() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        // Collect some frames
        for _ in 0..5 {
            panel.begin_frame();
            panel.end_frame();
        }
        assert_eq!(panel.total_frames, 5);

        // Pause
        panel.paused = true;
        for _ in 0..5 {
            panel.begin_frame(); // Should do nothing
            panel.end_frame(); // Should do nothing
        }
        assert_eq!(panel.total_frames, 5); // No change

        // Resume
        panel.paused = false;
        for _ in 0..5 {
            panel.begin_frame();
            panel.end_frame();
        }
        assert_eq!(panel.total_frames, 10);
    }

    #[test]
    fn test_bottleneck_phase_detection() {
        let mut timings = PhaseTimings::default();
        timings.blueprint_query = Duration::from_millis(2);
        timings.query_results = Duration::from_millis(15); // Slowest
        timings.update_overrides = Duration::from_millis(3);
        timings.execute_systems = Duration::from_millis(4);
        timings.ui_rendering = Duration::from_millis(5);
        timings.gc = Duration::from_millis(1);

        assert_eq!(timings.bottleneck_phase(), "Query Results");
    }
}
