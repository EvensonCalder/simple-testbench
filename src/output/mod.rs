//! Persistent outputs and report rendering.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize)]
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

#[derive(Debug, Serialize)]
pub struct ExecutionRecord {
    pub id: String,
    pub provider_id: String,
    pub model_id: String,
    pub test_id: String,
    pub repeat_index: u32,
    pub api_style: String,
    pub status: RecordStatus,
    pub attempts: u8,
    pub output_text: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordStatus {
    Success,
    Failed,
    SkippedModelDisabled,
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
        let output_path = output_dir.join("output.json");
        if output_path
            .try_exists()
            .with_context(|| format!("failed to access {}", display_path(&output_path)))?
        {
            fs::remove_file(&output_path)
                .with_context(|| format!("failed to remove {}", display_path(&output_path)))?;
        }
    }

    Ok(output_dir)
}

pub fn output_json_path(output_dir: &Path) -> PathBuf {
    output_dir.join("output.json")
}

pub fn write_run_output(path: &Path, run_output: &RunOutput) -> anyhow::Result<()> {
    let content =
        serde_json::to_string_pretty(run_output).context("failed to serialize output.json")?;
    fs::write(path, content).with_context(|| format!("failed to write {}", display_path(path)))
}

pub fn next_record_id() -> String {
    Uuid::new_v4().to_string()
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
