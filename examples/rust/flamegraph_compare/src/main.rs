//! Visual flamegraph comparison tool
//!
//! Compare two flamegraphs (baseline vs optimized) with an interactive GUI.
//!
//! Run with:
//! ```sh
//! cargo run -p flamegraph_compare -- baseline.svg optimized.svg
//! ```

mod parser;

use clap::Parser as _;
use eframe::egui;
use egui::{Color32, RichText};
use egui_extras::{Column, TableBuilder};
use parser::{FlameGraphData, FunctionComparison};

// Helper function to format numbers with thousand separators
fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if i > 0 && (chars.len() - i) % 3 == 0 {
            result.push(',');
        }
        result.push(*ch);
    }
    result
}

#[derive(clap::Parser)]
#[clap(about = "Visual flamegraph comparison tool")]
struct Args {
    /// Path to the baseline flamegraph file
    #[clap(value_name = "BASELINE")]
    baseline: Option<std::path::PathBuf>,

    /// Path to the optimized flamegraph file
    #[clap(value_name = "OPTIMIZED")]
    optimized: Option<std::path::PathBuf>,
}

fn main() -> eframe::Result {
    re_log::setup_logging();

    let args = Args::parse();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("flamegraph_compare")
            .with_inner_size([1400.0, 900.0])
            .with_title("Flamegraph Comparison Tool"),
        ..Default::default()
    };

    eframe::run_native(
        "Flamegraph Comparison",
        native_options,
        Box::new(move |cc| {
            re_ui::apply_style_and_install_loaders(&cc.egui_ctx);
            Ok(Box::new(FlameGraphCompareApp::new(
                cc,
                args.baseline,
                args.optimized,
            )))
        }),
    )
}

struct FlameGraphCompareApp {
    baseline_path: Option<std::path::PathBuf>,
    optimized_path: Option<std::path::PathBuf>,
    baseline_data: Option<FlameGraphData>,
    optimized_data: Option<FlameGraphData>,
    comparisons: Vec<FunctionComparison>,
    error_message: Option<String>,
    search_query: String,
    sort_by: SortBy,
    sort_ascending: bool,
    show_only_regressions: bool,
    show_only_improvements: bool,
    min_threshold_pct: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortBy {
    Name,
    BaselineTotal,
    OptimizedTotal,
    TotalChange,
    SelfChange,
}

impl FlameGraphCompareApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        baseline_path: Option<std::path::PathBuf>,
        optimized_path: Option<std::path::PathBuf>,
    ) -> Self {
        let mut app = Self {
            baseline_path,
            optimized_path,
            baseline_data: None,
            optimized_data: None,
            comparisons: Vec::new(),
            error_message: None,
            search_query: String::new(),
            sort_by: SortBy::TotalChange,
            sort_ascending: false,
            show_only_regressions: false,
            show_only_improvements: false,
            min_threshold_pct: 0.0,
        };

        // If files were provided via command line, load them
        if app.baseline_path.is_some() && app.optimized_path.is_some() {
            app.load_flamegraphs();
        }

        app
    }

    fn load_flamegraphs(&mut self) {
        self.error_message = None;

        let Some(baseline_path) = &self.baseline_path else {
            self.error_message = Some("No baseline file selected".to_owned());
            return;
        };

        let Some(optimized_path) = &self.optimized_path else {
            self.error_message = Some("No optimized file selected".to_owned());
            return;
        };

        match parser::parse_flamegraph(baseline_path) {
            Ok(data) => {
                self.baseline_data = Some(data);
            }
            Err(e) => {
                self.error_message = Some(format!("Error loading baseline: {e}"));
                return;
            }
        }

        match parser::parse_flamegraph(optimized_path) {
            Ok(data) => {
                self.optimized_data = Some(data);
            }
            Err(e) => {
                self.error_message = Some(format!("Error loading optimized: {e}"));
                return;
            }
        }

        // Perform comparison
        if let (Some(baseline), Some(optimized)) = (&self.baseline_data, &self.optimized_data) {
            self.comparisons = parser::compare_flamegraphs(baseline, optimized);
            self.sort_comparisons();
        }
    }

    fn sort_comparisons(&mut self) {
        let ascending = self.sort_ascending;
        let sort_by = self.sort_by;

        self.comparisons.sort_by(|a, b| {
            let cmp = match sort_by {
                SortBy::Name => a.name.cmp(&b.name),
                SortBy::BaselineTotal => a
                    .baseline_total_pct
                    .partial_cmp(&b.baseline_total_pct)
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortBy::OptimizedTotal => a
                    .optimized_total_pct
                    .partial_cmp(&b.optimized_total_pct)
                    .unwrap_or(std::cmp::Ordering::Equal),
                SortBy::TotalChange => {
                    let a_change = if a.total_change_pct.is_finite() {
                        a.total_change_pct.abs()
                    } else {
                        f64::MAX
                    };
                    let b_change = if b.total_change_pct.is_finite() {
                        b.total_change_pct.abs()
                    } else {
                        f64::MAX
                    };
                    b_change
                        .partial_cmp(&a_change)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }
                SortBy::SelfChange => {
                    let a_change = if a.self_change_pct.is_finite() {
                        a.self_change_pct.abs()
                    } else {
                        f64::MAX
                    };
                    let b_change = if b.self_change_pct.is_finite() {
                        b.self_change_pct.abs()
                    } else {
                        f64::MAX
                    };
                    b_change
                        .partial_cmp(&a_change)
                        .unwrap_or(std::cmp::Ordering::Equal)
                }
            };

            if ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });
    }

    fn ui_file_picker(&mut self, ui: &mut egui::Ui) {
        ui.heading("Load Flamegraphs");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("Baseline:");
            if let Some(path) = &self.baseline_path {
                ui.label(path.display().to_string());
            } else {
                ui.label(RichText::new("No file selected").italics());
            }
            if ui.button("📁 Browse").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Flamegraph", &["svg", "txt", "json"])
                    .pick_file()
                {
                    self.baseline_path = Some(path);
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("Optimized:");
            if let Some(path) = &self.optimized_path {
                ui.label(path.display().to_string());
            } else {
                ui.label(RichText::new("No file selected").italics());
            }
            if ui.button("📁 Browse").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Flamegraph", &["svg", "txt", "json"])
                    .pick_file()
                {
                    self.optimized_path = Some(path);
                }
            }
        });

        ui.add_space(10.0);

        ui.horizontal(|ui| {
            let can_load = self.baseline_path.is_some() && self.optimized_path.is_some();
            if ui
                .add_enabled(can_load, egui::Button::new("🔄 Load & Compare"))
                .clicked()
            {
                self.load_flamegraphs();
            }

            if ui.button("❌ Clear").clicked() {
                self.baseline_path = None;
                self.optimized_path = None;
                self.baseline_data = None;
                self.optimized_data = None;
                self.comparisons.clear();
                self.error_message = None;
            }
        });

        if let Some(error) = &self.error_message {
            ui.add_space(10.0);
            ui.colored_label(Color32::RED, format!("❌ Error: {error}"));
        }
    }

    fn ui_summary(&self, ui: &mut egui::Ui) {
        let Some(baseline) = &self.baseline_data else {
            return;
        };
        let Some(optimized) = &self.optimized_data else {
            return;
        };

        ui.heading("Summary");
        ui.add_space(10.0);

        let overall_change = ((optimized.total_samples as f64 - baseline.total_samples as f64)
            / baseline.total_samples as f64)
            * 100.0;

        ui.horizontal(|ui| {
            ui.label("Baseline samples:");
            ui.label(RichText::new(format_with_commas(baseline.total_samples)).strong());
        });

        ui.horizontal(|ui| {
            ui.label("Optimized samples:");
            ui.label(RichText::new(format_with_commas(optimized.total_samples)).strong());
        });

        ui.horizontal(|ui| {
            ui.label("Overall change:");
            let color = if overall_change < 0.0 {
                Color32::from_rgb(0, 180, 0) // Green for improvement
            } else if overall_change > 0.0 {
                Color32::from_rgb(255, 100, 100) // Red for regression
            } else {
                Color32::GRAY
            };
            ui.colored_label(
                color,
                RichText::new(format!("{overall_change:+.2}%")).strong(),
            );
            if overall_change < -5.0 {
                ui.label("🎉 Significant Improvement");
            } else if overall_change < 0.0 {
                ui.label("✓ Minor Improvement");
            } else if overall_change > 5.0 {
                ui.label("⚠ Significant Regression");
            } else if overall_change > 0.0 {
                ui.label("⚠ Minor Regression");
            }
        });

        ui.add_space(5.0);

        let improvements = self
            .comparisons
            .iter()
            .filter(|c| c.total_change_pct < 0.0 && c.total_change_pct.is_finite())
            .count();
        let regressions = self
            .comparisons
            .iter()
            .filter(|c| c.total_change_pct > 0.0 && c.total_change_pct.is_finite())
            .count();

        ui.horizontal(|ui| {
            ui.label("Functions analyzed:");
            ui.label(format!("{}", self.comparisons.len()));
        });

        ui.horizontal(|ui| {
            ui.colored_label(Color32::from_rgb(0, 180, 0), format!("↓ {improvements}"));
            ui.label("improvements,");
            ui.colored_label(Color32::from_rgb(255, 100, 100), format!("↑ {regressions}"));
            ui.label("regressions");
        });
    }

    fn ui_filters(&mut self, ui: &mut egui::Ui) {
        ui.heading("Filters");
        ui.add_space(10.0);

        ui.horizontal(|ui| {
            ui.label("🔍 Search:");
            ui.text_edit_singleline(&mut self.search_query);
        });

        ui.add_space(5.0);

        ui.checkbox(&mut self.show_only_improvements, "Show only improvements");
        ui.checkbox(&mut self.show_only_regressions, "Show only regressions");

        ui.add_space(5.0);

        ui.horizontal(|ui| {
            ui.label("Min change threshold:");
            ui.add(egui::Slider::new(&mut self.min_threshold_pct, 0.0..=50.0).suffix("%"));
        });
    }

    fn ui_comparison_table(&mut self, ui: &mut egui::Ui) {
        // Track sort changes
        let mut need_resort = false;
        let mut new_sort_by = self.sort_by;
        let mut new_sort_ascending = self.sort_ascending;

        let filtered_comparisons: Vec<&FunctionComparison> = self
            .comparisons
            .iter()
            .filter(|c| {
                // Apply search filter
                if !self.search_query.is_empty()
                    && !c
                        .name
                        .to_lowercase()
                        .contains(&self.search_query.to_lowercase())
                {
                    return false;
                }

                // Apply improvement/regression filters
                if self.show_only_improvements && c.total_change_pct >= 0.0 {
                    return false;
                }
                if self.show_only_regressions && c.total_change_pct <= 0.0 {
                    return false;
                }

                // Apply threshold filter
                if c.total_change_pct.is_finite()
                    && c.total_change_pct.abs() < self.min_threshold_pct as f64
                {
                    return false;
                }

                true
            })
            .collect();

        ui.heading(format!("Comparison ({} functions)", filtered_comparisons.len()));
        ui.add_space(5.0);

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::auto().at_least(250.0)) // Function name
            .column(Column::auto().at_least(80.0)) // Baseline %
            .column(Column::auto().at_least(80.0)) // Optimized %
            .column(Column::auto().at_least(80.0)) // Change %
            .column(Column::auto().at_least(100.0)) // Visual bar
            .header(20.0, |mut header| {
                header.col(|ui| {
                    if ui.button("Function Name").clicked() {
                        if new_sort_by == SortBy::Name {
                            new_sort_ascending = !new_sort_ascending;
                        } else {
                            new_sort_by = SortBy::Name;
                            new_sort_ascending = true;
                        }
                        need_resort = true;
                    }
                });
                header.col(|ui| {
                    if ui.button("Baseline %").clicked() {
                        if new_sort_by == SortBy::BaselineTotal {
                            new_sort_ascending = !new_sort_ascending;
                        } else {
                            new_sort_by = SortBy::BaselineTotal;
                            new_sort_ascending = false;
                        }
                        need_resort = true;
                    }
                });
                header.col(|ui| {
                    if ui.button("Optimized %").clicked() {
                        if new_sort_by == SortBy::OptimizedTotal {
                            new_sort_ascending = !new_sort_ascending;
                        } else {
                            new_sort_by = SortBy::OptimizedTotal;
                            new_sort_ascending = false;
                        }
                        need_resort = true;
                    }
                });
                header.col(|ui| {
                    if ui.button("Change %").clicked() {
                        if new_sort_by == SortBy::TotalChange {
                            new_sort_ascending = !new_sort_ascending;
                        } else {
                            new_sort_by = SortBy::TotalChange;
                            new_sort_ascending = false;
                        }
                        need_resort = true;
                    }
                });
                header.col(|ui| {
                    ui.label("Visual");
                });
            })
            .body(|body| {
                body.rows(20.0, filtered_comparisons.len(), |mut row| {
                    let idx = row.index();
                    if let Some(comp) = filtered_comparisons.get(idx) {
                        row.col(|ui| {
                            ui.label(&comp.name);
                        });
                        row.col(|ui| {
                            ui.label(format!("{:.2}%", comp.baseline_total_pct));
                        });
                        row.col(|ui| {
                            ui.label(format!("{:.2}%", comp.optimized_total_pct));
                        });
                        row.col(|ui| {
                            let change = comp.total_change_pct;
                            let color = if change < 0.0 {
                                Color32::from_rgb(0, 180, 0)
                            } else if change > 0.0 {
                                Color32::from_rgb(255, 100, 100)
                            } else {
                                Color32::GRAY
                            };

                            let text = if change.is_finite() {
                                format!("{change:+.2}%")
                            } else {
                                "NEW".to_owned()
                            };

                            ui.colored_label(color, text);
                        });
                        row.col(|ui| {
                            let change = comp.total_change_pct;
                            if change.is_finite() {
                                let bar_width = (change.abs().min(50.0) / 50.0) as f32 * 80.0;
                                let color = if change < 0.0 {
                                    Color32::from_rgb(0, 200, 0)
                                } else {
                                    Color32::from_rgb(255, 100, 100)
                                };

                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(80.0, 12.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    egui::Rect::from_min_size(
                                        rect.left_top(),
                                        egui::vec2(bar_width, 12.0),
                                    ),
                                    2.0,
                                    color,
                                );
                            }
                        });
                    }
                });
            });

        // Apply sort changes if needed
        if need_resort {
            self.sort_by = new_sort_by;
            self.sort_ascending = new_sort_ascending;
            self.sort_comparisons();
        }
    }
}

impl eframe::App for FlameGraphCompareApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(RichText::new("🔥 Flamegraph Comparison Tool").size(24.0));
            ui.separator();
            ui.add_space(10.0);

            // File picker section
            self.ui_file_picker(ui);

            ui.add_space(20.0);
            ui.separator();
            ui.add_space(10.0);

            // Only show the rest if data is loaded
            if self.baseline_data.is_some() && self.optimized_data.is_some() {
                // Summary section
                self.ui_summary(ui);

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);

                // Filters section
                self.ui_filters(ui);

                ui.add_space(20.0);
                ui.separator();
                ui.add_space(10.0);

                // Comparison table
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.ui_comparison_table(ui);
                });
            }
        });
    }
}
