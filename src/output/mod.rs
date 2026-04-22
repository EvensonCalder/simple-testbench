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
    pub test_id: String,
    pub repeat_index: u32,
    pub api_style: String,
    pub status: RecordStatus,
    pub attempts: u32,
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
    pub score_name: String,
    pub kind: String,
    pub count: usize,
    pub mean: f64,
    pub std_dev: f64,
}

#[derive(Debug, Serialize)]
pub struct ResultsReport {
    pub records: Vec<ExecutionRecord>,
    pub aggregates: Vec<ScoreAggregate>,
}

#[derive(Debug, Clone)]
pub struct ReportArtifacts {
    pub results_json: Option<PathBuf>,
    pub score_mean_csv: PathBuf,
    pub score_std_csv: PathBuf,
    pub aggregates: Vec<ScoreAggregate>,
    pub terminal_table: String,
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
    pub test_id: String,
    pub repeat_index: u32,
}

impl ExecutionRecord {
    pub fn key(&self) -> RecordKey {
        RecordKey {
            provider_id: self.provider_id.clone(),
            model_id: self.model_id.clone(),
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
    let aggregates = build_score_aggregates(run_output);
    let score_mean_csv = output_dir.join("score_mean.csv");
    let score_std_csv = output_dir.join("score_std.csv");
    write_score_mean_csv(&score_mean_csv, &aggregates)?;
    write_score_std_csv(&score_std_csv, &aggregates)?;

    let results_json = if write_results_json {
        let path = output_dir.join("results.json");
        let report = ResultsReport {
            records: run_output.records.clone(),
            aggregates: aggregates.clone(),
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
        aggregates: aggregates.clone(),
        terminal_table: render_terminal_table(&aggregates),
    })
}

pub fn build_score_aggregates(run_output: &RunOutput) -> Vec<ScoreAggregate> {
    let mut grouped = BTreeMap::<(String, String, String, String), Vec<f64>>::new();

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
                    score.name.clone(),
                    score.kind.clone(),
                ))
                .or_default()
                .push(f64::from(value));
        }
    }

    grouped
        .into_iter()
        .map(|((provider_id, model_id, score_name, kind), values)| {
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
                score_name,
                kind,
                count,
                mean,
                std_dev: variance.sqrt(),
            }
        })
        .collect()
}

pub fn render_terminal_table(aggregates: &[ScoreAggregate]) -> String {
    if aggregates.is_empty() {
        return "No scored outputs available.".to_string();
    }

    let headers = [
        "provider_id",
        "model_id",
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
                aggregate.score_name.clone(),
                aggregate.kind.clone(),
                aggregate.count.to_string(),
                format!("{:.4}", aggregate.mean),
                format!("{:.4}", aggregate.std_dev),
            ]
        })
        .collect::<Vec<_>>();

    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.len());
        }
    }

    let mut lines = Vec::new();
    lines.push(format_row(&headers.map(str::to_string), &widths));
    lines.push(format_separator(&widths));
    lines.extend(rows.iter().map(|row| format_row(row, &widths)));
    lines.join("\n")
}

fn write_score_mean_csv(path: &Path, aggregates: &[ScoreAggregate]) -> anyhow::Result<()> {
    let mut lines = vec!["provider_id,model_id,score_name,kind,count,mean".to_string()];
    lines.extend(aggregates.iter().map(|aggregate| {
        format!(
            "{},{},{},{},{},{:.4}",
            csv_field(&aggregate.provider_id),
            csv_field(&aggregate.model_id),
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
    let mut lines = vec!["provider_id,model_id,score_name,kind,count,std_dev".to_string()];
    lines.extend(aggregates.iter().map(|aggregate| {
        format!(
            "{},{},{},{},{},{:.4}",
            csv_field(&aggregate.provider_id),
            csv_field(&aggregate.model_id),
            csv_field(&aggregate.score_name),
            csv_field(&aggregate.kind),
            aggregate.count,
            aggregate.std_dev,
        )
    }));
    fs::write(path, lines.join("\n"))
        .with_context(|| format!("failed to write {}", display_path(path)))
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
        ExecutionRecord, RecordStatus, RunOutput, ScoreResult, ScoreStatus, build_score_aggregates,
        render_terminal_table,
    };

    #[test]
    fn aggregates_scores_by_provider_model_and_score_name() {
        let run_output = RunOutput {
            records: vec![
                ExecutionRecord {
                    id: "1".to_string(),
                    provider_id: "mock".to_string(),
                    model_id: "model-a".to_string(),
                    test_id: "test-1".to_string(),
                    repeat_index: 1,
                    api_style: "openai_chat_completions".to_string(),
                    status: RecordStatus::Success,
                    attempts: 1,
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
                    test_id: "test-2".to_string(),
                    repeat_index: 1,
                    api_style: "openai_chat_completions".to_string(),
                    status: RecordStatus::Success,
                    attempts: 1,
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
        let table = render_terminal_table(&[super::ScoreAggregate {
            provider_id: "mock".to_string(),
            model_id: "model-a".to_string(),
            score_name: "json".to_string(),
            kind: "lua".to_string(),
            count: 2,
            mean: 90.0,
            std_dev: 10.0,
        }]);

        assert!(table.contains("provider_id"));
        assert!(table.contains("model-a"));
        assert!(table.contains("90.0000"));
    }
}
