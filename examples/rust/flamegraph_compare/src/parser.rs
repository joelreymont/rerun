//! Flamegraph parsing and comparison logic

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct StackSample {
    pub stack: String,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub struct FunctionStats {
    pub name: String,
    pub self_time: u64,
    pub total_time: u64,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct FlameGraphData {
    pub stacks: Vec<StackSample>,
    pub function_stats: HashMap<String, FunctionStats>,
    pub total_samples: u64,
}

impl FlameGraphData {
    pub fn new() -> Self {
        Self {
            stacks: Vec::new(),
            function_stats: HashMap::new(),
            total_samples: 0,
        }
    }

    pub fn add_stack(&mut self, stack: String, count: u64) {
        self.stacks.push(StackSample {
            stack: stack.clone(),
            count,
        });
        self.total_samples += count;

        // Update function statistics
        let functions: Vec<&str> = stack.split(';').collect();
        for (i, func) in functions.iter().enumerate() {
            let stats = self
                .function_stats
                .entry(func.to_string())
                .or_insert_with(|| FunctionStats {
                    name: func.to_string(),
                    self_time: 0,
                    total_time: 0,
                    count: 0,
                });

            // Every function in the stack gets the total time
            stats.total_time += count;
            stats.count += 1;

            // Only the leaf function gets the self time
            if i == functions.len() - 1 {
                stats.self_time += count;
            }
        }
    }

    pub fn get_function_total_percentage(&self, func_name: &str) -> f64 {
        if self.total_samples == 0 {
            return 0.0;
        }
        self.function_stats
            .get(func_name)
            .map(|stats| (stats.total_time as f64 / self.total_samples as f64) * 100.0)
            .unwrap_or(0.0)
    }

    pub fn get_function_self_percentage(&self, func_name: &str) -> f64 {
        if self.total_samples == 0 {
            return 0.0;
        }
        self.function_stats
            .get(func_name)
            .map(|stats| (stats.self_time as f64 / self.total_samples as f64) * 100.0)
            .unwrap_or(0.0)
    }
}

#[derive(Debug, Clone)]
pub struct FunctionComparison {
    pub name: String,
    pub baseline_total_pct: f64,
    pub optimized_total_pct: f64,
    pub baseline_self_pct: f64,
    pub optimized_self_pct: f64,
    pub total_change_pct: f64,
    pub self_change_pct: f64,
}

pub fn parse_flamegraph(path: &Path) -> anyhow::Result<FlameGraphData> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match extension {
        "svg" => parse_svg_format(path),
        "json" => parse_json_format(path),
        _ => parse_collapsed_format(path),
    }
}

fn parse_collapsed_format(path: &Path) -> anyhow::Result<FlameGraphData> {
    let content = std::fs::read_to_string(path)?;
    let mut data = FlameGraphData::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split stack and count
        let parts: Vec<&str> = line.rsplitn(2, ' ').collect();
        if parts.len() != 2 {
            continue;
        }

        let count_str = parts[0];
        let stack = parts[1];

        if let Ok(count) = count_str.parse::<u64>() {
            data.add_stack(stack.to_owned(), count);
        }
    }

    Ok(data)
}

fn parse_svg_format(path: &Path) -> anyhow::Result<FlameGraphData> {
    let content = std::fs::read_to_string(path)?;
    let mut data = FlameGraphData::new();

    // Parse XML to extract <title> elements
    use std::io::Cursor;
    let cursor = Cursor::new(content);
    let parser = xml::reader::EventReader::new(cursor);

    let mut in_title = false;
    let mut current_text = String::new();

    for event in parser {
        match event {
            Ok(xml::reader::XmlEvent::StartElement { name, .. }) => {
                if name.local_name == "title" {
                    in_title = true;
                    current_text.clear();
                }
            }
            Ok(xml::reader::XmlEvent::Characters(text)) => {
                if in_title {
                    current_text.push_str(&text);
                }
            }
            Ok(xml::reader::XmlEvent::EndElement { name }) => {
                if name.local_name == "title" && in_title {
                    in_title = false;
                    if let Some((stack, count)) = parse_svg_title(&current_text) {
                        data.add_stack(stack, count);
                    }
                }
            }
            Err(_) => break,
            _ => {}
        }
    }

    Ok(data)
}

fn parse_svg_title(text: &str) -> Option<(String, u64)> {
    let text = text.trim();

    // Skip "all" or "flamegraph" entries
    if text.eq_ignore_ascii_case("all") || text.eq_ignore_ascii_case("flamegraph") {
        return None;
    }

    // Try pattern: "stack (N samples, X%)"
    if let Some(cap_idx) = text.find(" (") {
        let stack = &text[..cap_idx];
        let rest = &text[cap_idx + 2..];

        // Look for "N samples"
        if let Some(samples_idx) = rest.find(" samples") {
            let count_str = rest[..samples_idx].replace(',', "");
            if let Ok(count) = count_str.parse::<u64>() {
                return Some((stack.to_owned(), count));
            }
        }

        // Try pattern: "stack (N)"
        if let Some(close_paren) = rest.find(')') {
            let count_str = rest[..close_paren].replace(',', "");
            if let Ok(count) = count_str.parse::<u64>() {
                return Some((stack.to_owned(), count));
            }
        }
    }

    // Try pattern: "stack N"
    let parts: Vec<&str> = text.rsplitn(2, ' ').collect();
    if parts.len() == 2 {
        let count_str = parts[0].replace(',', "");
        if let Ok(count) = count_str.parse::<u64>() {
            return Some((parts[1].to_owned(), count));
        }
    }

    None
}

fn parse_json_format(path: &Path) -> anyhow::Result<FlameGraphData> {
    let content = std::fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;
    let mut data = FlameGraphData::new();

    // Handle different JSON structures
    let stacks = if let Some(stacks_arr) = json.get("stacks").and_then(|v| v.as_array()) {
        stacks_arr.clone()
    } else if let Some(arr) = json.as_array() {
        arr.clone()
    } else {
        return Ok(data);
    };

    for entry in stacks {
        if let Some(obj) = entry.as_object() {
            let stack_arr = obj.get("stack");
            let count = obj
                .get("count")
                .or_else(|| obj.get("samples"))
                .and_then(|v| v.as_u64())
                .unwrap_or(1);

            if let Some(stack_arr) = stack_arr.and_then(|v| v.as_array()) {
                let stack = stack_arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(";");
                data.add_stack(stack, count);
            }
        }
    }

    Ok(data)
}

pub fn compare_flamegraphs(
    baseline: &FlameGraphData,
    optimized: &FlameGraphData,
) -> Vec<FunctionComparison> {
    let mut all_functions: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    all_functions.extend(baseline.function_stats.keys().cloned());
    all_functions.extend(optimized.function_stats.keys().cloned());

    let mut comparisons = Vec::new();

    for func_name in all_functions {
        let baseline_total = baseline.get_function_total_percentage(&func_name);
        let optimized_total = optimized.get_function_total_percentage(&func_name);
        let baseline_self = baseline.get_function_self_percentage(&func_name);
        let optimized_self = optimized.get_function_self_percentage(&func_name);

        // Calculate percentage change
        let total_change = if baseline_total > 0.0 {
            ((optimized_total - baseline_total) / baseline_total) * 100.0
        } else if optimized_total > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        let self_change = if baseline_self > 0.0 {
            ((optimized_self - baseline_self) / baseline_self) * 100.0
        } else if optimized_self > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        comparisons.push(FunctionComparison {
            name: func_name,
            baseline_total_pct: baseline_total,
            optimized_total_pct: optimized_total,
            baseline_self_pct: baseline_self,
            optimized_self_pct: optimized_self,
            total_change_pct: total_change,
            self_change_pct: self_change,
        });
    }

    comparisons
}
