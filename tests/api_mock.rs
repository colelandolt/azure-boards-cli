//! Integration tests against a mocked Azure DevOps HTTP server (wiremock),
//! exercising request shapes, response envelopes, error mapping, pagination,
//! retry, and dry-run network silence.

use serde_json::{json, Value};
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn run_ab(server_uri: &str, args: &[&str]) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let uri = server_uri.to_string();
    tokio::task::spawn_blocking(move || {
        let tmp = std::env::temp_dir().join(format!("ab-mock-home-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).ok();
        // Do NOT env_clear(): on Windows that strips SystemRoot and breaks the
        // child process's networking, so it can never reach the mock server.
        // Remove only the vars that would interfere; the explicit ADO_PAT below
        // pins auth to the env-PAT path.
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_azure-boards"));
        for var in [
            "ADO_ORG",
            "ADO_PROJECT",
            "ADO_TEAM",
            "ADO_TOKEN",
            "AZURE_DEVOPS_EXT_PAT",
            "AZURE_BOARDS_CLIENT_ID",
        ] {
            command.env_remove(var);
        }
        let out = command
            .env("HOME", &tmp)
            .env("XDG_CONFIG_HOME", tmp.join(".config"))
            .env("NO_COLOR", "1")
            .env("ADO_PAT", "test-pat")
            .env("AZURE_BOARDS_API_BASE", &uri)
            .args(&args)
            .output()
            .expect("spawn ab");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    })
    .await
    .unwrap()
}

const CTX: &[&str] = &[
    "--org",
    "testorg",
    "--project",
    "Proj",
    "--detect",
    "false",
    "--json",
];

fn with_ctx<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut v = args.to_vec();
    v.extend_from_slice(CTX);
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_404_maps_to_exit_5_with_stable_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("//testorg/Proj/_apis/wit/workitems/99"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "message": "TF401232: Work item 99 does not exist",
            "typeKey": "WorkItemNotFoundException",
        })))
        .mount(&server)
        .await;
    let (code, stdout, stderr) =
        run_ab(&server.uri(), &with_ctx(&["work-item", "show", "99"])).await;
    assert_eq!(code, 5, "stderr: {stderr}");
    assert!(stdout.is_empty());
    let err: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(err["code"], "notFound");
    assert!(err["message"].as_str().unwrap().contains("TF401232"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auth_failure_maps_to_exit_3() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"message": "bad token"})))
        .mount(&server)
        .await;
    let (code, _, _) = run_ab(&server.uri(), &with_ctx(&["work-item", "show", "1"])).await;
    assert_eq!(code, 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wiql_fetch_hydrates_in_query_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/testorg/Proj/_apis/wit/wiql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "queryType": "flat",
            "workItems": [{"id": 7}, {"id": 3}, {"id": 5}],
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("//testorg/Proj/_apis/wit/workitemsbatch"))
        .and(body_partial_json(json!({"ids": [7, 3, 5]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count": 3,
            "value": [
                {"id": 3, "rev": 1, "url": "https://dev.azure.com/x/3", "fields": {"System.Title": "three"}},
                {"id": 5, "rev": 1, "url": "https://dev.azure.com/x/5", "fields": {"System.Title": "five"}},
                {"id": 7, "rev": 1, "url": "https://dev.azure.com/x/7", "fields": {"System.Title": "seven"}},
            ],
        })))
        .mount(&server)
        .await;
    let (code, stdout, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&[
            "wiql",
            "fetch",
            "SELECT [System.Id] FROM WorkItems ORDER BY [System.Id] DESC",
        ]),
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["count"], 3);
    let ids: Vec<i64> = v["value"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![7, 3, 5], "ORDER BY must be preserved");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn comment_list_follows_continuation_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/testorg/Proj/_apis/wit/workItems/12/comments"))
        .and(query_param("continuationToken", "page2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "comments": [{"id": 2, "text": "second"}],
            "count": 1,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/testorg/Proj/_apis/wit/workItems/12/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "comments": [{"id": 1, "text": "first"}],
            "count": 1,
            "continuationToken": "page2",
        })))
        .mount(&server)
        .await;
    let (code, stdout, stderr) = run_ab(&server.uri(), &with_ctx(&["comment", "list", "12"])).await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["count"], 2, "both pages collected: {v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_limit_retried_with_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/testorg/Proj/_apis/wit/wiql"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "1")
                .set_body_json(json!({"message": "throttled"})),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/testorg/Proj/_apis/wit/wiql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "queryType": "flat",
            "workItems": [],
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (code, stdout, _) = run_ab(
        &server.uri(),
        &with_ctx(&["wiql", "run", "SELECT [System.Id] FROM WorkItems"]),
    )
    .await;
    assert_eq!(code, 0);
    assert!(stdout.contains("workItems"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_makes_zero_write_requests() {
    let server = MockServer::start().await;
    // No mocks mounted: ANY request would 404 and the strict count below
    // would fail. Update --dry-run must not touch the network at all.
    let (code, stdout, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&["work-item", "update", "5", "--state", "Active", "--dry-run"]),
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(server.received_requests().await.unwrap().len(), 0);
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["dryRun"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_sends_json_patch_and_expected_rev_conflict_maps_to_6() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("//testorg/Proj/_apis/wit/workitems/5"))
        .and(body_partial_json(json!([
            {"op": "test", "path": "/rev", "value": 3}
        ])))
        .respond_with(ResponseTemplate::new(409).set_body_json(json!({
            "message": "the test operation for path /rev failed",
        })))
        .mount(&server)
        .await;
    let (code, _, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&[
            "work-item",
            "update",
            "5",
            "--state",
            "Active",
            "--expected-rev",
            "3",
        ]),
    )
    .await;
    assert_eq!(code, 6, "stderr: {stderr}");
    let err: Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(err["code"], "conflict");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tag_add_preserves_existing_tags() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("//testorg/Proj/_apis/wit/workitems/8"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 8, "rev": 2, "url": "https://dev.azure.com/x/8",
            "fields": {"System.Tags": "Existing; QA"},
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("//testorg/Proj/_apis/wit/workitems/8"))
        .and(body_partial_json(json!([
            {"op": "add", "path": "/fields/System.Tags", "value": "Existing; QA; new-tag"}
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 8, "rev": 3, "url": "https://dev.azure.com/x/8",
            "fields": {"System.Tags": "Existing; QA; new-tag"},
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (code, stdout, stderr) =
        run_ab(&server.uri(), &with_ctx(&["tag", "add", "8", "new-tag"])).await;
    assert_eq!(code, 0, "stderr: {stderr}");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["result"]["tags"], "Existing; QA; new-tag");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relation_remove_uses_rev_test_and_index() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/testorg/Proj/_apis/wit/workitems/9"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 9, "rev": 4,
            "relations": [
                {"rel": "System.LinkTypes.Hierarchy-Reverse", "url": "https://dev.azure.com/testorg/_apis/wit/workItems/1"},
                {"rel": "System.LinkTypes.Related", "url": "https://dev.azure.com/testorg/_apis/wit/workItems/2"},
            ],
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("//testorg/Proj/_apis/wit/workitems/9"))
        .and(body_partial_json(json!([
            {"op": "test", "path": "/rev", "value": 4},
            {"op": "remove", "path": "/relations/1"}
        ])))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"id": 9, "rev": 5, "url": "https://dev.azure.com/x/9", "fields": {}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let (code, _, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&["relation", "remove", "9", "--relation-id", "1", "--yes"]),
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn batch_get_reports_list_envelope() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("//testorg/Proj/_apis/wit/workitemsbatch"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "count": 2,
            "value": [
                {"id": 1, "rev": 1, "url": "https://dev.azure.com/x/1", "fields": {"System.Title": "a"}},
                {"id": 2, "rev": 1, "url": "https://dev.azure.com/x/2", "fields": {"System.Title": "b"}},
            ],
        })))
        .mount(&server)
        .await;
    let (code, stdout, _) = run_ab(
        &server.uri(),
        &with_ctx(&["work-item", "batch-get", "--ids", "1,2"]),
    )
    .await;
    assert_eq!(code, 0);
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["count"], 2);
    assert_eq!(v["value"][0]["id"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_sends_expanded_file_content_in_request_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 42, "rev": 1, "url": "https://dev.azure.com/x/42", "fields": {}
        })))
        .mount(&server)
        .await;

    let dir = std::env::temp_dir().join(format!("ab-mock-atfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let desc = dir.join("desc.md");
    std::fs::write(&desc, "## Heading\nBody with `code` and a $dollar.").unwrap();
    let at = format!("@{}", desc.display());

    let (code, _stdout, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&[
            "work-item",
            "create",
            "--type",
            "Task",
            "--title",
            "T",
            "--description",
            &at,
        ]),
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let reqs = server.received_requests().await.unwrap();
    let post = reqs
        .iter()
        .find(|r| r.method == wiremock::http::Method::POST)
        .unwrap();
    let body: Value = serde_json::from_slice(&post.body).unwrap();
    let ops = body.as_array().expect("json-patch array");
    let desc_op = ops
        .iter()
        .find(|o| o["path"] == "/fields/System.Description")
        .expect("description op present");
    assert_eq!(
        desc_op["value"],
        "## Heading\nBody with `code` and a $dollar."
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_field_value_expands_at_file() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("//testorg/Proj/_apis/wit/workitems/5"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 5, "rev": 4, "url": "https://dev.azure.com/x/5", "fields": {}
        })))
        .mount(&server)
        .await;

    let dir = std::env::temp_dir().join(format!("ab-mock-upd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let body_file = dir.join("body.md");
    std::fs::write(&body_file, "Updated description from a file.").unwrap();
    let f = format!("System.Description=@{}", body_file.display());

    let (code, _o, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&["work-item", "update", "5", "-f", &f]),
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let reqs = server.received_requests().await.unwrap();
    let patch = reqs
        .iter()
        .find(|r| r.method == wiremock::http::Method::PATCH)
        .unwrap();
    let ops: Value = serde_json::from_slice(&patch.body).unwrap();
    let op = ops
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["path"] == "/fields/System.Description")
        .unwrap();
    assert_eq!(op["value"], "Updated description from a file.");
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_from_json_sends_all_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 7, "rev": 1, "url": "https://dev.azure.com/x/7", "fields": {}
        })))
        .mount(&server)
        .await;

    let dir = std::env::temp_dir().join(format!("ab-mock-fromjson-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plan = dir.join("story.json");
    std::fs::write(
        &plan,
        r#"{"type":"User Story","fields":{"System.Title":"JSON story","Microsoft.VSTS.Common.Priority":3}}"#,
    )
    .unwrap();
    let at = format!("@{}", plan.display());

    let (code, _o, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&["work-item", "create", "--from-json", &at]),
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");

    let reqs = server.received_requests().await.unwrap();
    let post = reqs
        .iter()
        .find(|r| r.method == wiremock::http::Method::POST)
        .unwrap();
    // Type goes in the URL path: .../workitems/$User Story
    assert!(
        post.url.path().contains("$User"),
        "type in path: {}",
        post.url.path()
    );
    let ops: Value = serde_json::from_slice(&post.body).unwrap();
    let arr = ops.as_array().unwrap();
    assert!(arr
        .iter()
        .any(|o| o["path"] == "/fields/System.Title" && o["value"] == "JSON story"));
    assert!(arr
        .iter()
        .any(|o| o["path"] == "/fields/Microsoft.VSTS.Common.Priority" && o["value"] == 3));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutation_query_id_resolves_from_response_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 4242, "rev": 1, "url": "https://dev.azure.com/x/4242", "fields": {}
        })))
        .mount(&server)
        .await;
    // `--query id` must yield the created id (hoisted from result.id), not null.
    let (code, stdout, stderr) = run_ab(
        &server.uri(),
        &with_ctx(&[
            "work-item",
            "create",
            "--type",
            "Task",
            "--title",
            "T",
            "--query",
            "id",
        ]),
    )
    .await;
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), "4242");
}
