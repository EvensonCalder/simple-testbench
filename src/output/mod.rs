//! Persistent outputs and report rendering.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct RunOutput {
    pub records: Vec<ExecutionRecord>,
}

impl RunOutput {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExecutionRecord {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default)]
    pub model_instance_id: String,
    #[serde(default)]
    pub model_config_key: String,
    pub test_id: String,
    pub repeat_index: u32,
    pub api_style: String,
    pub status: RecordStatus,
    pub attempts: u32,
    #[serde(default)]
    pub elapsed_ms: u64,
    pub output_text: Option<String>,
    #[serde(default)]
    pub processed_output: Option<String>,
    #[serde(default)]
    pub post_process_applied: bool,
    #[serde(default)]
    pub post_process_retries: u32,
    #[serde(default)]
    pub scores: Vec<ScoreResult>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ScoreResult {
    pub name: String,
    pub kind: String,
    pub status: ScoreStatus,
    pub score: Option<u8>,
    #[serde(default)]
    pub details: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ScoreAggregate {
    pub provider_id: String,
    pub model_id: String,
    pub model_instance_id: String,
    pub model_config_key: String,
    pub score_name: String,
    pub kind: String,
    pub count: usize,
    pub mean: f64,
    pub std_dev: f64,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct DurationAggregate {
    pub provider_id: String,
    pub model_id: String,
    pub model_instance_id: String,
    pub model_config_key: String,
    pub successful_count: usize,
    pub mean_elapsed_ms: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct ResultsReport {
    pub records: Vec<ExecutionRecord>,
    pub score_aggregates: Vec<ScoreAggregate>,
    pub duration_aggregates: Vec<DurationAggregate>,
}

#[derive(Debug, Clone)]
pub struct ReportArtifacts {
    pub results_json: Option<PathBuf>,
    pub score_mean_csv: PathBuf,
    pub score_std_csv: PathBuf,
    pub duration_mean_csv: PathBuf,
    pub score_aggregates: Vec<ScoreAggregate>,
    pub duration_aggregates: Vec<DurationAggregate>,
    pub terminal_report: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScoreStatus {
    Success,
    Failed,
    SkippedNotImplemented,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Success,
    Failed,
    SkippedModelDisabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecordKey {
    pub provider_id: String,
    pub model_id: String,
    pub model_instance_id: String,
    pub test_id: String,
    pub repeat_index: u32,
}

impl ExecutionRecord {
    pub fn key(&self) -> RecordKey {
        RecordKey {
            provider_id: self.provider_id.clone(),
            model_id: self.model_id.clone(),
            model_instance_id: self.model_instance_id.clone(),
            test_id: self.test_id.clone(),
            repeat_index: self.repeat_index,
        }
    }
}

pub fn prepare_output_dir(
    explicit_output_dir: Option<&Path>,
    fresh: bool,
) -> anyhow::Result<PathBuf> {
    let output_dir = explicit_output_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(default_output_dir);

    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "failed to create output directory {}",
            display_path(&output_dir)
        )
    })?;

    if fresh {
        for path in [
            output_dir.join("output.json"),
            output_dir.join("results.json"),
            output_dir.join("score_mean.csv"),
            output_dir.join("score_std.csv"),
            output_dir.join("duration_mean.csv"),
        ] {
            if path
                .try_exists()
                .with_context(|| format!("failed to access {}", display_path(&path)))?
            {
                fs::remove_file(&path)
                    .with_context(|| format!("failed to remove {}", display_path(&path)))?;
            }
        }
    }

    Ok(output_dir)
}

pub fn output_json_path(output_dir: &Path) -> PathBuf {
    output_dir.join("output.json")
}

pub fn load_run_output(path: &Path) -> anyhow::Result<RunOutput> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", display_path(path)))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", display_path(path)))
}

pub fn write_run_output(path: &Path, run_output: &RunOutput) -> anyhow::Result<()> {
    let content =
        serde_json::to_string_pretty(run_output).context("failed to serialize output.json")?;
    fs::write(path, content).with_context(|| format!("failed to write {}", display_path(path)))
}

pub fn next_record_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn write_reports(
    output_dir: &Path,
    run_output: &RunOutput,
    write_results_json: bool,
) -> anyhow::Result<ReportArtifacts> {
    let score_aggregates = build_score_aggregates(run_output);
    let duration_aggregates = build_duration_aggregates(run_output);
    let score_mean_csv = output_dir.join("score_mean.csv");
    let score_std_csv = output_dir.join("score_std.csv");
    let duration_mean_csv = output_dir.join("duration_mean.csv");
    write_score_mean_csv(&score_mean_csv, &score_aggregates)?;
    write_score_std_csv(&score_std_csv, &score_aggregates)?;
    write_duration_mean_csv(&duration_mean_csv, &duration_aggregates)?;

    let results_json = if write_results_json {
        let path = output_dir.join("results.json");
        let report = ResultsReport {
            records: run_output.records.clone(),
            score_aggregates: score_aggregates.clone(),
            duration_aggregates: duration_aggregates.clone(),
        };
        let content =
            serde_json::to_string_pretty(&report).context("failed to serialize results.json")?;
        fs::write(&path, content)
            .with_context(|| format!("failed to write {}", display_path(&path)))?;
        Some(path)
    } else {
        None
    };

    Ok(ReportArtifacts {
        results_json,
        score_mean_csv,
        score_std_csv,
        duration_mean_csv,
        score_aggregates: score_aggregates.clone(),
        duration_aggregates: duration_aggregates.clone(),
        terminal_report: render_terminal_report(&duration_aggregates, &score_aggregates),
    })
}

pub fn build_score_aggregates(run_output: &RunOutput) -> Vec<ScoreAggregate> {
    let mut grouped = BTreeMap::<(String, String, String, String, String, String), Vec<f64>>::new();

    for record in &run_output.records {
        if record.status != RecordStatus::Success {
            continue;
        }

        for score in &record.scores {
            if score.status != ScoreStatus::Success {
                continue;
            }

            let Some(value) = score.score else {
                continue;
            };

            grouped
                .entry((
                    record.provider_id.clone(),
                    record.model_id.clone(),
                    record.model_instance_id.clone(),
                    record.model_config_key.clone(),
                    score.name.clone(),
                    score.kind.clone(),
                ))
                .or_default()
                .push(f64::from(value));
        }
    }

    grouped
        .into_iter()
        .map(
            |(
                (provider_id, model_id, model_instance_id, model_config_key, score_name, kind),
                values,
            )| {
                let count = values.len();
                let mean = values.iter().sum::<f64>() / count as f64;
                let variance = values
                    .iter()
                    .map(|value| {
                        let delta = value - mean;
                        delta * delta
                    })
                    .sum::<f64>()
                    / count as f64;

                ScoreAggregate {
                    provider_id,
                    model_id,
                    model_instance_id,
                    model_config_key,
                    score_name,
                    kind,
                    count,
                    mean,
                    std_dev: variance.sqrt(),
                }
            },
        )
        .collect()
}

pub fn build_duration_aggregates(run_output: &RunOutput) -> Vec<DurationAggregate> {
    let mut grouped = BTreeMap::<(String, String, String, String), Vec<f64>>::new();

    for record in &run_output.records {
        grouped
            .entry((
                record.provider_id.clone(),
                record.model_id.clone(),
                record.model_instance_id.clone(),
                record.model_config_key.clone(),
            ))
            .or_default();

        if record.status == RecordStatus::Success && record.elapsed_ms > 0 {
            grouped
                .get_mut(&(
                    record.provider_id.clone(),
                    record.model_id.clone(),
                    record.model_instance_id.clone(),
                    record.model_config_key.clone(),
                ))
                .expect("duration grouping entry should exist")
                .push(record.elapsed_ms as f64);
        }
    }

    grouped
        .into_iter()
        .map(
            |((provider_id, model_id, model_instance_id, model_config_key), values)| {
                DurationAggregate {
                    provider_id,
                    model_id,
                    model_instance_id,
                    model_config_key,
                    successful_count: values.len(),
                    mean_elapsed_ms: (!values.is_empty())
                        .then(|| values.iter().sum::<f64>() / values.len() as f64),
                }
            },
        )
        .collect()
}

pub fn render_terminal_report(
    duration_aggregates: &[DurationAggregate],
    score_aggregates: &[ScoreAggregate],
) -> String {
    let mut sections = Vec::new();

    if !duration_aggregates.is_empty() {
        sections.push(render_duration_table(duration_aggregates));
    }

    if !score_aggregates.is_empty() {
        sections.push(render_score_table(score_aggregates));
    }

    if sections.is_empty() {
        return "No duration or score aggregates available.".to_string();
    }

    sections.join("\n\n")
}

fn render_duration_table(aggregates: &[DurationAggregate]) -> String {
    let headers = ["provider_id", "model_id", "instance", "count", "avg_ms"];
    let rows = aggregates
        .iter()
        .map(|aggregate| {
            vec![
                aggregate.provider_id.clone(),
                aggregate.model_id.clone(),
                short_instance_id(&aggregate.model_instance_id),
                aggregate.successful_count.to_string(),
                aggregate
                    .mean_elapsed_ms
                    .map(|value| format!("{value:.2}"))
                    .unwrap_or_else(|| "N/A".to_string()),
            ]
        })
        .collect::<Vec<_>>();

    render_table("Duration", &headers, &rows)
}

fn render_score_table(aggregates: &[ScoreAggregate]) -> String {
    if aggregates.is_empty() {
        return "No scored outputs available.".to_string();
    }

    let headers = [
        "provider_id",
        "model_id",
        "instance",
        "score",
        "kind",
        "count",
        "mean",
        "std_dev",
    ];
    let rows = aggregates
        .iter()
        .map(|aggregate| {
            vec![
                aggregate.provider_id.clone(),
                aggregate.model_id.clone(),
                short_instance_id(&aggregate.model_instance_id),
                aggregate.score_name.clone(),
                aggregate.kind.clone(),
                aggregate.count.to_string(),
                format!("{:.4}", aggregate.mean),
                format!("{:.4}", aggregate.std_dev),
            ]
        })
        .collect::<Vec<_>>();

    render_table("Scores", &headers, &rows)
}

fn render_table(title: &str, headers: &[&str], rows: &[Vec<String>]) -> String {
    let header_cells = headers
        .iter()
        .map(|header| (*header).to_string())
        .collect::<Vec<_>>();
    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }

    let mut lines = Vec::new();
    lines.push(title.to_string());
    lines.push(format_row(&header_cells, &widths));
    lines.push(format_separator(&widths));
    lines.extend(rows.iter().map(|row| format_row(row, &widths)));
    lines.join("\n")
}

fn write_score_mean_csv(path: &Path, aggregates: &[ScoreAggregate]) -> anyhow::Result<()> {
    let mut lines = vec![
        "provider_id,model_id,model_instance_id,model_config_key,score_name,kind,count,mean"
            .to_string(),
    ];
    lines.extend(aggregates.iter().map(|aggregate| {
        format!(
            "{},{},{},{},{},{},{},{:.4}",
            csv_field(&aggregate.provider_id),
            csv_field(&aggregate.model_id),
            csv_field(&aggregate.model_instance_id),
            csv_field(&aggregate.model_config_key),
            csv_field(&aggregate.score_name),
            csv_field(&aggregate.kind),
            aggregate.count,
            aggregate.mean,
        )
    }));
    fs::write(path, lines.join("\n"))
        .with_context(|| format!("failed to write {}", display_path(path)))
}

fn write_score_std_csv(path: &Path, aggregates: &[ScoreAggregate]) -> anyhow::Result<()> {
    let mut lines = vec![
        "provider_id,model_id,model_instance_id,model_config_key,score_name,kind,count,std_dev"
            .to_string(),
    ];
    lines.extend(aggregates.iter().map(|aggregate| {
        format!(
            "{},{},{},{},{},{},{},{:.4}",
            csv_field(&aggregate.provider_id),
            csv_field(&aggregate.model_id),
            csv_field(&aggregate.model_instance_id),
            csv_field(&aggregate.model_config_key),
            csv_field(&aggregate.score_name),
            csv_field(&aggregate.kind),
            aggregate.count,
            aggregate.std_dev,
        )
    }));
    fs::write(path, lines.join("\n"))
        .with_context(|| format!("failed to write {}", display_path(path)))
}

fn write_duration_mean_csv(path: &Path, aggregates: &[DurationAggregate]) -> anyhow::Result<()> {
    let mut lines = vec![
        "provider_id,model_id,model_instance_id,model_config_key,successful_count,mean_elapsed_ms"
            .to_string(),
    ];
    lines.extend(aggregates.iter().map(|aggregate| {
        format!(
            "{},{},{},{},{},{}",
            csv_field(&aggregate.provider_id),
            csv_field(&aggregate.model_id),
            csv_field(&aggregate.model_instance_id),
            csv_field(&aggregate.model_config_key),
            aggregate.successful_count,
            aggregate
                .mean_elapsed_ms
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "N/A".to_string()),
        )
    }));
    fs::write(path, lines.join("\n"))
        .with_context(|| format!("failed to write {}", display_path(path)))
}

fn short_instance_id(instance_id: &str) -> String {
    instance_id.chars().take(8).collect()
}

fn csv_field(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn format_row(row: &[String], widths: &[usize]) -> String {
    row.iter()
        .enumerate()
        .map(|(index, cell)| format!("{cell:<width$}", width = widths[index]))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn format_separator(widths: &[usize]) -> String {
    widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>()
        .join("-+-")
}

fn default_output_dir() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(format!("stb_out_{timestamp}"))
}

fn display_path(path: &Path) -> &str {
    path.to_str().unwrap_or("<non-utf8 path>")
}

#[cfg(test)]
mod tests {
    use super::{
        DurationAggregate, ExecutionRecord, RecordStatus, RunOutput, ScoreResult, ScoreStatus,
        build_duration_aggregates, build_score_aggregates, render_terminal_report,
    };

    #[test]
    fn aggregates_scores_by_provider_model_and_score_name() {
        let run_output = RunOutput {
            records: vec![
                ExecutionRecord {
                    id: "1".to_string(),
                    provider_id: "mock".to_string(),
                    model_id: "model-a".to_string(),
                    model_instance_id: "instance-a".to_string(),
                    model_config_key: "config-a".to_string(),
                    test_id: "test-1".to_string(),
                    repeat_index: 1,
                    api_style: "openai_chat_completions".to_string(),
                    status: RecordStatus::Success,
                    attempts: 1,
                    elapsed_ms: 120,
                    output_text: Some("a".to_string()),
                    processed_output: Some("a".to_string()),
                    post_process_applied: false,
                    post_process_retries: 0,
                    scores: vec![ScoreResult {
                        name: "json".to_string(),
                        kind: "lua".to_string(),
                        status: ScoreStatus::Success,
                        score: Some(80),
                        details: None,
                        error: None,
                    }],
                    error: None,
                },
                ExecutionRecord {
                    id: "2".to_string(),
                    provider_id: "mock".to_string(),
                    model_id: "model-a".to_string(),
                    model_instance_id: "instance-a".to_string(),
                    model_config_key: "config-a".to_string(),
                    test_id: "test-2".to_string(),
                    repeat_index: 1,
                    api_style: "openai_chat_completions".to_string(),
                    status: RecordStatus::Success,
                    attempts: 1,
                    elapsed_ms: 180,
                    output_text: Some("b".to_string()),
                    processed_output: Some("b".to_string()),
                    post_process_applied: false,
                    post_process_retries: 0,
                    scores: vec![ScoreResult {
                        name: "json".to_string(),
                        kind: "lua".to_string(),
                        status: ScoreStatus::Success,
                        score: Some(100),
                        details: None,
                        error: None,
                    }],
                    error: None,
                },
            ],
        };

        let aggregates = build_score_aggregates(&run_output);
        assert_eq!(aggregates.len(), 1);
        assert_eq!(aggregates[0].count, 2);
        assert_eq!(aggregates[0].mean, 90.0);
        assert_eq!(format!("{:.4}", aggregates[0].std_dev), "10.0000");
    }

    #[test]
    fn renders_terminal_table_for_aggregates() {
        let table = render_terminal_report(
            &[DurationAggregate {
                provider_id: "mock".to_string(),
                model_id: "model-a".to_string(),
                model_instance_id: "instance-a".to_string(),
                model_config_key: "config-a".to_string(),
                successful_count: 2,
                mean_elapsed_ms: Some(150.0),
            }],
            &[super::ScoreAggregate {
                provider_id: "mock".to_string(),
                model_id: "model-a".to_string(),
                model_instance_id: "instance-a".to_string(),
                model_config_key: "config-a".to_string(),
                score_name: "json".to_string(),
                kind: "lua".to_string(),
                count: 2,
                mean: 90.0,
                std_dev: 10.0,
            }],
        );

        assert!(table.contains("Duration"));
        assert!(table.contains("Scores"));
        assert!(table.contains("model-a"));
        assert!(table.contains("90.0000"));
        assert!(table.contains("150.00"));
    }

    #[test]
    fn duration_aggregate_reports_na_for_all_failed_model() {
        let run_output = RunOutput {
            records: vec![ExecutionRecord {
                id: "1".to_string(),
                provider_id: "mock".to_string(),
                model_id: "model-a".to_string(),
                model_instance_id: "instance-a".to_string(),
                model_config_key: "config-a".to_string(),
                test_id: "test-1".to_string(),
                repeat_index: 1,
                api_style: "openai_chat_completions".to_string(),
                status: RecordStatus::Failed,
                attempts: 1,
                elapsed_ms: 0,
                output_text: None,
                processed_output: None,
                post_process_applied: false,
                post_process_retries: 0,
                scores: vec![],
                error: Some("boom".to_string()),
            }],
        };

        let aggregates = build_duration_aggregates(&run_output);
        assert_eq!(aggregates.len(), 1);
        assert_eq!(aggregates[0].successful_count, 0);
        assert_eq!(aggregates[0].mean_elapsed_ms, None);

        let report = render_terminal_report(&aggregates, &[]);
        assert!(report.contains("N/A"));
    }
}
