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
fn export_missing_file_fails_cleanly() {
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["export", "missing.alm"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn export_rejects_unsupported_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.alm");
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["new", path.to_str().unwrap()])
        .assert()
        .success();
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["export", path.to_str().unwrap(), "--format", "pdf"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported --format"));
}

#[test]
fn export_with_no_dashboards_reports_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.alm");
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["new", path.to_str().unwrap()])
        .assert()
        .success();
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["export", path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no dashboards"));
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

// --- Phase 14: management commands -------------------------------------------

/// Create a `.alm` from the `sales` template at a fresh temp path.
fn sales_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    let path = dir.path().join("sales.alm");
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["new", path.to_str().unwrap(), "--from-template", "sales"])
        .assert()
        .success();
    path
}

#[test]
fn new_from_template_sales_bakes_data_and_dashboard() {
    let dir = tempfile::tempdir().unwrap();
    let path = sales_file(&dir);
    assert!(path.exists());
    // Info should report the embedded table and the starter dashboard.
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["info", path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("sales"))
        .stdout(predicate::str::contains("Sales Overview"));
}

#[test]
fn new_from_unknown_template_fails() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.alm");
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["new", path.to_str().unwrap(), "--from-template", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unknown template"));
}

#[test]
fn info_json_is_parseable() {
    let dir = tempfile::tempdir().unwrap();
    let path = sales_file(&dir);
    let output = Command::cargo_bin("almagest")
        .unwrap()
        .args(["info", path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let v: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(v["format_version"], 2);
    assert_eq!(v["tables"][0]["name"], "sales");
    assert_eq!(v["auth_enabled"], false);
}

#[test]
fn validate_strict_passes_on_template_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = sales_file(&dir);
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["validate", path.to_str().unwrap(), "--strict"])
        .assert()
        .success()
        .stdout(predicate::str::contains("is valid"));
}

#[test]
fn validate_reports_corruption_with_nonzero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("garbage.alm");
    std::fs::write(&path, b"this is not a sqlite database").unwrap();
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["validate", path.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("INVALID"));
}

#[test]
fn dashboard_list_json_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let path = sales_file(&dir);

    // List as JSON → exactly one dashboard.
    let out = Command::cargo_bin("almagest")
        .unwrap()
        .args(["dashboard", "list", path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let id = v[0]["id"].as_str().unwrap().to_string();
    assert_eq!(v.as_array().unwrap().len(), 1);

    // Delete it by id (with --yes to skip the prompt).
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["dashboard", "delete", path.to_str().unwrap(), &id, "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Deleted"));

    // Now the list is empty.
    let out = Command::cargo_bin("almagest")
        .unwrap()
        .args(["dashboard", "list", path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 0);
}

#[test]
fn user_add_enables_auth_and_lists() {
    let dir = tempfile::tempdir().unwrap();
    let path = sales_file(&dir);

    // First user enables auth.
    Command::cargo_bin("almagest")
        .unwrap()
        .args([
            "user",
            "add",
            path.to_str().unwrap(),
            "--username",
            "admin",
            "--role",
            "admin",
            "--password",
            "supersecret",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Auth is now enabled"));

    // The file now reports auth enabled with one user.
    let out = Command::cargo_bin("almagest")
        .unwrap()
        .args(["user", "list", path.to_str().unwrap(), "--json"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["auth_enabled"], true);
    assert_eq!(v["users"][0]["username"], "admin");
    assert_eq!(v["users"][0]["role"], "admin");
}

#[test]
fn user_password_too_short_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = sales_file(&dir);
    Command::cargo_bin("almagest")
        .unwrap()
        .args([
            "user",
            "add",
            path.to_str().unwrap(),
            "--username",
            "bob",
            "--role",
            "viewer",
            "--password",
            "short",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("at least"));
}

#[test]
fn cannot_remove_last_admin() {
    let dir = tempfile::tempdir().unwrap();
    let path = sales_file(&dir);
    Command::cargo_bin("almagest")
        .unwrap()
        .args([
            "user",
            "add",
            path.to_str().unwrap(),
            "--username",
            "admin",
            "--role",
            "admin",
            "--password",
            "supersecret",
        ])
        .assert()
        .success();
    Command::cargo_bin("almagest")
        .unwrap()
        .args([
            "user",
            "remove",
            path.to_str().unwrap(),
            "--username",
            "admin",
            "--yes",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("last admin"));
}

#[test]
fn doctor_reports_healthy_environment() {
    Command::cargo_bin("almagest")
        .unwrap()
        .arg("doctor")
        .assert()
        .success()
        .stdout(predicate::str::contains("query engine"))
        .stdout(predicate::str::contains("sqlite"));
}

#[test]
fn import_dashboard_from_json() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.alm");
    Command::cargo_bin("almagest")
        .unwrap()
        .args(["new", path.to_str().unwrap()])
        .assert()
        .success();

    let dash = dir.path().join("d.json");
    std::fs::write(
        &dash,
        r#"{"version":1,"name":"Imported","layout":{"rows":[{"panels":[
            {"id":"t","span":12,"kind":"text","content":"hi"}]}]}}"#,
    )
    .unwrap();

    Command::cargo_bin("almagest")
        .unwrap()
        .args(["import", path.to_str().unwrap(), dash.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported dashboard"));
}
