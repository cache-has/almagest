// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests driving the router directly via `tower::oneshot` — no port
//! binding, no HTTP client dependency. Each test builds an in-memory fixture
//! (`.alm` with one dataset and one dashboard), then exercises an endpoint.

use crate::build_router;
use crate::state::AppState;
use almagest_core::{AlmagestFile, Compression};
use almagest_query::AlmagestQueryContext;
use arrow::array::{Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use std::sync::Arc;
use tower::ServiceExt;

/// A built fixture: the temp dir (kept alive), the router, and the dashboard id.
struct Fixture {
    _dir: tempfile::TempDir,
    router: Router,
    dashboard_id: String,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.alm");
    let mut file = AlmagestFile::create(&path).unwrap();
    file.set_title("Test Almagest").unwrap();

    // One dataset: sales(region, amount).
    let schema = Arc::new(Schema::new(vec![
        Field::new("region", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["EU", "EU", "US"])),
            Arc::new(Int64Array::from(vec![100, 50, 30])),
        ],
    )
    .unwrap();
    file.put_dataset("sales", schema, &[batch], Compression::Zstd)
        .unwrap();

    // One dashboard: a metric filtered by a `region` select parameter, plus a
    // dynamic-options parameter backed by an options_query.
    let dashboard_json = r#"{
        "version": 1,
        "name": "Sales",
        "parameters": [
            {"id": "region", "kind": "select", "options": ["EU", "US"], "default": "EU"},
            {"id": "region_dyn", "kind": "select",
             "options_query": "SELECT DISTINCT region FROM sales ORDER BY region",
             "default": "EU"}
        ],
        "layout": {
            "rows": [
                {"panels": [
                    {"id": "rev", "span": 4, "kind": "metric",
                     "query": {"sql": "SELECT SUM(amount) AS value FROM sales WHERE region = {{region}}"}},
                    {"id": "note", "span": 8, "kind": "text", "content": "Hello"}
                ]}
            ]
        }
    }"#;
    let dashboard_id = file
        .create_dashboard("Sales", None, None, dashboard_json)
        .unwrap();

    let query = AlmagestQueryContext::open(&file).unwrap();
    let state = AppState::new(file, query);
    let router = build_router(state, false);

    Fixture {
        _dir: dir,
        router,
        dashboard_id,
    }
}

/// Like [`fixture`] but served read-only (every mutation should be rejected).
fn read_only_fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ro.alm");
    let mut file = AlmagestFile::create(&path).unwrap();
    let schema = Arc::new(Schema::new(vec![Field::new(
        "region",
        DataType::Utf8,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(StringArray::from(vec!["EU"]))],
    )
    .unwrap();
    file.put_dataset("sales", schema, &[batch], Compression::Zstd)
        .unwrap();
    let dashboard_id = file
        .create_dashboard(
            "RO",
            None,
            None,
            r#"{"version":1,"name":"RO","layout":{"rows":[{"panels":[
                {"id":"t","span":12,"kind":"text","content":"hi"}]}]}}"#,
        )
        .unwrap();
    let query = AlmagestQueryContext::open(&file).unwrap();
    let state = AppState::new(file, query).with_flags(true, false);
    Fixture {
        _dir: dir,
        router: build_router(state, false),
        dashboard_id,
    }
}

/// Send a request and return (status, headers, body bytes).
async fn send(
    router: &Router,
    request: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    (status, headers, bytes)
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

fn post_json(uri: &str, json: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json.to_string()))
        .unwrap()
}

fn post_bytes(uri: &str, content_type: &str, bytes: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, content_type)
        .body(bytes.into())
        .unwrap()
}

#[tokio::test]
async fn meta_reports_title_and_dashboard_count() {
    let fx = fixture();
    let (status, _h, body) = send(&fx.router, get("/api/almagest")).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["title"], "Test Almagest");
    assert_eq!(v["dashboard_count"], 1);
    assert_eq!(v["format_version"], almagest_core::FORMAT_VERSION);
    // Auth is opt-in: a freshly created file is in no-auth mode.
    assert_eq!(v["auth_enabled"], false);
}

#[tokio::test]
async fn lists_and_fetches_dashboard() {
    let fx = fixture();
    let (status, _h, body) = send(&fx.router, get("/api/almagest/dashboards")).await;
    assert_eq!(status, StatusCode::OK);
    let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "Sales");

    let uri = format!("/api/almagest/dashboards/{}", fx.dashboard_id);
    let (status, _h, body) = send(&fx.router, get(&uri)).await;
    assert_eq!(status, StatusCode::OK);
    let dash: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(dash["name"], "Sales");
    assert_eq!(dash["layout"]["rows"][0]["panels"][0]["id"], "rev");
}

#[tokio::test]
async fn missing_dashboard_is_404() {
    let fx = fixture();
    let (status, _h, body) = send(&fx.router, get("/api/almagest/dashboards/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["error"]["code"], "not_found");
}

#[tokio::test]
async fn schema_lists_registered_tables() {
    let fx = fixture();
    let (status, _h, body) = send(&fx.router, get("/api/almagest/schema")).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let tables = v["tables"].as_array().unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0]["name"], "sales");
    assert_eq!(tables[0]["row_count"], 3);
}

#[tokio::test]
async fn panel_execute_returns_arrow_with_default_param() {
    let fx = fixture();
    let req = post_json(
        "/api/almagest/panels/execute",
        serde_json::json!({
            "dashboard_id": fx.dashboard_id,
            "panel_id": "rev",
            "parameters": {}
        }),
    );
    let (status, headers, body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/vnd.apache.arrow.stream"
    );
    assert_eq!(headers.get("x-almagest-row-count").unwrap(), "1");

    // Decode the Arrow IPC stream and confirm the aggregate (EU: 100 + 50).
    let reader = arrow::ipc::reader::StreamReader::try_new(body.as_slice(), None).unwrap();
    let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
    assert_eq!(batches.len(), 1);
    let value = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(value.value(0), 150);
}

#[tokio::test]
async fn panel_execute_honors_provided_param() {
    let fx = fixture();
    let req = post_json(
        "/api/almagest/panels/execute",
        serde_json::json!({
            "dashboard_id": fx.dashboard_id,
            "panel_id": "rev",
            "parameters": {"region": "US"}
        }),
    );
    let (status, _h, body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::OK);
    let reader = arrow::ipc::reader::StreamReader::try_new(body.as_slice(), None).unwrap();
    let batches: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
    let value = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(value.value(0), 30);
}

#[tokio::test]
async fn panel_execute_on_text_panel_is_bad_request() {
    let fx = fixture();
    let req = post_json(
        "/api/almagest/panels/execute",
        serde_json::json!({
            "dashboard_id": fx.dashboard_id,
            "panel_id": "note",
            "parameters": {}
        }),
    );
    let (status, _h, _body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn options_static_and_dynamic() {
    let fx = fixture();
    // Static options.
    let req = post_json(
        "/api/almagest/options",
        serde_json::json!({"dashboard_id": fx.dashboard_id, "parameter": "region"}),
    );
    let (status, _h, body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["options"], serde_json::json!(["EU", "US"]));

    // Dynamic options resolved by running the options_query.
    let req = post_json(
        "/api/almagest/options",
        serde_json::json!({"dashboard_id": fx.dashboard_id, "parameter": "region_dyn"}),
    );
    let (status, _h, body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["options"], serde_json::json!(["EU", "US"]));
}

#[tokio::test]
async fn dashboard_crud_lifecycle() {
    let fx = fixture();
    let body = serde_json::json!({
        "version": 1,
        "name": "Created",
        "layout": {"rows": [{"panels": [
            {"id": "t", "span": 12, "kind": "text", "content": "hi"}
        ]}]}
    });
    // Create.
    let (status, _h, resp) = send(&fx.router, post_json("/api/almagest/dashboards", body)).await;
    assert_eq!(status, StatusCode::CREATED);
    let created: serde_json::Value = serde_json::from_slice(&resp).unwrap();
    let new_id = created["id"].as_str().unwrap().to_string();

    // Update.
    let update = serde_json::json!({
        "version": 1,
        "name": "Renamed",
        "layout": {"rows": [{"panels": [
            {"id": "t", "span": 12, "kind": "text", "content": "bye"}
        ]}]}
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/almagest/dashboards/{new_id}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(update.to_string()))
        .unwrap();
    let (status, _h, _b) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Delete.
    let req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/almagest/dashboards/{new_id}"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Gone.
    let (status, _h, _b) = send(
        &fx.router,
        get(&format!("/api/almagest/dashboards/{new_id}")),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn export_then_import_roundtrips() {
    let fx = fixture();
    let uri = format!("/api/almagest/export/dashboard/{}", fx.dashboard_id);
    let (status, headers, body) = send(&fx.router, post_json(&uri, serde_json::Value::Null)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_DISPOSITION)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("attachment")
    );

    // Re-import the exported JSON as a new dashboard.
    let exported: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let (status, _h, resp) = send(
        &fx.router,
        post_json("/api/almagest/import/dashboard", exported),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let created: serde_json::Value = serde_json::from_slice(&resp).unwrap();
    assert!(created["id"].as_str().is_some());
}

#[tokio::test]
async fn serves_frontend_index() {
    let fx = fixture();
    let (status, headers, body) = send(&fx.router, get("/")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-cache");
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<!DOCTYPE html>"));
}

#[tokio::test]
async fn spa_fallback_serves_index_for_unknown_route() {
    let fx = fixture();
    let (status, _h, body) = send(&fx.router, get("/dashboard/some-name")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body).contains("<!DOCTYPE html>"));
}

// --- ingest / dataset management --------------------------------------------

const CSV: &str = "city,pop\nAustin,1000\nDallas,2000\n";

async fn dataset_names(router: &Router) -> Vec<String> {
    let (_s, _h, body) = send(router, get("/api/almagest/datasets")).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap().to_string())
        .collect()
}

async fn schema_table_names(router: &Router) -> Vec<String> {
    let (_s, _h, body) = send(router, get("/api/almagest/schema")).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    v["tables"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn ingest_csv_persists_and_registers_for_query() {
    let fx = fixture();
    let req = post_bytes(
        "/api/almagest/datasets?format=csv&name=cities",
        "text/csv",
        CSV,
    );
    let (status, _h, body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::CREATED);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["dry_run"], false);
    assert_eq!(res["datasets"][0]["name"], "cities");
    assert_eq!(res["datasets"][0]["row_count"], 2);

    // Listed, and — crucially — registered in the rebuilt query context.
    assert!(
        dataset_names(&fx.router)
            .await
            .contains(&"cities".to_string())
    );
    assert!(
        schema_table_names(&fx.router)
            .await
            .contains(&"cities".to_string())
    );
}

#[tokio::test]
async fn ingest_dry_run_previews_without_persisting() {
    let fx = fixture();
    let req = post_bytes(
        "/api/almagest/datasets?format=csv&name=preview_me&dry_run=true",
        "text/csv",
        CSV,
    );
    let (status, _h, body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::OK);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["dry_run"], true);
    assert_eq!(res["datasets"][0]["columns"].as_array().unwrap().len(), 2);

    // Nothing was written to the real file.
    assert!(
        !dataset_names(&fx.router)
            .await
            .contains(&"preview_me".to_string())
    );
}

#[tokio::test]
async fn ingest_json_ndjson_autodetect() {
    let fx = fixture();
    let ndjson = "{\"a\":1,\"b\":\"x\"}\n{\"a\":2,\"b\":\"y\"}\n";
    let req = post_bytes(
        "/api/almagest/datasets?format=json&name=events",
        "application/json",
        ndjson,
    );
    let (status, _h, body) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::CREATED);
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["datasets"][0]["row_count"], 2);
}

#[tokio::test]
async fn ingest_infers_format_from_filename() {
    let fx = fixture();
    let req = post_bytes("/api/almagest/datasets?filename=towns.csv", "text/csv", CSV);
    let (status, _h, _b) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        dataset_names(&fx.router)
            .await
            .contains(&"towns".to_string())
    );
}

#[tokio::test]
async fn ingest_empty_and_unknown_format_are_bad_request() {
    let fx = fixture();
    let (s1, _h, _b) = send(
        &fx.router,
        post_bytes("/api/almagest/datasets?format=csv", "text/csv", ""),
    )
    .await;
    assert_eq!(s1, StatusCode::BAD_REQUEST);

    let (s2, _h, _b) = send(
        &fx.router,
        post_bytes("/api/almagest/datasets?format=xlsx", "x", CSV),
    )
    .await;
    assert_eq!(s2, StatusCode::BAD_REQUEST);

    let (s3, _h, _b) = send(&fx.router, post_bytes("/api/almagest/datasets", "x", CSV)).await;
    assert_eq!(s3, StatusCode::BAD_REQUEST); // no format and no filename
}

#[tokio::test]
async fn rename_then_delete_dataset() {
    let fx = fixture();
    // The fixture already has `sales`. Rename it.
    let req = post_json(
        "/api/almagest/datasets/sales/rename",
        serde_json::json!({ "to": "revenue" }),
    );
    let (status, _h, _b) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let tables = schema_table_names(&fx.router).await;
    assert!(tables.contains(&"revenue".to_string()));
    assert!(!tables.contains(&"sales".to_string()));

    // Delete it.
    let del = Request::builder()
        .method("DELETE")
        .uri("/api/almagest/datasets/revenue")
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&fx.router, del).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        !schema_table_names(&fx.router)
            .await
            .contains(&"revenue".to_string())
    );
}

#[tokio::test]
async fn delete_missing_dataset_is_404() {
    let fx = fixture();
    let del = Request::builder()
        .method("DELETE")
        .uri("/api/almagest/datasets/nope")
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&fx.router, del).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- asset upload / delete ---------------------------------------------------

#[tokio::test]
async fn asset_upload_get_delete_roundtrip() {
    let fx = fixture();
    let png = vec![0x89u8, 0x50, 0x4e, 0x47, 1, 2, 3, 4];

    let put = Request::builder()
        .method("PUT")
        .uri("/api/almagest/assets/logo.png")
        .header(header::CONTENT_TYPE, "image/png")
        .body(Body::from(png.clone()))
        .unwrap();
    let (status, _h, _b) = send(&fx.router, put).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Listed.
    let (_s, _h, body) = send(&fx.router, get("/api/almagest/assets")).await;
    let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list[0]["path"], "logo.png");

    // Fetched with the right type and bytes.
    let (status, headers, body) = send(&fx.router, get("/api/almagest/assets/logo.png")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/png");
    assert_eq!(body, png);

    // Deleted.
    let del = Request::builder()
        .method("DELETE")
        .uri("/api/almagest/assets/logo.png")
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&fx.router, del).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _h, _b) = send(&fx.router, get("/api/almagest/assets/logo.png")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- read-only mode + heartbeat lifecycle ------------------------------------

#[tokio::test]
async fn read_only_meta_advertises_the_flag() {
    let fx = read_only_fixture();
    let (status, _h, body) = send(&fx.router, get("/api/almagest")).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["read_only"], true);
}

#[tokio::test]
async fn read_only_rejects_writes_but_allows_reads() {
    let fx = read_only_fixture();

    // Reads still work.
    let (status, _h, _b) = send(&fx.router, get("/api/almagest/dashboards")).await;
    assert_eq!(status, StatusCode::OK);

    // Every mutation is 403 Forbidden.
    let create = post_json(
        "/api/almagest/dashboards",
        serde_json::json!({"version":1,"name":"X","layout":{"rows":[]}}),
    );
    let (status, _h, body) = send(&fx.router, create).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["error"]["code"], "forbidden");

    let del = Request::builder()
        .method("DELETE")
        .uri(format!("/api/almagest/dashboards/{}", fx.dashboard_id))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&fx.router, del).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let ingest = post_bytes(
        "/api/almagest/datasets?format=csv&name=x",
        "text/csv",
        "a,b\n1,2\n",
    );
    let (status, _h, _b) = send(&fx.router, ingest).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let put = Request::builder()
        .method("PUT")
        .uri("/api/almagest/assets/x.png")
        .header(header::CONTENT_TYPE, "image/png")
        .body(Body::from(vec![1u8, 2, 3]))
        .unwrap();
    let (status, _h, _b) = send(&fx.router, put).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn heartbeat_endpoint_accepts_pings() {
    let fx = fixture();
    let (status, _h, _b) = send(
        &fx.router,
        Request::builder()
            .method("POST")
            .uri("/api/almagest/heartbeat")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

// --- host-provided auth gate (embedding) -------------------------------------

#[tokio::test]
async fn auth_hook_gates_requests_by_header() {
    use std::sync::Arc;
    let fx = fixture();
    // Require a specific host header to be present and correct.
    let hook: crate::AuthHook = Arc::new(|headers: &header::HeaderMap| {
        headers
            .get("x-host-user")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == "trusted")
            .unwrap_or(false)
    });
    let gated = crate::with_auth(fx.router.clone(), hook);

    // No header → 401.
    let (status, _h, _b) = send(&gated, get("/api/almagest")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Wrong header → 401.
    let req = Request::builder()
        .uri("/api/almagest")
        .header("x-host-user", "imposter")
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&gated, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Correct header → passes through to the handler.
    let req = Request::builder()
        .uri("/api/almagest")
        .header("x-host-user", "trusted")
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&gated, req).await;
    assert_eq!(status, StatusCode::OK);
}

// --- local-account auth & multi-user (doc 13) --------------------------------

/// Pull the `alm_session` / `alm_csrf` values out of a response's `Set-Cookie`
/// headers and assemble a `Cookie` header value plus the CSRF token.
fn auth_cookies(h: &axum::http::HeaderMap) -> (String, String) {
    let mut session = String::new();
    let mut csrf = String::new();
    for v in h.get_all(header::SET_COOKIE) {
        let s = v.to_str().unwrap();
        if let Some(rest) = s.strip_prefix("alm_session=") {
            session = rest.split(';').next().unwrap().to_string();
        } else if let Some(rest) = s.strip_prefix("alm_csrf=") {
            csrf = rest.split(';').next().unwrap().to_string();
        }
    }
    (format!("alm_session={session}; alm_csrf={csrf}"), csrf)
}

/// A request carrying an auth cookie (and CSRF header for mutations).
fn authed(
    method: &str,
    uri: &str,
    cookie: &str,
    csrf: &str,
    json: Option<serde_json::Value>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::COOKIE, cookie)
        .header("x-csrf-token", csrf);
    if json.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/json");
    }
    let body = json
        .map(|j| Body::from(j.to_string()))
        .unwrap_or(Body::empty());
    b.body(body).unwrap()
}

/// Run the first-admin setup flow on a fixture and return (cookie, csrf).
async fn enable_auth_as_admin(fx: &Fixture) -> (String, String) {
    let (status, h, body) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/setup",
            serde_json::json!({"username": "admin", "password": "supersecret"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "setup should succeed");
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["user"]["role"], "admin");
    auth_cookies(&h)
}

#[tokio::test]
async fn setup_enables_auth_then_me_reports_user() {
    let fx = fixture();
    // Before setup: auth off, me says needs no login.
    let (_s, _h, body) = send(&fx.router, get("/api/almagest/auth/me")).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["auth_enabled"], false);

    let (cookie, _csrf) = enable_auth_as_admin(&fx).await;

    // Auth is now enforced; meta advertises it.
    let (_s, _h, body) = send(&fx.router, get("/api/almagest")).await;
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["auth_enabled"], true);

    // me with the session cookie returns the admin user.
    let (status, _h, body) = send(
        &fx.router,
        Request::builder()
            .uri("/api/almagest/auth/me")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["user"]["username"], "admin");

    // A second setup is rejected — the file already has an admin.
    let (status, _h, _b) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/setup",
            serde_json::json!({"username": "x", "password": "anotherpw1"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn protected_routes_require_a_session() {
    let fx = fixture();
    let (cookie, _csrf) = enable_auth_as_admin(&fx).await;

    // No cookie → 401.
    let (status, _h, _b) = send(&fx.router, get("/api/almagest/dashboards")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // With the session cookie → 200.
    let (status, _h, _b) = send(
        &fx.router,
        Request::builder()
            .uri("/api/almagest/dashboards")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn login_rejects_bad_credentials() {
    let fx = fixture();
    enable_auth_as_admin(&fx).await;

    let (status, _h, _b) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/login",
            serde_json::json!({"username": "admin", "password": "wrong"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Unknown user is also 401 (no user enumeration).
    let (status, _h, _b) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/login",
            serde_json::json!({"username": "ghost", "password": "whatever1"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Correct credentials succeed.
    let (status, _h, _b) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/login",
            serde_json::json!({"username": "admin", "password": "supersecret"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn csrf_required_for_writes() {
    let fx = fixture();
    let (cookie, csrf) = enable_auth_as_admin(&fx).await;
    let dash = serde_json::json!({
        "version": 1, "name": "New",
        "layout": {"rows": [{"panels": [{"id": "t", "span": 12, "kind": "text", "content": "hi"}]}]}
    });

    // Session cookie but no CSRF header → 403.
    let req = Request::builder()
        .method("POST")
        .uri("/api/almagest/dashboards")
        .header(header::COOKIE, &cookie)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(dash.to_string()))
        .unwrap();
    let (status, _h, _b) = send(&fx.router, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // With the matching CSRF header → 201.
    let (status, _h, _b) = send(
        &fx.router,
        authed(
            "POST",
            "/api/almagest/dashboards",
            &cookie,
            &csrf,
            Some(dash),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn viewer_cannot_write_but_can_read() {
    let fx = fixture();
    let (admin_cookie, admin_csrf) = enable_auth_as_admin(&fx).await;

    // Admin creates a viewer.
    let (status, _h, _b) = send(
        &fx.router,
        authed(
            "POST",
            "/api/almagest/admin/users",
            &admin_cookie,
            &admin_csrf,
            Some(
                serde_json::json!({"username": "val", "password": "viewerpass1", "role": "viewer"}),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    // Log in as the viewer.
    let (status, h, _b) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/login",
            serde_json::json!({"username": "val", "password": "viewerpass1"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (viewer_cookie, viewer_csrf) = auth_cookies(&h);

    // Viewer can read dashboards.
    let (status, _h, _b) = send(
        &fx.router,
        Request::builder()
            .uri("/api/almagest/dashboards")
            .header(header::COOKIE, &viewer_cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Viewer cannot create a dashboard (403), even with a valid CSRF token.
    let dash = serde_json::json!({
        "version": 1, "name": "Nope",
        "layout": {"rows": [{"panels": [{"id": "t", "span": 12, "kind": "text", "content": "x"}]}]}
    });
    let (status, _h, _b) = send(
        &fx.router,
        authed(
            "POST",
            "/api/almagest/dashboards",
            &viewer_cookie,
            &viewer_csrf,
            Some(dash),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Viewer cannot reach admin endpoints.
    let (status, _h, _b) = send(
        &fx.router,
        Request::builder()
            .uri("/api/almagest/admin/users")
            .header(header::COOKIE, &viewer_cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn last_admin_is_protected() {
    let fx = fixture();
    let (cookie, csrf) = enable_auth_as_admin(&fx).await;

    // Find the admin's id.
    let (_s, _h, body) = send(
        &fx.router,
        Request::builder()
            .uri("/api/almagest/admin/users")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let users: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let admin_id = users[0]["id"].as_str().unwrap().to_string();

    // Demoting the sole admin is rejected.
    let (status, _h, _b) = send(
        &fx.router,
        authed(
            "PUT",
            &format!("/api/almagest/admin/users/{admin_id}"),
            &cookie,
            &csrf,
            Some(serde_json::json!({"role": "viewer"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Deleting the sole admin is rejected.
    let (status, _h, _b) = send(
        &fx.router,
        authed(
            "DELETE",
            &format!("/api/almagest/admin/users/{admin_id}"),
            &cookie,
            &csrf,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn audit_log_records_auth_events() {
    let fx = fixture();
    let (cookie, _csrf) = enable_auth_as_admin(&fx).await;

    let (status, _h, body) = send(
        &fx.router,
        Request::builder()
            .uri("/api/almagest/admin/audit")
            .header(header::COOKIE, &cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let kinds: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event_kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"auth_enabled"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"login"), "kinds: {kinds:?}");
    assert!(kinds.contains(&"user_created"), "kinds: {kinds:?}");
}

#[tokio::test]
async fn change_password_then_login_with_new() {
    let fx = fixture();
    let (cookie, csrf) = enable_auth_as_admin(&fx).await;

    // Wrong current password → 401.
    let (status, _h, _b) = send(
        &fx.router,
        authed(
            "POST",
            "/api/almagest/auth/change-password",
            &cookie,
            &csrf,
            Some(serde_json::json!({"current_password": "nope", "new_password": "brandnew123"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Correct current password → 204.
    let (status, _h, _b) = send(
        &fx.router,
        authed(
            "POST",
            "/api/almagest/auth/change-password",
            &cookie,
            &csrf,
            Some(serde_json::json!({"current_password": "supersecret", "new_password": "brandnew123"})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The new password works; the old one doesn't.
    let (status, _h, _b) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/login",
            serde_json::json!({"username": "admin", "password": "brandnew123"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _h, _b) = send(
        &fx.router,
        post_json(
            "/api/almagest/auth/login",
            serde_json::json!({"username": "admin", "password": "supersecret"}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
