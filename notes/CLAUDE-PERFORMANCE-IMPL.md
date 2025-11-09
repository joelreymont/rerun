# Rerun Performance Optimization: Implementation Plan

**Related:** CLAUDE-PERFORMANCE.md (Issue #8233)
**Target:** Achieve 60 FPS (16.67ms per frame) for air traffic 2h dataset
**Current State:** P95 ~30ms, P99 ~40ms
**Timeline:** 8-12 weeks across 3 phases

---

## Executive Summary

This document provides a step-by-step implementation plan for the 8 identified performance bottlenecks. Each phase includes:
- **Detailed task breakdown** with file-level changes
- **Testing strategies** to verify correctness
- **Measurement approaches** to validate improvements
- **Risk mitigation** and rollback procedures
- **Success criteria** with concrete metrics

### Target Improvements by Phase

| Phase | Duration | Frame Time Reduction | Cumulative Target |
|-------|----------|---------------------|-------------------|
| Phase 1: Quick Wins | 1-2 weeks | 30-40% | P95 < 20ms |
| Phase 2: Architectural | 3-4 weeks | Additional 20-30% | P95 < 16ms |
| Phase 3: Long-term | 1-2 months | Additional 10-20% | P95 < 14ms |

---

## Table of Contents

1. [Phase 1: Quick Wins (1-2 weeks)](#phase-1-quick-wins)
2. [Phase 2: Architectural Improvements (3-4 weeks)](#phase-2-architectural-improvements)
3. [Phase 3: Long-term Optimizations (1-2 months)](#phase-3-long-term-optimizations)
4. [Testing Strategy](#testing-strategy)
5. [Measurement & Validation](#measurement--validation)
6. [Risk Management](#risk-management)

---

## Phase 1: Quick Wins (1-2 weeks)

**Goal:** 30-40% frame time reduction through low-risk, high-impact optimizations
**Target Metric:** P95 frame time < 20ms (currently ~30ms)

### Task 1.1: Enhanced Performance Metrics Floating Panel

**Priority:** CRITICAL (baseline measurement + visual feedback + optimization tracking)
**Duration:** 3-4 days
**Risk:** VERY LOW

#### Overview

Create a comprehensive floating egui panel that provides real-time, bottleneck-specific performance metrics during viewer operation. This panel serves as both a diagnostic tool and a visual progress tracker, showing baseline performance and improvements as each optimization is implemented.

**Key Design Principles:**
- **Bottleneck-focused**: Track metrics for each of the 8 identified bottlenecks
- **Trend awareness**: Show not just current values, but trends and history
- **Interactive**: Pause/resume, export data, compare before/after
- **Low overhead**: Minimal performance impact when enabled, zero when disabled
- **Actionable**: Clearly indicate which optimizations are active and working

**Pattern:** Based on `rrd_loading_metrics.rs` (commit 873d6a0) but significantly enhanced

#### Implementation Steps

**Step 1: Create Enhanced Metrics Module (Day 1, Full Day)**

```bash
# Create new module
touch crates/viewer/re_viewer/src/performance_panel.rs
```

**File:** `crates/viewer/re_viewer/src/performance_panel.rs`

```rust
//! Enhanced real-time performance metrics panel for issue #8233
//!
//! This panel provides comprehensive bottleneck tracking and optimization progress monitoring.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Ui};

// ============================================================================
// Main Panel Structure
// ============================================================================

/// Performance metrics collector and display with bottleneck-specific tracking
pub struct PerformancePanel {
    /// Whether panel is visible
    pub enabled: bool,

    /// Data collection state
    pub paused: bool,

    /// Rolling window configuration
    frame_window_size: usize,  // Default: 60 frames (~1s at 60 FPS)
    long_window_size: usize,   // Default: 300 frames (~5s at 60 FPS)

    /// Frame timing data
    frame_times: VecDeque<Duration>,
    frame_times_long: VecDeque<Duration>,  // Longer history for trends

    /// Start time of current frame
    frame_start: Option<Instant>,

    /// Per-phase timings (updated each frame)
    pub phase_timings: PhaseTimings,
    phase_history: VecDeque<PhaseTimings>,  // History for sparklines

    /// Bottleneck-specific metrics
    pub bottleneck_metrics: BottleneckMetrics,

    /// Cache statistics
    pub cache_stats: CacheStatistics,

    /// Memory usage tracking
    pub memory_stats: MemoryStatistics,

    /// Baseline for comparison (optional)
    baseline: Option<PerformanceBaseline>,

    /// Session start time
    session_start: Instant,

    /// Total frames collected
    total_frames: u64,
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
    pub annotation_loads_history: VecDeque<u64>,

    // Bottleneck 2: Per-view entity tree walk
    pub entity_tree_walks_per_frame: u64,
    pub entities_visited_per_frame: u64,

    // Bottleneck 3: Conservative transform invalidation
    pub transform_invalidations_per_frame: u64,
    pub transform_invalidations_history: VecDeque<u64>,

    // Bottleneck 4: Eager timeline indexing
    pub timelines_indexed_per_frame: u64,
    pub timelines_total: u64,

    // Bottleneck 5: Blueprint tree rebuilds
    pub blueprint_tree_rebuilds_per_frame: u64,
    pub blueprint_tree_rebuilds_history: VecDeque<u64>,

    // Bottleneck 6: Query result tree traversal
    pub query_traversals_per_frame: u64,
    pub query_traversals_per_view: Vec<u64>,

    // Bottleneck 7: Per-frame system execution overhead
    pub systems_executed_per_frame: u64,
    pub system_overhead_us: u64,

    // Bottleneck 8: Time series tessellation
    pub time_series_tessellation_count: u64,
    pub time_series_tessellation_time: Duration,
}

#[derive(Default)]
pub struct CacheStatistics {
    // Query cache
    pub query_cache_hits: u64,
    pub query_cache_misses: u64,
    pub query_cache_size_mb: f64,

    // Transform cache
    pub transform_cache_hits: u64,
    pub transform_cache_misses: u64,
    pub transform_cache_size_mb: f64,

    // Blueprint tree cache
    pub blueprint_tree_cache_hits: u64,
    pub blueprint_tree_cache_misses: u64,

    // Mesh/Image decode caches
    pub mesh_cache_hits: u64,
    pub mesh_cache_misses: u64,
    pub image_decode_cache_hits: u64,
    pub image_decode_cache_misses: u64,
}

#[derive(Default)]
pub struct MemoryStatistics {
    pub rss_bytes: u64,
    pub counted_bytes: u64,
    pub chunk_store_bytes: u64,
    pub query_cache_bytes: u64,
}

/// Baseline metrics for comparison
#[derive(Clone)]
struct PerformanceBaseline {
    p50: Duration,
    p95: Duration,
    p99: Duration,
    avg_frame_time: Duration,
    annotation_loads_per_frame: f64,
    transform_invalidations_per_frame: f64,
    timestamp: Instant,
}

// ============================================================================
// Implementation
// ============================================================================

impl Default for PerformancePanel {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformancePanel {
    const SHORT_WINDOW: usize = 60;   // 1 second at 60 FPS
    const LONG_WINDOW: usize = 300;   // 5 seconds at 60 FPS
    const HISTORY_SIZE: usize = 100;  // Phase history for sparklines

    pub fn new() -> Self {
        Self {
            enabled: false,
            paused: false,
            frame_window_size: Self::SHORT_WINDOW,
            long_window_size: Self::LONG_WINDOW,
            frame_times: VecDeque::with_capacity(Self::SHORT_WINDOW),
            frame_times_long: VecDeque::with_capacity(Self::LONG_WINDOW),
            frame_start: None,
            phase_timings: Default::default(),
            phase_history: VecDeque::with_capacity(Self::HISTORY_SIZE),
            bottleneck_metrics: Default::default(),
            cache_stats: Default::default(),
            memory_stats: Default::default(),
            baseline: None,
            session_start: Instant::now(),
            total_frames: 0,
        }
    }

    /// Call at start of frame (only if enabled and not paused)
    pub fn begin_frame(&mut self) {
        if !self.enabled || self.paused {
            return;
        }
        self.frame_start = Some(Instant::now());
    }

    /// Call at end of frame (only if enabled and not paused)
    pub fn end_frame(&mut self) {
        if !self.enabled || self.paused {
            return;
        }

        if let Some(start) = self.frame_start.take() {
            let frame_time = start.elapsed();

            // Update short window
            self.frame_times.push_back(frame_time);
            while self.frame_times.len() > self.frame_window_size {
                self.frame_times.pop_front();
            }

            // Update long window
            self.frame_times_long.push_back(frame_time);
            while self.frame_times_long.len() > self.long_window_size {
                self.frame_times_long.pop_front();
            }

            // Update phase history
            self.phase_history.push_back(self.phase_timings);
            while self.phase_history.len() > Self::HISTORY_SIZE {
                self.phase_history.pop_front();
            }

            // Update bottleneck history
            self.update_bottleneck_history();

            self.total_frames += 1;
        }
    }

    fn update_bottleneck_history(&mut self) {
        let bm = &mut self.bottleneck_metrics;

        // Annotation loads
        bm.annotation_loads_history.push_back(bm.annotation_loads_per_frame);
        while bm.annotation_loads_history.len() > Self::HISTORY_SIZE {
            bm.annotation_loads_history.pop_front();
        }

        // Transform invalidations
        bm.transform_invalidations_history.push_back(bm.transform_invalidations_per_frame);
        while bm.transform_invalidations_history.len() > Self::HISTORY_SIZE {
            bm.transform_invalidations_history.pop_front();
        }

        // Blueprint tree rebuilds
        bm.blueprint_tree_rebuilds_history.push_back(bm.blueprint_tree_rebuilds_per_frame);
        while bm.blueprint_tree_rebuilds_history.len() > Self::HISTORY_SIZE {
            bm.blueprint_tree_rebuilds_history.pop_front();
        }
    }

    /// Calculate percentile from frame times
    fn percentile(&self, p: f64, use_long_window: bool) -> Duration {
        let window = if use_long_window {
            &self.frame_times_long
        } else {
            &self.frame_times
        };

        if window.is_empty() {
            return Duration::ZERO;
        }

        let mut sorted: Vec<_> = window.iter().copied().collect();
        sorted.sort();

        let index = ((sorted.len() as f64) * p) as usize;
        sorted[index.min(sorted.len() - 1)]
    }

    /// Calculate average frame time
    fn average_frame_time(&self) -> Duration {
        if self.frame_times.is_empty() {
            return Duration::ZERO;
        }

        let total: Duration = self.frame_times.iter().sum();
        total / self.frame_times.len() as u32
    }

    /// Set current metrics as baseline for comparison
    pub fn set_baseline(&mut self) {
        self.baseline = Some(PerformanceBaseline {
            p50: self.percentile(0.5, false),
            p95: self.percentile(0.95, false),
            p99: self.percentile(0.99, false),
            avg_frame_time: self.average_frame_time(),
            annotation_loads_per_frame: self.avg_annotation_loads(),
            transform_invalidations_per_frame: self.avg_transform_invalidations(),
            timestamp: Instant::now(),
        });
    }

    /// Clear baseline
    pub fn clear_baseline(&mut self) {
        self.baseline = None;
    }

    fn avg_annotation_loads(&self) -> f64 {
        if self.bottleneck_metrics.annotation_loads_history.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.bottleneck_metrics.annotation_loads_history.iter().sum();
        sum as f64 / self.bottleneck_metrics.annotation_loads_history.len() as f64
    }

    fn avg_transform_invalidations(&self) -> f64 {
        if self.bottleneck_metrics.transform_invalidations_history.is_empty() {
            return 0.0;
        }
        let sum: u64 = self.bottleneck_metrics.transform_invalidations_history.iter().sum();
        sum as f64 / self.bottleneck_metrics.transform_invalidations_history.len() as f64
    }

    /// Reset all statistics
    pub fn reset(&mut self) {
        self.frame_times.clear();
        self.frame_times_long.clear();
        self.phase_history.clear();
        self.cache_stats = Default::default();
        self.bottleneck_metrics = Default::default();
        self.total_frames = 0;
        self.session_start = Instant::now();
    }

    /// Export metrics to JSON string
    pub fn export_json(&self) -> String {
        // Simplified export - in real implementation would use serde_json
        format!(
            r#"{{
  "session_duration_s": {:.1},
  "total_frames": {},
  "p50_ms": {:.2},
  "p95_ms": {:.2},
  "p99_ms": {:.2},
  "avg_annotation_loads": {:.1},
  "avg_transform_invalidations": {:.1}
}}"#,
            self.session_start.elapsed().as_secs_f64(),
            self.total_frames,
            self.percentile(0.5, false).as_secs_f64() * 1000.0,
            self.percentile(0.95, false).as_secs_f64() * 1000.0,
            self.percentile(0.99, false).as_secs_f64() * 1000.0,
            self.avg_annotation_loads(),
            self.avg_transform_invalidations(),
        )
    }

    // ========================================================================
    // UI Methods
    // ========================================================================

    /// Show the performance panel
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.enabled {
            return;
        }

        egui::Window::new("⚡ Performance Metrics (Issue #8233)")
            .default_pos([20.0, 100.0])
            .default_size([480.0, 800.0])
            .resizable(true)
            .collapsible(true)
            .scroll(true)
            .show(ctx, |ui| {
                self.ui_impl(ui);
            });
    }

    fn ui_impl(&mut self, ui: &mut Ui) {
        // Control bar
        ui.horizontal(|ui| {
            if ui.button(if self.paused { "▶ Resume" } else { "⏸ Pause" }).clicked() {
                self.paused = !self.paused;
            }

            if ui.button("🔄 Reset").clicked() {
                self.reset();
            }

            if self.baseline.is_some() {
                if ui.button("Clear Baseline").clicked() {
                    self.clear_baseline();
                }
            } else {
                if ui.button("📊 Set Baseline").clicked() {
                    self.set_baseline();
                }
            }

            if ui.button("💾 Export JSON").clicked() {
                let json = self.export_json();
                // In real implementation, would copy to clipboard or save to file
                re_log::info!("Performance metrics:\n{}", json);
            }
        });

        ui.add_space(5.0);

        // Session info
        ui.horizontal(|ui| {
            ui.label(format!("Session: {:.1}s", self.session_start.elapsed().as_secs_f64()));
            ui.label(format!("Frames: {}", self.total_frames));
            if self.paused {
                ui.colored_label(Color32::YELLOW, "⏸ PAUSED");
            }
        });

        ui.separator();

        // Main sections
        self.show_frame_time_section(ui);
        ui.add_space(10.0);
        ui.separator();

        self.show_phase_breakdown_section(ui);
        ui.add_space(10.0);
        ui.separator();

        self.show_bottleneck_metrics_section(ui);
        ui.add_space(10.0);
        ui.separator();

        self.show_cache_effectiveness_section(ui);
        ui.add_space(10.0);
        ui.separator();

        self.show_memory_section(ui);
        ui.add_space(10.0);
        ui.separator();

        self.show_optimization_status_section(ui);
    }

    fn show_frame_time_section(&self, ui: &mut Ui) {
        ui.heading("Frame Time");

        if self.frame_times.is_empty() {
            ui.label("No data yet...");
            return;
        }

        let p50 = self.percentile(0.5, false);
        let p95 = self.percentile(0.95, false);
        let p99 = self.percentile(0.99, false);
        let avg = self.average_frame_time();

        // Color coding based on P95 performance
        let p95_ms = p95.as_secs_f64() * 1000.0;
        let p95_color = if p95_ms < 16.0 {
            Color32::GREEN
        } else if p95_ms < 33.0 {
            Color32::YELLOW
        } else {
            Color32::RED
        };

        // Metrics grid
        egui::Grid::new("frame_time_grid")
            .num_columns(3)
            .spacing([20.0, 4.0])
            .show(ui, |ui| {
                // Headers
                ui.label(RichText::new("Metric").strong());
                ui.label(RichText::new("Current").strong());
                if self.baseline.is_some() {
                    ui.label(RichText::new("Baseline").strong());
                }
                ui.end_row();

                // P50
                ui.label("P50:");
                ui.label(format!("{:.1}ms", p50.as_secs_f64() * 1000.0));
                if let Some(baseline) = &self.baseline {
                    let delta = (p50.as_secs_f64() - baseline.p50.as_secs_f64()) * 1000.0;
                    self.show_delta(ui, delta, true);
                }
                ui.end_row();

                // P95 (main target)
                ui.label(RichText::new("P95:").strong());
                ui.colored_label(
                    p95_color,
                    RichText::new(format!("{:.1}ms", p95_ms)).strong(),
                );
                if let Some(baseline) = &self.baseline {
                    let delta = (p95.as_secs_f64() - baseline.p95.as_secs_f64()) * 1000.0;
                    self.show_delta(ui, delta, true);
                }
                ui.end_row();

                // P99
                ui.label("P99:");
                ui.label(format!("{:.1}ms", p99.as_secs_f64() * 1000.0));
                if let Some(baseline) = &self.baseline {
                    let delta = (p99.as_secs_f64() - baseline.p99.as_secs_f64()) * 1000.0;
                    self.show_delta(ui, delta, true);
                }
                ui.end_row();

                // Average
                ui.label("Average:");
                ui.label(format!("{:.1}ms", avg.as_secs_f64() * 1000.0));
                if let Some(baseline) = &self.baseline {
                    let delta = (avg.as_secs_f64() - baseline.avg_frame_time.as_secs_f64()) * 1000.0;
                    self.show_delta(ui, delta, true);
                }
                ui.end_row();

                // FPS
                let fps = if avg > Duration::ZERO {
                    1.0 / avg.as_secs_f64()
                } else {
                    0.0
                };
                ui.label("FPS:");
                ui.label(format!("{:.1}", fps));
                ui.end_row();
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

        // Frame time graph with dual windows
        self.show_frame_time_graph(ui);
    }

    fn show_delta(&self, ui: &mut Ui, delta_ms: f64, lower_is_better: bool) {
        let (color, sign) = if delta_ms.abs() < 0.01 {
            (Color32::GRAY, "±")
        } else if (lower_is_better && delta_ms < 0.0) || (!lower_is_better && delta_ms > 0.0) {
            (Color32::GREEN, if delta_ms < 0.0 { "" } else { "+" })
        } else {
            (Color32::RED, if delta_ms < 0.0 { "" } else { "+" })
        };

        ui.colored_label(color, format!("{}{:.1}ms", sign, delta_ms));
    }

    fn show_frame_time_graph(&self, ui: &mut Ui) {
        use egui::plot::{Line, Plot, PlotPoints};

        let points: PlotPoints = self
            .frame_times
            .iter()
            .enumerate()
            .map(|(i, &duration)| [i as f64, duration.as_secs_f64() * 1000.0])
            .collect();

        let line = Line::new(points)
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
                let target_60fps: PlotPoints = vec![
                    [0.0, 16.0],
                    [self.frame_window_size as f64, 16.0],
                ]
                .into();
                let target_line_60 = Line::new(target_60fps)
                    .color(Color32::GREEN)
                    .width(1.5)
                    .style(egui::plot::LineStyle::Dashed { length: 5.0 });
                plot_ui.line(target_line_60);

                // 30 FPS reference line (33ms)
                let target_30fps: PlotPoints = vec![
                    [0.0, 33.0],
                    [self.frame_window_size as f64, 33.0],
                ]
                .into();
                let target_line_30 = Line::new(target_30fps)
                    .color(Color32::YELLOW)
                    .width(1.0)
                    .style(egui::plot::LineStyle::Dashed { length: 3.0 });
                plot_ui.line(target_line_30);
            });
    }

    fn show_phase_breakdown_section(&self, ui: &mut Ui) {
        ui.heading("Phase Breakdown");

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
            ("Blueprint Query", self.phase_timings.blueprint_query, Color32::from_rgb(100, 150, 200)),
            ("Query Results", self.phase_timings.query_results, Color32::from_rgb(200, 100, 100)),
            ("Update Overrides", self.phase_timings.update_overrides, Color32::from_rgb(100, 200, 100)),
            ("Execute Systems", self.phase_timings.execute_systems, Color32::from_rgb(200, 200, 100)),
            ("UI Rendering", self.phase_timings.ui_rendering, Color32::from_rgb(150, 100, 200)),
            ("GC", self.phase_timings.gc, Color32::from_rgb(200, 100, 200)),
        ];

        egui::Grid::new("phase_grid")
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                for (name, duration, color) in phases {
                    let ms = duration.as_secs_f64() * 1000.0;
                    let percentage = if total > Duration::ZERO {
                        (duration.as_secs_f64() / total.as_secs_f64()) * 100.0
                    } else {
                        0.0
                    };

                    ui.colored_label(color, "█");
                    ui.label(format!("{:18}", name));
                    ui.label(format!("{:5.1}ms", ms));
                    ui.label(format!("({:4.1}%)", percentage));

                    if name == bottleneck {
                        ui.colored_label(Color32::RED, "← BOTTLENECK");
                    }

                    ui.end_row();
                }
            });
    }

    fn show_bottleneck_metrics_section(&self, ui: &mut Ui) {
        ui.heading("Bottleneck Metrics");

        let bm = &self.bottleneck_metrics;

        egui::Grid::new("bottleneck_grid")
            .num_columns(3)
            .spacing([15.0, 6.0])
            .striped(true)
            .show(ui, |ui| {
                // Header
                ui.label(RichText::new("Bottleneck").strong());
                ui.label(RichText::new("Current").strong());
                ui.label(RichText::new("Target").strong());
                ui.end_row();

                // 1. Annotation Loading
                let annot_color = if bm.annotation_loads_per_frame <= 1 {
                    Color32::GREEN
                } else if bm.annotation_loads_per_frame < 10 {
                    Color32::YELLOW
                } else {
                    Color32::RED
                };
                ui.label("1. Annotation Loads/frame:");
                ui.colored_label(annot_color, format!("{}", bm.annotation_loads_per_frame));
                ui.label("1 (single load)");
                ui.end_row();

                // 2. Entity Tree Walks
                let walks_color = if bm.entity_tree_walks_per_frame <= 1 {
                    Color32::GREEN
                } else {
                    Color32::YELLOW
                };
                ui.label("2. Entity Tree Walks/frame:");
                ui.colored_label(walks_color, format!("{}", bm.entity_tree_walks_per_frame));
                ui.label("1 (shared walk)");
                ui.end_row();

                // 3. Transform Invalidations
                ui.label("3. Transform Invalidations:");
                ui.label(format!("{}/frame", bm.transform_invalidations_per_frame));
                ui.label("minimal");
                ui.end_row();

                // 4. Timeline Indexing
                let timeline_ratio = if bm.timelines_total > 0 {
                    bm.timelines_indexed_per_frame as f64 / bm.timelines_total as f64
                } else {
                    0.0
                };
                let timeline_color = if timeline_ratio < 0.5 {
                    Color32::GREEN
                } else {
                    Color32::YELLOW
                };
                ui.label("4. Timelines Indexed:");
                ui.colored_label(
                    timeline_color,
                    format!("{}/{}", bm.timelines_indexed_per_frame, bm.timelines_total)
                );
                ui.label("lazy (as needed)");
                ui.end_row();

                // 5. Blueprint Tree Rebuilds
                let rebuild_color = if bm.blueprint_tree_rebuilds_per_frame == 0 {
                    Color32::GREEN
                } else {
                    Color32::RED
                };
                ui.label("5. Blueprint Tree Rebuilds:");
                ui.colored_label(rebuild_color, format!("{}/frame", bm.blueprint_tree_rebuilds_per_frame));
                ui.label("0 (cached)");
                ui.end_row();

                // 6. Query Traversals
                ui.label("6. Query Traversals/frame:");
                ui.label(format!("{}", bm.query_traversals_per_frame));
                ui.label("minimal");
                ui.end_row();

                // 7. System Execution
                ui.label("7. System Overhead:");
                ui.label(format!("{}µs", bm.system_overhead_us));
                ui.label("<100µs");
                ui.end_row();

                // 8. Time Series Tessellation
                ui.label("8. Time Series Tessellation:");
                ui.label(format!("{} ({:.1}ms)",
                    bm.time_series_tessellation_count,
                    bm.time_series_tessellation_time.as_secs_f64() * 1000.0
                ));
                ui.label("incremental");
                ui.end_row();
            });
    }

    fn show_cache_effectiveness_section(&self, ui: &mut Ui) {
        ui.heading("Cache Effectiveness");

        let cs = &self.cache_stats;

        egui::Grid::new("cache_grid")
            .num_columns(4)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                // Header
                ui.label(RichText::new("Cache").strong());
                ui.label(RichText::new("Hit Rate").strong());
                ui.label(RichText::new("Target").strong());
                ui.label(RichText::new("Size").strong());
                ui.end_row();

                // Query cache
                let query_rate = self.cache_hit_rate(cs.query_cache_hits, cs.query_cache_misses);
                self.show_cache_row_expanded(
                    ui,
                    "Query",
                    query_rate,
                    90.0,
                    cs.query_cache_size_mb,
                );

                // Transform cache
                let transform_rate = self.cache_hit_rate(cs.transform_cache_hits, cs.transform_cache_misses);
                self.show_cache_row_expanded(
                    ui,
                    "Transform",
                    transform_rate,
                    85.0,
                    cs.transform_cache_size_mb,
                );

                // Blueprint tree cache
                let blueprint_rate = self.cache_hit_rate(
                    cs.blueprint_tree_cache_hits,
                    cs.blueprint_tree_cache_misses,
                );
                self.show_cache_row_expanded(ui, "Blueprint Tree", blueprint_rate, 95.0, 0.0);

                // Mesh cache
                let mesh_rate = self.cache_hit_rate(cs.mesh_cache_hits, cs.mesh_cache_misses);
                self.show_cache_row_expanded(ui, "Mesh", mesh_rate, 80.0, 0.0);

                // Image decode cache
                let image_rate = self.cache_hit_rate(cs.image_decode_cache_hits, cs.image_decode_cache_misses);
                self.show_cache_row_expanded(ui, "Image Decode", image_rate, 75.0, 0.0);
            });
    }

    fn cache_hit_rate(&self, hits: u64, misses: u64) -> f64 {
        let total = hits + misses;
        if total > 0 {
            (hits as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }

    fn show_cache_row_expanded(&self, ui: &mut Ui, name: &str, hit_rate: f64, target: f64, size_mb: f64) {
        let color = if hit_rate >= target {
            Color32::GREEN
        } else if hit_rate >= target - 10.0 {
            Color32::YELLOW
        } else {
            Color32::RED
        };

        ui.label(name);
        ui.colored_label(color, format!("{:.1}%", hit_rate));
        ui.label(format!(">{:.0}%", target));
        if size_mb > 0.0 {
            ui.label(format!("{:.1} MB", size_mb));
        } else {
            ui.label("-");
        }
        ui.end_row();
    }

    fn show_memory_section(&self, ui: &mut Ui) {
        ui.heading("Memory Usage");

        let ms = &self.memory_stats;

        egui::Grid::new("memory_grid")
            .num_columns(2)
            .spacing([20.0, 4.0])
            .show(ui, |ui| {
                ui.label("RSS:");
                ui.label(format!("{:.1} MB", ms.rss_bytes as f64 / 1_000_000.0));
                ui.end_row();

                ui.label("Counted:");
                ui.label(format!("{:.1} MB", ms.counted_bytes as f64 / 1_000_000.0));
                ui.end_row();

                if ms.chunk_store_bytes > 0 {
                    ui.label("Chunk Store:");
                    ui.label(format!("{:.1} MB", ms.chunk_store_bytes as f64 / 1_000_000.0));
                    ui.end_row();
                }

                if ms.query_cache_bytes > 0 {
                    ui.label("Query Cache:");
                    ui.label(format!("{:.1} MB", ms.query_cache_bytes as f64 / 1_000_000.0));
                    ui.end_row();
                }
            });
    }

    fn show_optimization_status_section(&self, ui: &mut Ui) {
        ui.heading("Optimization Status");

        let bm = &self.bottleneck_metrics;
        let cs = &self.cache_stats;

        let optimizations = [
            ("1. Annotation Loading", bm.annotation_loads_per_frame <= 1, "Task 1.2"),
            ("2. Lazy Timeline Indexing", bm.timelines_indexed_per_frame < bm.timelines_total, "Task 1.3"),
            ("3. Blueprint Tree Caching", cs.blueprint_tree_cache_hits > 0, "Task 2.1"),
            ("4. Shared Entity Walk", bm.entity_tree_walks_per_frame <= 1, "Task 2.2"),
            ("5. Transform Invalidation", bm.transform_invalidations_per_frame < 10, "Task 2.3"),
            ("6. Incremental UI", bm.time_series_tessellation_count == 0, "Task 3.1"),
            ("7. Viewport Culling", false, "Task 3.2"),  // TODO: Add metric
            ("8. Performance Tests", false, "Task 3.3"),  // TODO: Check CI
        ];

        egui::Grid::new("optimization_grid")
            .num_columns(3)
            .spacing([10.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                for (name, status, task) in optimizations {
                    let (icon, color) = if status {
                        ("✓", Color32::GREEN)
                    } else {
                        ("○", Color32::GRAY)
                    };

                    ui.colored_label(color, RichText::new(icon).size(16.0));
                    ui.label(name);
                    ui.label(RichText::new(task).weak());
                    ui.end_row();
                }
            });
    }
}
```

**Step 2: Integrate with App Structure (Day 2, Morning)**

**File:** `crates/viewer/re_viewer/src/lib.rs`

```rust
// Add to module declarations
mod performance_panel;

// Re-export for convenience
pub use performance_panel::PerformancePanel;

// Add to App struct
pub struct App {
    // ... existing fields ...

    #[cfg(not(target_arch = "wasm32"))]
    performance_panel: PerformancePanel,
}
```

**File:** `crates/viewer/re_viewer/src/app.rs`

```rust
impl App {
    pub fn new(...) -> Self {
        Self {
            // ... existing fields ...

            #[cfg(not(target_arch = "wasm32"))]
            performance_panel: PerformancePanel::new(),
        }
    }

    pub fn update(&mut self, egui_ctx: &egui::Context, frame: &eframe::Frame) {
        re_tracing::profile_function!();

        // Begin frame timing (only if enabled)
        #[cfg(not(target_arch = "wasm32"))]
        self.performance_panel.begin_frame();

        // === Phase 1: Blueprint Query ===
        #[cfg(not(target_arch = "wasm32"))]
        let bp_start = Instant::now();

        // ... existing blueprint query code ...

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.performance_panel.phase_timings.blueprint_query = bp_start.elapsed();
        }

        // === Phase 2: Query Results ===
        #[cfg(not(target_arch = "wasm32"))]
        let query_start = Instant::now();

        // ... existing query execution code ...

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.performance_panel.phase_timings.query_results = query_start.elapsed();
        }

        // === Phase 3: Update Overrides ===
        #[cfg(not(target_arch = "wasm32"))]
        let overrides_start = Instant::now();

        // ... existing overrides code ...

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.performance_panel.phase_timings.update_overrides = overrides_start.elapsed();
        }

        // === Phase 4: Execute Systems ===
        #[cfg(not(target_arch = "wasm32"))]
        let systems_start = Instant::now();

        // ... existing systems code ...

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.performance_panel.phase_timings.execute_systems = systems_start.elapsed();
        }

        // === Phase 5: UI Rendering ===
        #[cfg(not(target_arch = "wasm32"))]
        let ui_start = Instant::now();

        // ... existing UI code ...

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.performance_panel.phase_timings.ui_rendering = ui_start.elapsed();
        }

        // === Phase 6: Garbage Collection ===
        #[cfg(not(target_arch = "wasm32"))]
        let gc_start = Instant::now();

        // ... existing GC code ...

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.performance_panel.phase_timings.gc = gc_start.elapsed();
        }

        // End frame timing and update metrics
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.update_bottleneck_metrics();
            self.performance_panel.end_frame();
        }

        // Show panel (only if enabled)
        #[cfg(not(target_arch = "wasm32"))]
        self.performance_panel.ui(egui_ctx);

        // Handle keyboard shortcut (F12)
        #[cfg(not(target_arch = "wasm32"))]
        if egui_ctx.input(|i| i.key_pressed(egui::Key::F12)) {
            self.performance_panel.enabled = !self.performance_panel.enabled;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn update_bottleneck_metrics(&mut self) {
        // Collect bottleneck metrics from various subsystems
        let bm = &mut self.performance_panel.bottleneck_metrics;

        // Update from global atomic counters (set in Step 4)
        bm.annotation_loads_per_frame = ANNOTATION_LOADS_THIS_FRAME.swap(0, Ordering::Relaxed);
        bm.entity_tree_walks_per_frame = ENTITY_TREE_WALKS_THIS_FRAME.swap(0, Ordering::Relaxed);
        bm.transform_invalidations_per_frame = TRANSFORM_INVALIDATIONS_THIS_FRAME.swap(0, Ordering::Relaxed);
        bm.blueprint_tree_rebuilds_per_frame = BLUEPRINT_TREE_REBUILDS_THIS_FRAME.swap(0, Ordering::Relaxed);
        bm.query_traversals_per_frame = QUERY_TRAVERSALS_THIS_FRAME.swap(0, Ordering::Relaxed);

        // Timeline metrics
        if let Some(transform_cache) = self.transform_cache() {
            bm.timelines_indexed_per_frame = transform_cache.indexed_timelines_count();
            bm.timelines_total = transform_cache.total_timelines_count();
        }

        // Cache stats
        let cs = &mut self.performance_panel.cache_stats;
        cs.query_cache_hits = QUERY_CACHE_HITS.swap(0, Ordering::Relaxed);
        cs.query_cache_misses = QUERY_CACHE_MISSES.swap(0, Ordering::Relaxed);
        cs.transform_cache_hits = TRANSFORM_CACHE_HITS.swap(0, Ordering::Relaxed);
        cs.transform_cache_misses = TRANSFORM_CACHE_MISSES.swap(0, Ordering::Relaxed);
        cs.blueprint_tree_cache_hits = BLUEPRINT_TREE_CACHE_HITS.swap(0, Ordering::Relaxed);
        cs.blueprint_tree_cache_misses = BLUEPRINT_TREE_CACHE_MISSES.swap(0, Ordering::Relaxed);

        // Memory stats
        let ms = &mut self.performance_panel.memory_stats;
        if let Some(memory_info) = re_memory::MemoryUse::capture() {
            ms.rss_bytes = memory_info.resident.unwrap_or(0);
            ms.counted_bytes = memory_info.counted.unwrap_or(0);
        }
    }
}
```

**Step 3: Add Menu Toggle and UI Integration (Day 2, Afternoon)**

**File:** `crates/viewer/re_viewer/src/ui/top_panel.rs`

```rust
impl App {
    fn top_panel_ui(&mut self, frame: &eframe::Frame, ui: &mut egui::Ui) {
        // ... existing menu ...

        ui.menu_button("View", |ui| {
            // ... existing view items ...

            #[cfg(not(target_arch = "wasm32"))]
            {
                ui.separator();

                let icon = if self.performance_panel.enabled { "✓" } else { "" };
                let text = format!("{} ⚡ Performance Metrics (F12)", icon);

                if ui.button(text).clicked() {
                    self.performance_panel.enabled = !self.performance_panel.enabled;
                    ui.close_menu();
                }

                // Additional panel controls in submenu
                if self.performance_panel.enabled {
                    ui.menu_button("Performance Options", |ui| {
                        if ui.button(if self.performance_panel.paused { "▶ Resume" } else { "⏸ Pause" }).clicked() {
                            self.performance_panel.paused = !self.performance_panel.paused;
                        }

                        if ui.button("🔄 Reset").clicked() {
                            self.performance_panel.reset();
                        }

                        if self.performance_panel.baseline.is_some() {
                            if ui.button("Clear Baseline").clicked() {
                                self.performance_panel.clear_baseline();
                            }
                        } else {
                            if ui.button("📊 Set Baseline").clicked() {
                                self.performance_panel.set_baseline();
                            }
                        }

                        if ui.button("💾 Export JSON").clicked() {
                            let json = self.performance_panel.export_json();
                            #[cfg(not(target_arch = "wasm32"))]
                            {
                                if let Err(e) = arboard::Clipboard::new()
                                    .and_then(|mut clipboard| clipboard.set_text(&json))
                                {
                                    re_log::warn!("Failed to copy to clipboard: {}", e);
                                } else {
                                    re_log::info!("Performance metrics copied to clipboard");
                                }
                            }
                        }
                    });
                }
            }
        });
    }
}
```

**Step 4: Instrument Code with Metrics Collection (Day 3, Full Day)**

**Create global atomic counters:**

**File:** `crates/viewer/re_viewer/src/performance_panel.rs` (add to end)

```rust
// ============================================================================
// Global Metrics Collection (Thread-safe atomics)
// ============================================================================

use std::sync::atomic::AtomicU64;

// Bottleneck metrics (reset each frame)
pub static ANNOTATION_LOADS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);
pub static ENTITY_TREE_WALKS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);
pub static TRANSFORM_INVALIDATIONS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);
pub static BLUEPRINT_TREE_REBUILDS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);
pub static QUERY_TRAVERSALS_THIS_FRAME: AtomicU64 = AtomicU64::new(0);

// Cache metrics (reset each frame)
pub static QUERY_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
pub static QUERY_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
pub static TRANSFORM_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
pub static TRANSFORM_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
pub static BLUEPRINT_TREE_CACHE_HITS: AtomicU64 = AtomicU64::new(0);
pub static BLUEPRINT_TREE_CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
```

**Instrument key locations:**

**File:** `crates/viewer/re_data_ui/src/lib.rs` (annotation loading)

```rust
pub fn annotations(...) -> Arc<Annotations> {
    #[cfg(not(target_arch = "wasm32"))]
    re_viewer::ANNOTATION_LOADS_THIS_FRAME.fetch_add(1, Ordering::Relaxed);

    re_tracing::profile_function!();
    // ... existing logic
}
```

**File:** `crates/viewer/re_viewport_blueprint/src/view_contents.rs` (entity tree walks)

```rust
pub fn execute_query(...) {
    #[cfg(not(target_arch = "wasm32"))]
    re_viewer::ENTITY_TREE_WALKS_THIS_FRAME.fetch_add(1, Ordering::Relaxed);

    re_tracing::profile_function!();
    // ... existing walk logic
}
```

**File:** `crates/store/re_tf/src/transform_resolution_cache.rs` (invalidations)

```rust
fn invalidate_at_path(...) {
    let invalidated_count = ...;

    #[cfg(not(target_arch = "wasm32"))]
    re_viewer::TRANSFORM_INVALIDATIONS_THIS_FRAME.fetch_add(invalidated_count as u64, Ordering::Relaxed);

    re_tracing::profile_function!();
    // ... existing logic
}
```

**File:** `crates/viewer/re_blueprint_tree/src/blueprint_tree.rs` (tree rebuilds)

```rust
fn rebuild_tree(...) {
    #[cfg(not(target_arch = "wasm32"))]
    re_viewer::BLUEPRINT_TREE_REBUILDS_THIS_FRAME.fetch_add(1, Ordering::Relaxed);

    re_tracing::profile_function!();
    // ... existing logic
}
```

**File:** `crates/store/re_query/src/cache.rs` (query cache)

```rust
impl QueryCache {
    pub fn latest_at(...) -> Result<...> {
        if let Some(cached) = self.try_get_cached(...) {
            #[cfg(not(target_arch = "wasm32"))]
            re_viewer::QUERY_CACHE_HITS.fetch_add(1, Ordering::Relaxed);
            return Ok(cached);
        }

        #[cfg(not(target_arch = "wasm32"))]
        re_viewer::QUERY_CACHE_MISSES.fetch_add(1, Ordering::Relaxed);

        // ... compute and cache
    }
}
```

**Step 5: Testing & Validation (Day 4, Full Day)**

**Unit Tests:**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_performance_panel_frame_tracking() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        // Simulate frames
        for i in 0..100 {
            panel.begin_frame();
            std::thread::sleep(Duration::from_micros(if i % 2 == 0 { 15_000 } else { 17_000 }));
            panel.end_frame();
        }

        // Check we have data
        assert_eq!(panel.frame_times.len(), 60); // SHORT_WINDOW
        assert_eq!(panel.frame_times_long.len(), 100); // min(100, LONG_WINDOW)

        // Check percentiles are reasonable
        let p50 = panel.percentile(0.5, false);
        let p95 = panel.percentile(0.95, false);
        assert!(p50 > Duration::from_millis(14));
        assert!(p50 < Duration::from_millis(18));
        assert!(p95 > p50);
    }

    #[test]
    fn test_baseline_comparison() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        // Simulate slow frames
        for _ in 0..60 {
            panel.begin_frame();
            std::thread::sleep(Duration::from_millis(20));
            panel.end_frame();
        }

        // Set baseline
        panel.set_baseline();
        let baseline = panel.baseline.as_ref().unwrap();
        assert!(baseline.p95.as_millis() >= 19);

        // Reset and simulate faster frames
        panel.reset();
        for _ in 0..60 {
            panel.begin_frame();
            std::thread::sleep(Duration::from_millis(10));
            panel.end_frame();
        }

        // Check improvement
        let new_p95 = panel.percentile(0.95, false);
        assert!(new_p95 < baseline.p95);
    }

    #[test]
    fn test_pause_resume() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        // Collect some frames
        for _ in 0..30 {
            panel.begin_frame();
            std::thread::sleep(Duration::from_millis(10));
            panel.end_frame();
        }
        assert_eq!(panel.total_frames, 30);

        // Pause
        panel.paused = true;
        for _ in 0..10 {
            panel.begin_frame(); // Should do nothing
            panel.end_frame(); // Should do nothing
        }
        assert_eq!(panel.total_frames, 30); // No change

        // Resume
        panel.paused = false;
        for _ in 0..10 {
            panel.begin_frame();
            std::thread::sleep(Duration::from_millis(10));
            panel.end_frame();
        }
        assert_eq!(panel.total_frames, 40);
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

    #[test]
    fn test_export_json() {
        let mut panel = PerformancePanel::new();
        panel.enabled = true;

        for _ in 0..60 {
            panel.begin_frame();
            std::thread::sleep(Duration::from_millis(15));
            panel.end_frame();
        }

        let json = panel.export_json();
        assert!(json.contains("total_frames"));
        assert!(json.contains("p95_ms"));
        assert!(json.contains("60")); // 60 frames
    }
}
```

**Integration Tests:**

```rust
// tests/performance_panel_integration.rs

#[test]
#[ignore] // Run with --ignored flag
fn test_panel_with_real_viewer() {
    use re_viewer::App;

    // Create app instance
    let mut app = App::new(...);

    #[cfg(not(target_arch = "wasm32"))]
    {
        app.performance_panel.enabled = true;

        // Simulate multiple frames
        for _ in 0..100 {
            app.update(&egui_ctx, &frame);
        }

        // Check metrics were collected
        assert!(app.performance_panel.total_frames >= 100);
        assert!(app.performance_panel.phase_timings.total() > Duration::ZERO);

        // Export for manual inspection
        println!("{}", app.performance_panel.export_json());
    }
}
```

**Manual Testing Checklist:**

1. **Basic Functionality:**
   - [ ] Panel opens with F12
   - [ ] Panel shows in View menu
   - [ ] Panel closes when toggled again
   - [ ] Metrics update every frame

2. **Frame Time Tracking:**
   - [ ] P50/P95/P99 display correctly
   - [ ] Graph shows last 60 frames
   - [ ] 16ms and 33ms reference lines visible
   - [ ] FPS calculation accurate

3. **Phase Breakdown:**
   - [ ] All 6 phases show timing
   - [ ] Percentages add up to ~100%
   - [ ] Bottleneck phase highlighted
   - [ ] Total matches sum of phases

4. **Bottleneck Metrics:**
   - [ ] Annotation loads counted
   - [ ] Entity tree walks counted
   - [ ] Transform invalidations tracked
   - [ ] Timeline indexing stats shown
   - [ ] Blueprint tree rebuilds counted

5. **Cache Effectiveness:**
   - [ ] Query cache hit rate shown
   - [ ] Transform cache hit rate shown
   - [ ] Blueprint cache hit rate shown
   - [ ] Color coding works (green/yellow/red)

6. **Interactive Features:**
   - [ ] Pause stops data collection
   - [ ] Resume restarts collection
   - [ ] Reset clears all metrics
   - [ ] Set Baseline captures current state
   - [ ] Baseline comparison shows deltas
   - [ ] Export JSON works

7. **Performance:**
   - [ ] No visible lag when panel enabled
   - [ ] Zero overhead when panel disabled
   - [ ] Panel doesn't affect measured frame times significantly

#### Success Criteria

**Functional Requirements:**
- ✅ Panel toggles with F12 or View menu
- ✅ All 8 bottleneck metrics tracked and displayed
- ✅ Frame time percentiles (P50, P95, P99) calculated correctly
- ✅ Graph shows 60-frame history with target lines
- ✅ Phase breakdown shows all 6 phases with bottleneck highlighting
- ✅ Cache effectiveness for 5 caches (Query, Transform, Blueprint, Mesh, Image)
- ✅ Memory usage (RSS, counted, chunk store, query cache)
- ✅ Optimization status for all 8 tasks (with checkmarks)

**Interactive Features:**
- ✅ Pause/resume data collection
- ✅ Reset all statistics
- ✅ Set baseline for before/after comparison
- ✅ Export metrics to JSON
- ✅ Baseline delta visualization (green/red deltas)

**Performance:**
- ✅ Zero overhead when disabled
- ✅ <0.1ms overhead when enabled (atomic operations only)
- ✅ No impact on measured frame times

**Data Quality:**
- ✅ Accurate percentile calculation
- ✅ Proper windowing (60 short, 300 long)
- ✅ Bottleneck history tracking (100 samples)
- ✅ Thread-safe metric collection (atomic counters)

#### Benefits Over Original Design

**1. Bottleneck-Specific Tracking:**
- Tracks all 8 identified bottlenecks explicitly
- Shows target values for each metric
- Color-coded status indicators
- Historical trends for key metrics

**2. Before/After Comparison:**
- Set baseline at any point
- See improvement deltas in real-time
- Export for reporting and documentation
- Visual confirmation of optimization progress

**3. Better Visualization:**
- Dual-window frame time tracking (1s and 5s)
- Both 60 FPS and 30 FPS reference lines
- Bottleneck phase highlighting
- Striped grids for readability

**4. Lower Overhead:**
- Only collects when enabled and not paused
- Atomic operations for thread safety
- No allocations in hot path
- Efficient VecDeque usage

**5. Developer Experience:**
- Pause to examine specific scenarios
- Reset to start fresh measurements
- Export for offline analysis
- Comprehensive optimization checklist

**6. Production Ready:**
- Comprehensive unit tests
- Integration test support
- Manual testing checklist
- cfg-gated for native builds only

#### Usage Examples

**Scenario 1: Establish Baseline**
```
1. Open viewer with air traffic 2h dataset
2. Press F12 to open panel
3. Wait for ~5 seconds to collect data
4. Click "📊 Set Baseline"
5. Note P95 baseline (~30ms expected)
```

**Scenario 2: Measure Task 1.2 Impact**
```
1. With baseline set, implement annotation loading fix
2. Rebuild and restart viewer
3. Open same dataset
4. Press F12
5. Watch "Annotation Loads/frame" drop from ~300 to 1
6. See P95 delta show improvement (-5 to -15ms expected)
7. Click "💾 Export JSON" to save metrics
```

**Scenario 3: Debug Regression**
```
1. Panel shows P95 >20ms (regression from Phase 1)
2. Check "Phase Breakdown" to see which phase is slow
3. Check "Bottleneck Metrics" to see which counter increased
4. Check "Cache Effectiveness" to see if hit rates dropped
5. Use data to pinpoint the issue
```

**Scenario 4: Continuous Monitoring**
```
1. Keep panel open during development
2. Implement optimization
3. Restart viewer
4. Immediately see impact in panel
5. Iterate without manual profiling
```

---

### Task 1.2: Fix Redundant Annotation Loading

**Priority:** CRITICAL (5-15ms improvement)
**Duration:** 2-3 days
**Risk:** LOW

#### Implementation Steps

**Step 1: Audit Current Usage (Day 1, Morning)**
```bash
# Find all call sites
rg "annotations\(" --type rust crates/viewer/re_data_ui/

# Expected output: lib.rs, annotation_context.rs, image.rs, component.rs, etc.
```

**Files to modify:**
- `crates/viewer/re_data_ui/src/lib.rs:144-153`
- All call sites using the helper

**Step 2: Refactor to Use Frame Cache (Day 1, Afternoon)**

```rust
// BEFORE: lib.rs:144-153
pub fn annotations(
    ctx: &ViewerContext<'_>,
    query: &LatestAtQuery,
    entity_path: &EntityPath,
) -> Arc<Annotations> {
    re_tracing::profile_function!();
    let mut annotation_map = AnnotationMap::default();
    annotation_map.load(ctx, query);  // 🔴 PROBLEM: Creates new map
    annotation_map.find(entity_path)
}

// AFTER: Use existing frame cache
pub fn annotations(
    ctx: &ViewerContext<'_>,
    _query: &LatestAtQuery,  // No longer needed
    entity_path: &EntityPath,
) -> Arc<Annotations> {
    re_tracing::profile_function!();
    // ✅ Reuse frame-cached AnnotationSceneContext
    ctx.annotation_scene_context()
        .0
        .find(entity_path)
}
```

**Step 3: Update Call Sites (Day 1, Evening)**
- Remove unused `query` parameter from call sites
- Simplify to single-line calls where possible
- Add profile scope to measure impact:
  ```rust
  re_tracing::profile_scope!("annotation_lookup_cached");
  ```

**Step 4: Add Instrumentation (Day 2, Morning)**
```rust
// In AnnotationSceneContext or AnnotationMap
static LOAD_CALLS: AtomicU64 = AtomicU64::new(0);

impl AnnotationMap {
    pub fn load(...) {
        LOAD_CALLS.fetch_add(1, Ordering::Relaxed);
        re_tracing::profile_function!();
        // ... existing logic
    }
}

// In app.rs or suitable location
pub fn report_annotation_stats() {
    let calls = LOAD_CALLS.swap(0, Ordering::Relaxed);
    if calls > 1 {
        re_log::warn!("AnnotationMap::load called {} times this frame (should be 1)", calls);
    }
}
```

**Step 5: Testing (Day 2, Afternoon)**
```rust
#[test]
fn test_annotation_loaded_once_per_frame() {
    // Setup viewer with annotated entities
    let mut viewer = setup_test_viewer_with_annotations(100);

    // Clear counter
    AnnotationMap::reset_stats();

    // Run one frame
    viewer.update();

    // Verify single load
    let load_count = AnnotationMap::load_calls();
    assert_eq!(load_count, 1, "Should load annotations once per frame, got {}", load_count);
}
```

**Step 6: Validation (Day 3)**
- Run air traffic 2h example
- Check puffin profiler: `AnnotationMap::load` should appear once per frame
- Measure frame time improvement: Expected 5-15ms reduction
- Verify no visual regressions in annotation display

#### Success Criteria
- ✅ `AnnotationMap::load` called exactly once per frame
- ✅ P95 frame time reduced by 5-15ms
- ✅ All annotation-based visualizations still work correctly
- ✅ Tests pass

#### Rollback Plan
If issues found:
1. Revert `lib.rs` changes
2. Keep instrumentation for debugging
3. Investigate cache invalidation timing

---

### Task 1.2: Implement Lazy Timeline Indexing

**Priority:** HIGH (1-3ms improvement)
**Duration:** 2-3 days
**Risk:** LOW

#### Implementation Steps

**Step 1: Analyze Current Indexing (Day 1, Morning)**
```bash
# Find eager indexing locations
rg "all_timelines" crates/store/re_tf/
rg "index_timeline" crates/store/re_tf/

# Check TODOs referencing issue #8233
rg "TODO.*8233|#8233" crates/store/re_tf/
```

**Files to modify:**
- `crates/store/re_tf/src/transform_resolution_cache.rs:777, 813`

**Step 2: Add Lazy Indexing State (Day 1, Afternoon)**

```rust
// In TransformResolutionCache
pub struct TransformResolutionCache {
    // ... existing fields ...

    /// Timelines that have been indexed
    indexed_timelines: HashSet<Timeline>,

    /// Static chunks pending indexing
    unindexed_static_chunks: HashSet<ChunkId>,
}

impl TransformResolutionCache {
    pub fn new() -> Self {
        Self {
            // ... existing initialization ...
            indexed_timelines: HashSet::new(),
            unindexed_static_chunks: HashSet::new(),
        }
    }
}
```

**Step 3: Defer Indexing to Query Time (Day 2, Morning)**

```rust
// BEFORE: add_static_chunk() - Line 777
fn add_static_chunk(&mut self, chunk: &Chunk) {
    // 🔴 PROBLEM: Indexes ALL timelines eagerly
    for timeline in all_timelines {
        self.index_timeline(timeline);
    }
}

// AFTER: Lazy indexing
fn add_static_chunk(&mut self, chunk: &Chunk) {
    re_tracing::profile_function!();
    // Just track that chunk exists, don't index yet
    self.unindexed_static_chunks.insert(chunk.id().clone());
}

fn ensure_timeline_indexed(&mut self, timeline: &Timeline) {
    if self.indexed_timelines.contains(timeline) {
        return;  // Already indexed
    }

    re_tracing::profile_scope!("lazy_index_timeline");

    // Index all pending chunks for this timeline
    for chunk_id in &self.unindexed_static_chunks {
        if let Some(chunk) = self.get_chunk(chunk_id) {
            self.index_chunk_for_timeline(chunk, timeline);
        }
    }

    self.indexed_timelines.insert(timeline.clone());
}

pub fn query(&mut self, timeline: &Timeline, ...) -> Result<...> {
    // Ensure timeline indexed before querying
    self.ensure_timeline_indexed(timeline);

    // ... rest of query logic ...
}
```

**Step 4: Add Metrics (Day 2, Afternoon)**
```rust
impl TransformResolutionCache {
    pub fn report_indexing_stats(&self) {
        re_log::debug!(
            "Transform cache: {}/{} timelines indexed, {} unindexed chunks",
            self.indexed_timelines.len(),
            self.total_available_timelines(),
            self.unindexed_static_chunks.len(),
        );
    }
}
```

**Step 5: Testing (Day 3, Morning)**
```rust
#[test]
fn test_lazy_timeline_indexing() {
    let mut cache = TransformResolutionCache::new();

    // Add static transforms
    cache.add_static_chunk(chunk1);
    cache.add_static_chunk(chunk2);

    // Should not have indexed any timelines yet
    assert_eq!(cache.indexed_timelines.len(), 0);

    // Query specific timeline
    cache.query(&timeline_a, ...);

    // Should have indexed only that timeline
    assert_eq!(cache.indexed_timelines.len(), 1);
    assert!(cache.indexed_timelines.contains(&timeline_a));

    // Query different timeline
    cache.query(&timeline_b, ...);

    // Should now have both indexed
    assert_eq!(cache.indexed_timelines.len(), 2);
}
```

**Step 6: Benchmark (Day 3, Afternoon)**
```rust
// Add to transform_resolution_cache_bench.rs
fn bench_lazy_vs_eager_indexing(c: &mut Criterion) {
    let mut group = c.benchmark_group("indexing");

    // Setup with 100 entities, 10 timelines
    let chunks = generate_static_chunks(100);
    let timelines = generate_timelines(10);

    group.bench_function("eager_indexing", |b| {
        b.iter(|| {
            let mut cache = TransformResolutionCache::new();
            for chunk in &chunks {
                cache.add_static_chunk_eager(chunk);  // Old behavior
            }
        });
    });

    group.bench_function("lazy_indexing", |b| {
        b.iter(|| {
            let mut cache = TransformResolutionCache::new();
            for chunk in &chunks {
                cache.add_static_chunk(chunk);  // New behavior
            }
            // Query only 2 timelines (realistic scenario)
            cache.query(&timelines[0], ...);
            cache.query(&timelines[1], ...);
        });
    });
}
```

#### Success Criteria
- ✅ Timelines indexed only when queried
- ✅ P95 frame time reduced by 1-3ms for static-heavy scenes
- ✅ Benchmark shows 50-80% reduction in indexing work for typical cases
- ✅ All transform resolution tests pass

#### Rollback Plan
If query correctness issues:
1. Add flag to toggle lazy vs eager indexing
2. Default to eager for safety
3. Investigate which timeline queries fail

---

### Task 1.3: Add Cache Effectiveness Monitoring

**Priority:** MEDIUM (enables data-driven optimization)
**Duration:** 2-3 days
**Risk:** VERY LOW

#### Implementation Steps

**Step 1: Define Cache Stats Structure (Day 1, Morning)**

```rust
// New file: crates/viewer/re_viewer_context/src/cache_stats.rs

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct CacheStats {
    pub hits: AtomicU64,
    pub misses: AtomicU64,
    pub invalidations: AtomicU64,
    pub memory_bytes: AtomicU64,
}

impl CacheStats {
    pub fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_invalidation(&self, count: usize) {
        self.invalidations.fetch_add(count as u64, Ordering::Relaxed);
    }

    pub fn hit_rate(&self) -> f64 {
        let hits = self.hits.load(Ordering::Relaxed) as f64;
        let misses = self.misses.load(Ordering::Relaxed) as f64;
        let total = hits + misses;
        if total > 0.0 {
            hits / total
        } else {
            0.0
        }
    }

    pub fn reset(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.invalidations.store(0, Ordering::Relaxed);
    }

    pub fn report(&self, name: &str) {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let invalidations = self.invalidations.load(Ordering::Relaxed);
        let memory_mb = self.memory_bytes.load(Ordering::Relaxed) as f64 / 1_000_000.0;

        re_log::debug!(
            "{}: {:.1}% hit rate ({} hits, {} misses, {} invalidations, {:.1} MB)",
            name,
            self.hit_rate() * 100.0,
            hits,
            misses,
            invalidations,
            memory_mb,
        );
    }
}
```

**Step 2: Integrate with QueryCache (Day 1, Afternoon)**

```rust
// In crates/store/re_query/src/cache.rs

pub struct QueryCache {
    // ... existing fields ...
    stats: CacheStats,
}

impl QueryCache {
    pub fn latest_at(...) -> Result<...> {
        // Check cache
        if let Some(cached) = self.try_get_cached(...) {
            self.stats.record_hit();
            return Ok(cached);
        }

        self.stats.record_miss();

        // Query store and cache result
        let result = self.query_store(...)?;
        self.cache_result(result);
        Ok(result)
    }

    pub fn invalidate(...) {
        let invalidated_count = self.invalidate_entries(...);
        self.stats.record_invalidation(invalidated_count);
    }

    pub fn report_stats(&self) {
        self.stats.report("QueryCache");
    }
}
```

**Step 3: Add to Other Caches (Day 2, Full Day)**

Integrate `CacheStats` into:
- `TransformResolutionCache`
- `ImageDecodeCache`
- `MeshCache`
- `TensorStatsCache`
- Any other performance-critical caches

**Step 4: Periodic Reporting (Day 3, Morning)**

```rust
// In app.rs main loop

pub struct App {
    // ... existing fields ...
    frame_count: u64,
    last_stats_report: web_time::Instant,
}

impl App {
    pub fn update(&mut self, ...) {
        self.frame_count += 1;

        // Report stats every 60 frames (~1 second at 60 FPS)
        if self.frame_count % 60 == 0 {
            self.report_cache_stats();
        }

        // ... rest of update logic ...
    }

    fn report_cache_stats(&self) {
        re_log::debug!("=== Cache Statistics (frame {}) ===", self.frame_count);

        // Query cache
        if let Some(query_cache) = self.store_hub.query_cache() {
            query_cache.report_stats();
        }

        // Transform cache
        if let Some(transform_cache) = self.transform_cache() {
            transform_cache.report_stats();
        }

        // Viewer caches
        self.caches.image_decode_cache.report_stats();
        self.caches.mesh_cache.report_stats();

        re_log::debug!("=================================");
    }
}
```

**Step 5: Testing (Day 3, Afternoon)**

```rust
#[test]
fn test_cache_stats_tracking() {
    let cache = QueryCache::new();

    // First query - should be miss
    cache.query(...);
    assert_eq!(cache.stats.hits.load(Ordering::Relaxed), 0);
    assert_eq!(cache.stats.misses.load(Ordering::Relaxed), 1);

    // Same query - should be hit
    cache.query(...);
    assert_eq!(cache.stats.hits.load(Ordering::Relaxed), 1);
    assert_eq!(cache.stats.misses.load(Ordering::Relaxed), 1);

    // Hit rate should be 50%
    assert_eq!(cache.stats.hit_rate(), 0.5);
}
```

#### Success Criteria
- ✅ Cache stats logged every ~1 second
- ✅ Hit rates visible for all major caches
- ✅ Invalidation counts tracked
- ✅ No measurable performance overhead from tracking

---

### Phase 1 Validation & Milestones

**Week 1 Checkpoint:**
- Tasks 1.1 and 1.2 completed
- Measure frame time improvement: Target 6-18ms reduction
- Expected P95: 22-24ms (down from ~30ms)

**Week 2 Checkpoint:**
- Task 1.3 completed (monitoring infrastructure)
- Full validation with air traffic 2h dataset
- **Phase 1 Success Criteria:**
  - ✅ P95 frame time < 20ms
  - ✅ No visual regressions
  - ✅ Cache monitoring shows >90% hit rates
  - ✅ All tests passing

**Deliverables:**
- [ ] PR #1: Fix redundant annotation loading
- [ ] PR #2: Lazy timeline indexing
- [ ] PR #3: Cache effectiveness monitoring
- [ ] Performance report comparing before/after metrics

---

## Phase 2: Architectural Improvements (3-4 weeks)

**Goal:** 50-60% cumulative frame time reduction
**Target Metric:** P95 frame time < 16ms (60 FPS)

### Task 2.1: Cache Blueprint Tree Construction

**Priority:** HIGH (2-20ms improvement)
**Duration:** 1 week
**Risk:** MEDIUM

#### Implementation Steps

**Step 1: Design Cache Key (Day 1)**

```rust
// In re_blueprint_tree/src/blueprint_tree.rs

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BlueprintTreeCacheKey {
    blueprint_generation: u64,
    recording_generation: u64,
    filter_string: String,
}

impl BlueprintTreeCacheKey {
    fn from_context(
        viewport_blueprint: &ViewportBlueprint,
        recording: &EntityDb,
        filter: &FilterMatcher,
    ) -> Self {
        Self {
            blueprint_generation: viewport_blueprint.generation(),
            recording_generation: recording.generation(),
            filter_string: filter.to_string(),
        }
    }
}
```

**Step 2: Add Cache to BlueprintTree (Days 2-3)**

```rust
pub struct BlueprintTree {
    // ... existing fields ...

    // Cache fields
    cached_tree: Option<BlueprintTreeData>,
    cache_key: Option<BlueprintTreeCacheKey>,
    cache_stats: CacheStats,
}

impl BlueprintTree {
    pub fn tree_ui(
        &mut self,
        ctx: &ViewerContext<'_>,
        viewport_blueprint: &ViewportBlueprint,
        ui: &mut egui::Ui,
    ) {
        re_tracing::profile_function!();

        // Generate cache key
        let current_key = BlueprintTreeCacheKey::from_context(
            viewport_blueprint,
            ctx.recording(),
            &self.filter_state.filter(),
        );

        // Check cache
        let blueprint_tree_data = if self.cache_key.as_ref() == Some(&current_key) {
            // Cache hit
            re_tracing::profile_scope!("blueprint_tree_cache_hit");
            self.cache_stats.record_hit();
            self.cached_tree.as_ref().unwrap()
        } else {
            // Cache miss - rebuild
            re_tracing::profile_scope!("blueprint_tree_cache_miss");
            self.cache_stats.record_miss();

            let tree = BlueprintTreeData::from_blueprint_and_filter(
                ctx,
                viewport_blueprint,
                &self.filter_state.filter(),
            );

            self.cached_tree = Some(tree);
            self.cache_key = Some(current_key);
            self.cached_tree.as_ref().unwrap()
        };

        // Render UI using cached tree
        self.tree_ui_impl(ctx, viewport_blueprint, blueprint_tree_data, ui);
    }
}
```

**Step 3: Invalidation Strategy (Day 4)**

```rust
impl BlueprintTree {
    pub fn invalidate_cache(&mut self, reason: &str) {
        if self.cached_tree.is_some() {
            re_log::trace!("Invalidating blueprint tree cache: {}", reason);
            self.cached_tree = None;
            self.cache_key = None;
        }
    }

    // Called when user modifies blueprint
    pub fn on_blueprint_modified(&mut self) {
        self.invalidate_cache("blueprint modified");
    }

    // Called when filter changes
    pub fn on_filter_changed(&mut self) {
        self.invalidate_cache("filter changed");
    }
}
```

**Step 4: Testing (Day 5)**

```rust
#[test]
fn test_blueprint_tree_caching() {
    let mut tree = BlueprintTree::new();
    let ctx = setup_test_context();

    // First render - cache miss
    tree.tree_ui(&ctx, &viewport, &mut ui);
    assert_eq!(tree.cache_stats.misses.load(Ordering::Relaxed), 1);

    // Second render with no changes - cache hit
    tree.tree_ui(&ctx, &viewport, &mut ui);
    assert_eq!(tree.cache_stats.hits.load(Ordering::Relaxed), 1);

    // Modify blueprint - should invalidate
    viewport.modify_container(...);
    tree.on_blueprint_modified();
    assert!(tree.cached_tree.is_none());

    // Next render - cache miss again
    tree.tree_ui(&ctx, &viewport, &mut ui);
    assert_eq!(tree.cache_stats.misses.load(Ordering::Relaxed), 2);
}

#[test]
fn test_blueprint_tree_cache_effectiveness() {
    let mut tree = BlueprintTree::new();
    let ctx = setup_test_context();

    // Simulate 100 frames with no changes
    for _ in 0..100 {
        tree.tree_ui(&ctx, &viewport, &mut ui);
    }

    // Should have 1 miss (first frame) and 99 hits
    assert_eq!(tree.cache_stats.misses.load(Ordering::Relaxed), 1);
    assert_eq!(tree.cache_stats.hits.load(Ordering::Relaxed), 99);

    let hit_rate = tree.cache_stats.hit_rate();
    assert!(hit_rate > 0.98, "Hit rate too low: {:.1}%", hit_rate * 100.0);
}
```

#### Success Criteria
- ✅ Cache hit rate >95% during normal operation
- ✅ Frame time reduced by 2-20ms (layout dependent)
- ✅ Invalidation correctly triggered on blueprint/filter changes
- ✅ UI still responsive to all user interactions

---

### Task 2.2: Shared Entity Tree Walk

**Priority:** HIGH (3-7ms improvement)
**Duration:** 1-2 weeks
**Risk:** MEDIUM-HIGH

#### Implementation Steps

**Step 1: Design Shared Walk API (Days 1-2)**

```rust
// New file: crates/viewer/re_viewport_blueprint/src/shared_entity_walk.rs

pub struct SharedEntityWalk {
    /// All entities encountered in tree walk
    all_entities: Vec<EntityPath>,

    /// Which views care about each entity
    /// Map: EntityPath -> Set of ViewClassIds
    visualizability: HashMap<EntityPath, HashSet<ViewClassIdentifier>>,

    /// When was this walk performed
    recording_generation: u64,
}

impl SharedEntityWalk {
    pub fn execute(
        ctx: &ViewerContext<'_>,
        view_classes: &[ViewClassIdentifier],
    ) -> Self {
        re_tracing::profile_function!();

        let mut all_entities = Vec::new();
        let mut visualizability = HashMap::new();

        // Single tree walk
        ctx.recording().tree().visit(&mut |entity_path, _entity_tree| {
            all_entities.push(entity_path.clone());

            // Determine which view classes care about this entity
            let mut applicable_views = HashSet::new();
            for view_class_id in view_classes {
                if is_applicable_to_view(ctx, entity_path, view_class_id) {
                    applicable_views.insert(*view_class_id);
                }
            }

            if !applicable_views.is_empty() {
                visualizability.insert(entity_path.clone(), applicable_views);
            }
        });

        Self {
            all_entities,
            visualizability,
            recording_generation: ctx.recording().generation(),
        }
    }

    pub fn filter_for_view(
        &self,
        view_class_id: ViewClassIdentifier,
        filter: &EntityPathFilter,
    ) -> Vec<EntityPath> {
        re_tracing::profile_scope!("filter_shared_walk");

        self.all_entities
            .iter()
            .filter(|entity_path| {
                // Check if view cares about this entity
                let applicable = self.visualizability
                    .get(*entity_path)
                    .map_or(false, |views| views.contains(&view_class_id));

                // Check against view's filter
                applicable && filter.matches(entity_path)
            })
            .cloned()
            .collect()
    }

    pub fn is_current(&self, ctx: &ViewerContext<'_>) -> bool {
        self.recording_generation == ctx.recording().generation()
    }
}
```

**Step 2: Integrate with View Query Execution (Days 3-5)**

```rust
// In app_state.rs

pub struct ViewportState {
    // ... existing fields ...

    /// Cached shared entity walk
    shared_walk: Option<SharedEntityWalk>,
}

impl ViewportState {
    fn execute_view_queries(
        &mut self,
        ctx: &ViewerContext<'_>,
        views: &[ViewBlueprint],
    ) {
        re_tracing::profile_function!();

        // Get or create shared walk
        let shared_walk = if let Some(walk) = &self.shared_walk {
            if walk.is_current(ctx) {
                walk  // Reuse existing walk
            } else {
                // Stale - rebuild
                let view_classes = self.collect_view_classes(views);
                let walk = SharedEntityWalk::execute(ctx, &view_classes);
                self.shared_walk = Some(walk);
                self.shared_walk.as_ref().unwrap()
            }
        } else {
            // First time - create
            let view_classes = self.collect_view_classes(views);
            let walk = SharedEntityWalk::execute(ctx, &view_classes);
            self.shared_walk = Some(walk);
            self.shared_walk.as_ref().unwrap()
        };

        // Execute per-view queries using shared walk
        for view in views {
            let relevant_entities = shared_walk.filter_for_view(
                view.class_identifier(),
                &view.contents.entity_path_filter,
            );

            view.execute_query_with_entities(ctx, relevant_entities);
        }
    }
}
```

**Step 3: Benchmark (Day 6)**

```rust
// Add to data_query.rs benchmark

fn bench_shared_vs_per_view_walk(c: &mut Criterion) {
    let mut group = c.benchmark_group("entity_walk");

    let (recording, views) = setup_test_data(
        num_entities: 10_000,
        num_views: 5,
    );

    group.bench_function("current_per_view", |b| {
        b.iter(|| {
            for view in &views {
                // Each view walks tree independently
                view.execute_query_current(recording);
            }
        });
    });

    group.bench_function("shared_walk", |b| {
        b.iter(|| {
            // Walk once
            let shared_walk = SharedEntityWalk::execute(&ctx, &view_classes);

            // Filter per view
            for view in &views {
                let entities = shared_walk.filter_for_view(
                    view.class_id(),
                    &view.filter,
                );
                view.execute_query_with_entities(entities);
            }
        });
    });
}
```

**Step 4: Testing (Day 7)**

```rust
#[test]
fn test_shared_walk_correctness() {
    let recording = build_test_recording(1000);
    let views = vec![
        create_3d_view("+ /**"),
        create_2d_view("+ /camera/**"),
        create_plot_view("+ /plots/**"),
    ];

    // Execute with shared walk
    let shared_walk = SharedEntityWalk::execute(&ctx, &view_classes);
    let shared_results: Vec<_> = views.iter()
        .map(|v| shared_walk.filter_for_view(v.class_id(), &v.filter))
        .collect();

    // Execute old way (per-view)
    let per_view_results: Vec<_> = views.iter()
        .map(|v| v.execute_query_old_way(&recording))
        .collect();

    // Results should be identical
    for (i, (shared, per_view)) in shared_results.iter()
        .zip(per_view_results.iter())
        .enumerate()
    {
        assert_eq!(
            shared, per_view,
            "View {} results differ between shared and per-view walk",
            i
        );
    }
}
```

#### Success Criteria
- ✅ Frame time reduced by 3-7ms
- ✅ Benchmark shows N×speedup where N=number of views
- ✅ Query results identical to per-view approach
- ✅ All view types render correctly

---

### Task 2.3: Smarter Transform Invalidation

**Priority:** MEDIUM (10-50% cache speedup)
**Duration:** 1-2 weeks
**Risk:** MEDIUM

#### Implementation Steps

**Step 1: Analyze Shadowing Patterns (Days 1-2)**

```rust
// Research: Document transform shadowing semantics
// - When does a new transform shadow previous ones?
// - What are the invalidation rules?
// - Edge cases to handle?
```

**Step 2: Implement Shadowing-Aware Invalidation (Days 3-7)**

```rust
// In transform_resolution_cache.rs

pub struct TransformShadowTracker {
    /// Times when transforms exist for each entity
    /// Sorted for binary search
    transform_times: BTreeMap<EntityPath, BTreeSet<TimeInt>>,
}

impl TransformShadowTracker {
    fn add_transform(&mut self, entity: &EntityPath, time: TimeInt) {
        self.transform_times
            .entry(entity.clone())
            .or_default()
            .insert(time);
    }

    fn next_shadowing_time(
        &self,
        entity: &EntityPath,
        after_time: TimeInt,
    ) -> Option<TimeInt> {
        self.transform_times
            .get(entity)?
            .range((Bound::Excluded(after_time), Bound::Unbounded))
            .next()
            .copied()
    }
}

impl TransformResolutionCache {
    fn invalidate_with_shadowing(
        &mut self,
        entity: &EntityPath,
        changed_time: TimeInt,
    ) {
        re_tracing::profile_function!();

        // Find next shadowing time
        let shadow_tracker = &self.shadow_tracker;
        let next_shadow = shadow_tracker.next_shadowing_time(entity, changed_time);

        match next_shadow {
            Some(next) => {
                // Invalidate only up to next shadow
                re_tracing::profile_scope!("invalidate_to_shadow");
                self.invalidate_time_range(entity, changed_time..next);
            }
            None => {
                // No future shadow - invalidate to end
                re_tracing::profile_scope!("invalidate_to_end");
                self.invalidate_time_range(entity, changed_time..TimeInt::MAX);
            }
        }
    }
}
```

**Step 3: Add Comprehensive Tests (Days 8-10)**

```rust
#[test]
fn test_shadowed_invalidation() {
    let mut cache = TransformResolutionCache::new();

    // Add transforms at t=0, t=100, t=200
    cache.add_transform(entity, t0, transform_a);
    cache.add_transform(entity, t100, transform_b);
    cache.add_transform(entity, t200, transform_c);

    // Query and cache at t=50, t=150, t=250
    cache.query(entity, t50);
    cache.query(entity, t150);
    cache.query(entity, t250);

    // All should be cached
    assert!(cache.is_cached(entity, t50));
    assert!(cache.is_cached(entity, t150));
    assert!(cache.is_cached(entity, t250));

    // Update transform at t=100
    cache.add_transform(entity, t100, transform_b_updated);

    // t=50 should still be valid (uses transform_a at t=0)
    assert!(cache.is_cached(entity, t50));

    // t=150 should be invalidated (uses transform_b at t=100)
    assert!(!cache.is_cached(entity, t150));

    // t=250 should still be valid (uses transform_c at t=200, not affected)
    assert!(cache.is_cached(entity, t250));
}

#[test]
fn test_scrubbing_performance() {
    let mut cache = TransformResolutionCache::new();

    // Setup: 1000 entities, transforms every 100ms, 2 hour recording
    for entity_idx in 0..1000 {
        for time_ms in (0..7_200_000).step_by(100) {
            cache.add_transform(
                entity(entity_idx),
                TimeInt::from_millis(time_ms),
                random_transform(),
            );
        }
    }

    // Simulate scrubbing: Query at many different times
    let scrub_times: Vec<_> = (0..7_200_000).step_by(1000).collect();

    let start = Instant::now();
    for time_ms in scrub_times {
        for entity_idx in 0..100 {  // Sample of entities
            cache.query(entity(entity_idx), TimeInt::from_millis(time_ms));
        }
    }
    let elapsed = start.elapsed();

    // With smart invalidation, this should be fast
    assert!(elapsed < Duration::from_secs(1), "Scrubbing too slow: {:?}", elapsed);
}
```

**Step 4: Benchmark Impact (Day 11-12)**

Add to `transform_resolution_cache_bench.rs`:
- Cold cache performance
- Warm cache performance
- Scrubbing scenario (query at many times)
- Invalidation overhead

#### Success Criteria
- ✅ Scrubbing scenario 2-5x faster
- ✅ Cache rebuild time reduced by 10-50%
- ✅ All edge cases handled correctly
- ✅ No regressions in transform correctness

---

### Phase 2 Validation & Milestones

**Week 3-4 Checkpoint:**
- Tasks 2.1 and 2.2 completed
- Measure cumulative improvement: Target 50% reduction from baseline
- Expected P95: 15-18ms

**Week 5-6 Checkpoint:**
- Task 2.3 completed
- Full validation with air traffic 2h dataset
- **Phase 2 Success Criteria:**
  - ✅ P95 frame time < 16ms (60 FPS achieved!)
  - ✅ Web viewer performance acceptable
  - ✅ All tests passing
  - ✅ No visual or interaction regressions

**Deliverables:**
- [ ] PR #4: Blueprint tree caching
- [ ] PR #5: Shared entity tree walk
- [ ] PR #6: Smarter transform invalidation
- [ ] Performance report showing 60 FPS achievement

---

## Phase 3: Long-term Optimizations (1-2 months)

**Goal:** Sustained 60 FPS for extreme scenarios
**Target Metric:** P95 < 14ms, handles Bevy "Alien Cake Addict" in real-time

### Task 3.1: Incremental UI Updates

**Priority:** MEDIUM (3-6ms improvement for time series)
**Duration:** 2-3 weeks
**Risk:** HIGH (major architectural change)

#### Overview
Replace egui immediate-mode rendering with retained-mode approach for time series views.

#### Implementation Steps (High-Level)

**Phase 3.1.1: Data Change Detection (Week 1)**
- Add generation tracking to time series data
- Implement change detection heuristics
- Profile to identify which views change frame-to-frame

**Phase 3.1.2: GPU Line Rendering (Week 2)**
- Use `re_renderer::LineRenderer` for plot lines
- Upload line data once, reuse GPU buffers
- Implement LOD (level of detail) for dense plots

**Phase 3.1.3: Integration & Testing (Week 3)**
- Integrate with existing time series views
- Benchmark before/after
- Comprehensive correctness testing

#### Success Criteria
- ✅ Frame time reduced by 3-6ms for time series heavy scenes
- ✅ Plot rendering quality maintained or improved
- ✅ Interaction still smooth (panning, zooming)

---

### Task 3.2: Viewport-Aware Culling

**Priority:** MEDIUM (variable impact)
**Duration:** 2-3 weeks
**Risk:** MEDIUM

#### Overview
Only process entities visible in viewport/time range, breaking linear scaling with dataset size.

#### Implementation Steps (High-Level)

**Phase 3.2.1: Spatial Indexing (Week 1-2)**
- Implement spatial index (R-tree or similar) for 3D/2D views
- Index entities by bounding box
- Query index based on viewport

**Phase 3.2.2: Temporal Filtering (Week 2-3)**
- Filter entities based on time range visibility
- Short-circuit queries outside visible range
- Maintain index for active time window

**Phase 3.2.3: Integration (Week 3)**
- Integrate with query execution
- Handle dynamic viewport changes
- Test with extreme datasets (100K+ entities)

#### Success Criteria
- ✅ Performance bounded by viewport, not dataset size
- ✅ 100K+ entity scenes remain at 60 FPS
- ✅ Smooth viewport navigation

---

### Task 3.3: End-to-End Performance Test Suite

**Priority:** HIGH (regression prevention)
**Duration:** 1 week
**Risk:** LOW

#### Implementation Steps

**Step 1: Define Test Scenarios (Day 1-2)**
```rust
// tests/performance/scenarios.rs

pub struct PerformanceScenario {
    name: String,
    dataset: Dataset,
    target_p95: Duration,
    target_p99: Duration,
}

pub fn standard_scenarios() -> Vec<PerformanceScenario> {
    vec![
        PerformanceScenario {
            name: "air_traffic_2h".into(),
            dataset: Dataset::AirTraffic2H,
            target_p95: Duration::from_millis(16),
            target_p99: Duration::from_millis(20),
        },
        PerformanceScenario {
            name: "many_entities_1000".into(),
            dataset: Dataset::ManyEntities(1000),
            target_p95: Duration::from_millis(16),
            target_p99: Duration::from_millis(20),
        },
        // ... more scenarios
    ]
}
```

**Step 2: Implement Test Runner (Day 3-4)**
```rust
#[test]
#[ignore]  // Only run with --ignored flag
fn test_performance_regressions() {
    for scenario in standard_scenarios() {
        println!("Testing scenario: {}", scenario.name);

        let viewer = setup_viewer_with_dataset(scenario.dataset);
        let frame_times = measure_frame_times(&viewer, num_frames: 100);

        let p95 = percentile(&frame_times, 0.95);
        let p99 = percentile(&frame_times, 0.99);

        println!("  P95: {:?} (target: {:?})", p95, scenario.target_p95);
        println!("  P99: {:?} (target: {:?})", p99, scenario.target_p99);

        assert!(
            p95 <= scenario.target_p95,
            "{}: P95 regression: {:?} > {:?}",
            scenario.name, p95, scenario.target_p95
        );

        assert!(
            p99 <= scenario.target_p99,
            "{}: P99 regression: {:?} > {:?}",
            scenario.name, p99, scenario.target_p99
        );
    }
}
```

**Step 3: CI Integration (Day 5-7)**
- Add performance test job to CI
- Run on performance-sensitive PRs
- Generate comparison reports
- Alert on regressions

#### Success Criteria
- ✅ Automated performance tests in CI
- ✅ Catches regressions before merge
- ✅ Historical trend tracking

---

### Phase 3 Validation & Milestones

**Month 2-3 Checkpoint:**
- Tasks 3.1 and 3.2 completed
- Extreme scenario testing
- **Phase 3 Success Criteria:**
  - ✅ P95 frame time < 14ms
  - ✅ Bevy example runs smoothly
  - ✅ 100K+ entity scenes at 60 FPS
  - ✅ Performance test suite in CI

**Final Deliverables:**
- [ ] PR #7: Incremental UI updates
- [ ] PR #8: Viewport-aware culling
- [ ] PR #9: Performance test suite
- [ ] Final performance report
- [ ] Documentation updates

---

## Testing Strategy

### Unit Testing
- Test each optimization in isolation
- Verify correctness before performance
- Use property-based testing where applicable

### Integration Testing
- Test interactions between optimizations
- Ensure caches invalidate correctly
- Verify no regressions in functionality

### Performance Testing
- Benchmark before and after each change
- Use consistent hardware/environment
- Track multiple percentiles (P50, P95, P99)

### Visual Regression Testing
- Capture reference screenshots
- Compare after each optimization
- Flag any visual differences

---

## Measurement & Validation

### Metrics to Track

| Metric | Measurement | Target |
|--------|-------------|--------|
| P50 frame time | Median of 100 frames | <13ms |
| P95 frame time | 95th percentile | <16ms |
| P99 frame time | 99th percentile | <20ms |
| Cache hit rate (query) | Hits / (Hits + Misses) | >90% |
| Cache hit rate (blueprint) | Hits / (Hits + Misses) | >95% |
| Memory usage | RSS peak | <2× current |

### Validation Process

**Per-Task Validation:**
1. Run benchmarks before change
2. Implement change
3. Run benchmarks after change
4. Calculate improvement percentage
5. Verify no regressions in correctness tests

**Phase Validation:**
1. Full air traffic 2h run
2. Collect 100+ frame samples
3. Calculate statistics
4. Compare against phase targets
5. Profile with puffin viewer
6. Review cache effectiveness

**Final Validation:**
1. All performance tests pass
2. No visual regressions
3. All scenarios meet targets
4. Documentation updated
5. Team review and sign-off

---

## Risk Management

### Risk: Performance Optimization Introduces Bugs

**Mitigation:**
- Comprehensive testing at each step
- Feature flags to toggle optimizations
- Gradual rollout (test on subset of users)
- Keep old code path for comparison

**Rollback Plan:**
- Each optimization in separate PR
- Can revert individual changes
- Feature flags allow disabling without code changes

### Risk: Cache Invalidation Bugs

**Mitigation:**
- Extensive testing of invalidation scenarios
- Logging and monitoring of cache behavior
- Assert cache consistency in debug builds
- Fuzz testing with random operations

**Detection:**
- Visual regressions caught by screenshot comparisons
- Correctness tests verify query results
- User reports (beta testing phase)

### Risk: Memory Usage Increases

**Mitigation:**
- Track memory usage in benchmarks
- Set maximum cache sizes
- Implement cache eviction policies
- Monitor in production

**Rollback Plan:**
- Can reduce cache sizes if needed
- Can disable caches with feature flags
- Memory profiling to identify culprits

### Risk: Architectural Changes Break Extensions

**Mitigation:**
- Document API changes thoroughly
- Deprecate before removing
- Provide migration guides
- Test with known extensions

**Communication:**
- Announce changes in advance
- Provide migration timeline
- Offer support during transition

---

## Success Criteria Summary

### Phase 1 Success (Week 2)
- ✅ P95 frame time < 20ms
- ✅ Annotation loading: 1 call/frame
- ✅ Timeline indexing: only when queried
- ✅ Cache monitoring infrastructure in place

### Phase 2 Success (Week 6)
- ✅ P95 frame time < 16ms (60 FPS!)
- ✅ Blueprint tree cache hit rate >95%
- ✅ Shared entity walk working
- ✅ Smart transform invalidation functional

### Phase 3 Success (Week 12)
- ✅ P95 frame time < 14ms
- ✅ Bevy example at 60 FPS
- ✅ 100K+ entities supported
- ✅ Performance test suite in CI

### Overall Success
- ✅ Air traffic 2h visualizes smoothly on web
- ✅ Native viewer handles infinite time ranges
- ✅ All issue #8233 goals met
- ✅ No functionality regressions
- ✅ Team satisfied with results

---

## Appendix: Quick Reference

### Performance Targets

| Dataset | Current P95 | Phase 1 | Phase 2 | Phase 3 |
|---------|-------------|---------|---------|---------|
| Air traffic 2h | ~30ms | <20ms | <16ms | <14ms |
| Many entities (1K) | ~25ms | <18ms | <15ms | <13ms |
| Many entities (10K) | ~50ms | <35ms | <25ms | <20ms |

### Key Files to Modify

| Optimization | Files |
|--------------|-------|
| Annotation loading | `re_data_ui/src/lib.rs:144-153` |
| Lazy timeline indexing | `re_tf/src/transform_resolution_cache.rs:777,813` |
| Blueprint tree caching | `re_blueprint_tree/src/blueprint_tree.rs:134` |
| Shared entity walk | `re_viewport_blueprint/src/view_contents.rs:298-305` |
| Transform invalidation | `re_tf/src/transform_resolution_cache.rs:507` |

### Profiling Commands

```bash
# Start viewer with profiling
cargo run --release -- --profiling

# Connect puffin viewer
puffin_viewer

# Run performance benchmarks
cargo bench --bench data_query
cargo bench --bench transform_resolution_cache_bench

# Run performance tests
cargo test --test performance_regressions --release --ignored
```

---

**Document Version:** 1.0
**Last Updated:** November 8, 2025
**Total Implementation Time:** 8-12 weeks
**Phases:** 3
**Tasks:** 12
**Expected Improvement:** 50-60% frame time reduction
