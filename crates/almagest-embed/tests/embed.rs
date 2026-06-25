// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Integration tests for the embedding API. They start real bound servers (the
// host's-runtime path) and hit them over a dependency-free raw-TCP HTTP/1.1
// client — no reqwest/hyper dev-dependency, in keeping with the project's
// minimize-deps value.

use almagest_core::{AlmagestFile, Compression};
use almagest_embed::AlmagestServer;
use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Write a fixture `.alm` with one dataset + one named dashboard. Returns nothing
/// — the caller passes the path to the builder.
fn write_fixture(path: &Path, title: &str) {
    let mut file = AlmagestFile::create(path).unwrap();
    file.set_title(title).unwrap();
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["EU", "US"])),
            Arc::new(Int64Array::from(vec![100, 30])),
        ],
    )
    .unwrap();
    file.put_dataset("sales", schema, &[batch], Compression::Zstd)
        .unwrap();
    file.create_dashboard(
        "Main",
        None,
        None,
        r#"{"version":1,"name":"Main","layout":{"rows":[{"panels":[
            {"id":"t","span":12,"kind":"text","content":"hi"}]}]}}"#,
    )
    .unwrap();
    file.close().unwrap();
}

/// Minimal HTTP/1.1 request over a fresh connection; returns (status, body).
async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<(&str, &str)>, // (content-type, body)
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some((ct, b)) = body {
        req.push_str(&format!(
            "Content-Type: {ct}\r\nContent-Length: {}\r\n",
            b.len()
        ));
        req.push_str("\r\n");
        req.push_str(b);
    } else {
        req.push_str("\r\n");
    }
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status: u16 = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    (status, body.to_string())
}

async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    http(addr, "GET", path, &[], None).await
}

#[tokio::test]
async fn serves_an_embedded_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.alm");
    write_fixture(&path, "Embedded A");

    let server = AlmagestServer::builder()
        .alm_file(&path)
        .bind_address("127.0.0.1:0")
        .start()
        .await
        .unwrap();

    let (status, body) = get(server.addr(), "/api/almagest").await;
    assert_eq!(status, 200);
    assert!(body.contains("Embedded A"), "meta body: {body}");

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn two_instances_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let pa = dir.path().join("a.alm");
    let pb = dir.path().join("b.alm");
    write_fixture(&pa, "Customer A");
    write_fixture(&pb, "Customer B");

    let a = AlmagestServer::builder()
        .alm_file(&pa)
        .start()
        .await
        .unwrap();
    let b = AlmagestServer::builder()
        .alm_file(&pb)
        .start()
        .await
        .unwrap();

    // Distinct ephemeral ports, each serving its own file.
    assert_ne!(a.addr(), b.addr());
    let (sa, ba) = get(a.addr(), "/api/almagest").await;
    let (sb, bb) = get(b.addr(), "/api/almagest").await;
    assert_eq!((sa, sb), (200, 200));
    assert!(ba.contains("Customer A"));
    assert!(bb.contains("Customer B"));

    a.shutdown().await.unwrap();
    b.shutdown().await.unwrap();
}

#[tokio::test]
async fn auth_hook_rejects_then_accepts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.alm");
    write_fixture(&path, "Gated");

    let server = AlmagestServer::builder()
        .alm_file(&path)
        .auth_hook(|h| h.get("x-host-user").is_some())
        .start()
        .await
        .unwrap();

    // No header → 401.
    let (status, _) = get(server.addr(), "/api/almagest").await;
    assert_eq!(status, 401);

    // With header → 200.
    let (status, _) = http(
        server.addr(),
        "GET",
        "/api/almagest",
        &[("x-host-user", "alice")],
        None,
    )
    .await;
    assert_eq!(status, 200);

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn read_only_blocks_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.alm");
    write_fixture(&path, "RO");

    let server = AlmagestServer::builder()
        .alm_file(&path)
        .read_only(true)
        .start()
        .await
        .unwrap();

    let (status, _) = http(
        server.addr(),
        "POST",
        "/api/almagest/dashboards",
        &[],
        Some((
            "application/json",
            r#"{"version":1,"name":"x","layout":{"rows":[]}}"#,
        )),
    )
    .await;
    assert_eq!(status, 403);

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_file_path_errors() {
    let res = AlmagestServer::builder().start().await;
    assert!(matches!(res, Err(almagest_embed::EmbedError::NoFile)));
}
