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
fn unimplemented_command_fails_cleanly() {
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["serve", "missing.alm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not yet implemented"));
}
