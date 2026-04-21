//! Archive packaging and unpacking for .stbt and .stbs bundles.

use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Write},
    path::Path,
};

use anyhow::Context;
use serde::Deserialize;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::FileOptions};

use crate::error::StbError;

const SYSTEM_PROMPTS_FILE: &str = "system_prompts.json";
const TESTS_FILE: &str = "tests.json";
const SCORING_FILE: &str = "scoring.json";
const POST_PROCESS_FILE: &str = "post_process.lua";

#[derive(Debug, Clone)]
pub struct PackagedBundle {
    pub kind: &'static str,
    pub files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LoadedTestBundle {
    pub system_prompts_json: String,
    pub tests_json: String,
}

pub fn package_test_bundle(input_dir: &Path, output: &Path) -> anyhow::Result<PackagedBundle> {
    let files = vec![SYSTEM_PROMPTS_FILE.to_string(), TESTS_FILE.to_string()];

    for file in &files {
        let path = input_dir.join(file);
        ensure_exists(&path)?;
    }

    write_archive(input_dir, output, &files)?;

    Ok(PackagedBundle {
        kind: "test",
        files,
    })
}

pub fn package_scoring_bundle(input_dir: &Path, output: &Path) -> anyhow::Result<PackagedBundle> {
    let scoring_path = input_dir.join(SCORING_FILE);
    ensure_exists(&scoring_path)?;

    let scoring = read_json_file::<ScoringFile>(&scoring_path)?;
    let mut files = BTreeSet::from([SCORING_FILE.to_string()]);

    let post_process_path = input_dir.join(POST_PROCESS_FILE);
    if post_process_path
        .try_exists()
        .with_context(|| format!("failed to access {}", display_path(&post_process_path)))?
    {
        files.insert(POST_PROCESS_FILE.to_string());
    }

    for item in scoring.scoring {
        files.insert(item.file);
    }

    let files = files.into_iter().collect::<Vec<_>>();
    for file in &files {
        let path = input_dir.join(file);
        ensure_exists(&path)?;
    }

    write_archive(input_dir, output, &files)?;

    Ok(PackagedBundle {
        kind: "scoring",
        files,
    })
}

pub fn load_test_bundle(path: &Path) -> anyhow::Result<LoadedTestBundle> {
    let file =
        File::open(path).with_context(|| format!("failed to open {}", display_path(path)))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("failed to read zip archive {}", display_path(path)))?;

    Ok(LoadedTestBundle {
        system_prompts_json: read_entry_to_string(&mut archive, SYSTEM_PROMPTS_FILE)?,
        tests_json: read_entry_to_string(&mut archive, TESTS_FILE)?,
    })
}

#[derive(Debug, Deserialize)]
struct ScoringFile {
    scoring: Vec<ScoringItem>,
}

#[derive(Debug, Deserialize)]
struct ScoringItem {
    file: String,
}

fn write_archive(input_dir: &Path, output: &Path, files: &[String]) -> anyhow::Result<()> {
    let file = File::create(output)
        .with_context(|| format!("failed to create {}", display_path(output)))?;
    let mut writer = ZipWriter::new(file);
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);

    for relative_path in files {
        let full_path = input_dir.join(relative_path);
        let bytes = std::fs::read(&full_path)
            .with_context(|| format!("failed to read {}", display_path(&full_path)))?;

        writer
            .start_file(relative_path, options)
            .with_context(|| format!("failed to start zip entry {}", relative_path))?;
        writer
            .write_all(&bytes)
            .with_context(|| format!("failed to write zip entry {}", relative_path))?;
    }

    writer.finish().context("failed to finalize zip archive")?;
    Ok(())
}

fn read_entry_to_string<R>(archive: &mut ZipArchive<R>, name: &str) -> anyhow::Result<String>
where
    R: Read + std::io::Seek,
{
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("missing {} in archive", name))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .with_context(|| format!("failed to read {} from archive", name))?;
    Ok(content)
}

fn ensure_exists(path: &Path) -> anyhow::Result<()> {
    if !path
        .try_exists()
        .with_context(|| format!("failed to access {}", display_path(path)))?
    {
        return Err(StbError::MissingPath(path.to_path_buf()).into());
    }

    Ok(())
}

fn read_json_file<T>(path: &Path) -> anyhow::Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", display_path(path)))?;
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", display_path(path)))
}

fn display_path(path: &Path) -> &str {
    path.to_str().unwrap_or("<non-utf8 path>")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{load_test_bundle, package_scoring_bundle, package_test_bundle};

    #[test]
    fn packages_and_loads_test_bundle() {
        let temp = tempdir().expect("temp dir should exist");
        let output = temp.path().join("example.stbt");

        let summary =
            package_test_bundle(Path::new("example"), &output).expect("bundle should package");
        assert_eq!(summary.files, vec!["system_prompts.json", "tests.json"]);

        let loaded = load_test_bundle(&output).expect("bundle should load");
        assert!(loaded.system_prompts_json.contains("todo_json_v1"));
        assert!(loaded.tests_json.contains("todo-010"));
    }

    #[test]
    fn packages_scoring_bundle_with_referenced_files() {
        let temp = tempdir().expect("temp dir should exist");
        let output = temp.path().join("example.stbs");

        let summary = package_scoring_bundle(Path::new("example"), &output)
            .expect("scoring bundle should package");
        assert!(summary.files.contains(&"scoring.json".to_string()));
        assert!(summary.files.contains(&"post_process.lua".to_string()));
        assert!(summary.files.contains(&"score_json.lua".to_string()));
        assert!(summary.files.contains(&"score_extract_ai.json".to_string()));
        assert!(fs::metadata(output).is_ok());
    }
}
