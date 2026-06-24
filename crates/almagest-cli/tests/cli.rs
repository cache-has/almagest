// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the `almagest` binary.

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn prints_version() {
    Command::cargo_bin("almagest")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn no_args_prints_banner() {
    Command::cargo_bin("almagest")
        .unwrap()
        .assert()
        .success()
        .stdout(predicate::str::contains("Dashboards as files"));
}

#[test]
fn help_lists_subcommands() {
    Command::cargo_bin("almagest")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("new"))
        .stdout(predicate::str::contains("serve"));
}

#[test]
fn serve_missing_file_fails_cleanly() {
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["serve", "missing.alm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn export_is_not_yet_implemented() {
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["export", "missing.alm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet implemented"));
}

#[test]
fn new_creates_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("created.alm");
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["new", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created"));
    assert!(path.exists(), "new should create the .alm file");
}
