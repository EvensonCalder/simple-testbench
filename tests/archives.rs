use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn packages_test_bundle_and_dry_runs_from_archive() {
    let temp = tempdir().expect("temp dir should exist");
    let input_dir = temp.path().join("input");
    fs::create_dir(&input_dir).expect("input dir should be created");
    fs::copy("example/providers.json", input_dir.join("providers.json"))
        .expect("providers copy should succeed");
    fs::copy("example/models.json", input_dir.join("models.json"))
        .expect("models copy should succeed");

    let archive_path = temp.path().join("example.stbt");

    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["mkt", "-i", "example", "-o"])
        .arg(&archive_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Packaged test bundle"));

    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["test", "--dry-run", "-i"])
        .arg(&input_dir)
        .args(["-t"])
        .arg(&archive_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("test archive:"))
        .stdout(predicate::str::contains("loaded tests: 10"))
        .stdout(predicate::str::contains("planned requests: 40"));
}

#[test]
fn packages_scoring_bundle() {
    let temp = tempdir().expect("temp dir should exist");
    let archive_path = temp.path().join("example.stbs");

    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["mks", "-i", "example", "-o"])
        .arg(&archive_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("Packaged scoring bundle"))
        .stdout(predicate::str::contains("scoring.json"))
        .stdout(predicate::str::contains("post_process.lua"));

    assert!(Path::new(&archive_path).exists());
}
