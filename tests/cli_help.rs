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
        .stdout(predicate::str::contains("0.1.0"));
}

#[test]
fn supports_test_dry_run() {
    Command::cargo_bin("stb")
        .expect("binary should build")
        .args(["test", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("STB dry run"))
        .stdout(predicate::str::contains(
            "planner status: pending implementation",
        ));
}
