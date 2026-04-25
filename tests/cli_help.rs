use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn prints_help() {
    Command::cargo_bin("stb")
        .expect("binary should build")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "STB benchmarks model outputs against test suites",
        ))
        .stdout(predicate::str::contains("test"))
        .stdout(predicate::str::contains("mkt"))
        .stdout(predicate::str::contains("mks"));
}

#[test]
fn prints_version() {
    Command::cargo_bin("stb")
        .expect("binary should build")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("0.1.2"));
}

#[test]
fn supports_test_dry_run() {
    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["test", "--dry-run", "-i", "example"])
        .assert()
        .success()
        .stdout(predicate::str::contains("STB dry run"))
        .stdout(predicate::str::contains("selected model instances: 4"))
        .stdout(predicate::str::contains("total tests: 10"))
        .stdout(predicate::str::contains("planned requests: 40"));
}

#[test]
fn supports_filtered_test_dry_run() {
    Command::cargo_bin("stb")
        .expect("binary should build")
        .args([
            "test",
            "--dry-run",
            "-i",
            "example",
            "--provider",
            "openrouter",
            "--model",
            "z-ai/glm-5.1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("selected model instances: 1"))
        .stdout(predicate::str::contains("planned requests: 10"))
        .stdout(predicate::str::contains(
            "openrouter/z-ai/glm-5.1 [openai_responses]",
        ));
}
