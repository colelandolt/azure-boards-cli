//! Agent-contract tests that need no network: exit codes, JSON error objects,
//! prompt blocking, dry-run envelopes, completions.

use assert_cmd::Command;

fn ab() -> Command {
    let mut cmd = Command::cargo_bin("azure-boards").expect("binary builds");
    // Hermetic environment WITHOUT env_clear(): on Windows, clearing the whole
    // environment strips SystemRoot and breaks all networking (winsock/DNS), so
    // we only remove the vars that would influence context/auth resolution.
    for var in [
        "ADO_ORG",
        "ADO_PROJECT",
        "ADO_TEAM",
        "ADO_PAT",
        "ADO_TOKEN",
        "AZURE_DEVOPS_EXT_PAT",
        "AZURE_BOARDS_CLIENT_ID",
        "AZURE_BOARDS_API_BASE",
        "AZURE_BOARDS_CONFIG_DIR",
    ] {
        cmd.env_remove(var);
    }
    // Isolate config via AZURE_BOARDS_CONFIG_DIR: on Windows dirs::config_dir()
    // ignores HOME/XDG and would read/write the real per-user config.
    let tmp = std::env::temp_dir().join(format!("ab-test-home-{}", std::process::id()));
    let cfg = tmp.join("config");
    std::fs::create_dir_all(&cfg).ok();
    cmd.env("AZURE_BOARDS_CONFIG_DIR", &cfg)
        .env("HOME", &tmp)
        .env("XDG_CONFIG_HOME", tmp.join(".config"))
        .env("NO_COLOR", "1");
    cmd
}

#[test]
fn usage_error_exits_2() {
    ab().arg("work-item").assert().code(2);
}

#[test]
fn unresolvable_context_is_validation_exit_7_with_json_error() {
    let assert = ab()
        .args(["work-item", "show", "1", "--detect", "false", "--json"])
        .env("ADO_PAT", "fake")
        .assert()
        .code(7);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    let err: serde_json::Value =
        serde_json::from_str(stderr.lines().last().unwrap_or("")).expect("stderr is JSON");
    assert_eq!(err["code"], "validation");
    assert_eq!(err["exitCode"], 7);
    // stdout stays clean in JSON mode.
    assert!(assert.get_output().stdout.is_empty());
}

#[test]
fn non_tty_delete_without_yes_blocks_exit_9() {
    ab().args([
        "work-item",
        "delete",
        "5",
        "--org",
        "o",
        "--project",
        "p",
        "--detect",
        "false",
    ])
    .env("ADO_PAT", "fake")
    .assert()
    .code(9);
}

#[test]
fn destroy_requires_matching_confirm_id_even_with_yes() {
    // No --confirm-id at all.
    ab().args([
        "work-item",
        "delete",
        "5",
        "--destroy",
        "--yes",
        "--org",
        "o",
        "--project",
        "p",
        "--detect",
        "false",
    ])
    .env("ADO_PAT", "fake")
    .assert()
    .code(9);
    // Mismatched --confirm-id.
    ab().args([
        "work-item",
        "delete",
        "5",
        "--destroy",
        "--confirm-id",
        "6",
        "--yes",
        "--org",
        "o",
        "--project",
        "p",
        "--detect",
        "false",
    ])
    .env("ADO_PAT", "fake")
    .assert()
    .code(9);
}

#[test]
fn dry_run_create_emits_mutation_envelope_without_network() {
    let assert = ab()
        .args([
            "work-item",
            "create",
            "--type",
            "User Story",
            "--title",
            "Test item",
            "--field",
            "Microsoft.VSTS.Common.Priority=1",
            "--dry-run",
            "--org",
            "nrgmr",
            "--project",
            "Yellow Hat",
            "--detect",
            "false",
            "--json",
        ])
        .env("ADO_PAT", "fake")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout is JSON");
    assert_eq!(v["operation"], "work-item.create");
    assert_eq!(v["dryRun"], true);
    assert_eq!(v["target"]["organization"], "nrgmr");
    assert_eq!(v["target"]["project"], "Yellow Hat");
    let patch = v["result"]["patch"].as_array().expect("patch ops");
    assert!(patch.iter().any(|op| op["path"] == "/fields/System.Title"));
}

/// Regression: a previous build stored the literal "@file" token instead of
/// the file's content. The dry-run must show the expanded content.
#[test]
fn dry_run_create_expands_at_file_token() {
    let dir = std::env::temp_dir().join(format!("ab-atfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let desc = dir.join("desc.md");
    std::fs::write(&desc, "## Summary\nReal **markdown** content, not a token.").unwrap();

    let assert = ab()
        .args([
            "work-item",
            "create",
            "--type",
            "User Story",
            "--title",
            "T",
            "--description",
            &format!("@{}", desc.display()),
            "-f",
            &format!(
                "Microsoft.VSTS.Common.AcceptanceCriteria=@{}",
                desc.display()
            ),
            "--dry-run",
            "--org",
            "nrgmr",
            "--project",
            "Yellow Hat",
            "--detect",
            "false",
            "--json",
        ])
        .env("ADO_PAT", "fake")
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).expect("json");
    let patch = v["result"]["patch"].as_array().unwrap();
    let desc_op = patch
        .iter()
        .find(|op| op["path"] == "/fields/System.Description")
        .expect("description op");
    assert!(
        desc_op["value"]
            .as_str()
            .unwrap()
            .contains("Real **markdown**"),
        "expected expanded file content, got {desc_op:?}"
    );
    // -f value-level @file expands too.
    let ac_op = patch
        .iter()
        .find(|op| op["path"] == "/fields/Microsoft.VSTS.Common.AcceptanceCriteria")
        .expect("ac op");
    assert!(ac_op["value"]
        .as_str()
        .unwrap()
        .contains("Real **markdown**"));
    // And never the literal token.
    assert!(!serde_json::to_string(&v).unwrap().contains("@/"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn missing_at_file_is_validation_exit_7() {
    ab().args([
        "work-item",
        "create",
        "--type",
        "Task",
        "--title",
        "T",
        "--description",
        "@/no/such/file-xyz.md",
        "--dry-run",
        "--org",
        "o",
        "--project",
        "p",
        "--detect",
        "false",
        "--json",
    ])
    .env("ADO_PAT", "fake")
    .assert()
    .code(7);
}

#[test]
fn from_json_dry_run_builds_patch_inline() {
    let assert = ab()
        .args([
            "work-item",
            "create",
            "--from-json",
            r#"{"type":"Task","fields":{"System.Title":"From JSON","Microsoft.VSTS.Common.Priority":2},"parent":100}"#,
            "--dry-run",
            "--org",
            "nrgmr",
            "--project",
            "Yellow Hat",
            "--detect",
            "false",
            "--json",
        ])
        .env("ADO_PAT", "fake")
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["result"]["type"], "Task");
    let patch = v["result"]["patch"].as_array().unwrap();
    assert!(patch
        .iter()
        .any(|op| op["path"] == "/fields/System.Title" && op["value"] == "From JSON"));
    // Number stays typed.
    assert!(patch
        .iter()
        .any(|op| op["path"] == "/fields/Microsoft.VSTS.Common.Priority" && op["value"] == 2));
    // parent => relation.
    assert!(patch.iter().any(|op| op["path"] == "/relations/-"));
}

#[test]
fn from_json_conflicts_with_field_flags_exit_7() {
    ab().args([
        "work-item",
        "create",
        "--from-json",
        r#"{"type":"Task","fields":{"System.Title":"X"}}"#,
        "--title",
        "also a title",
        "--dry-run",
        "--org",
        "o",
        "--project",
        "p",
        "--detect",
        "false",
        "--json",
    ])
    .env("ADO_PAT", "fake")
    .assert()
    .code(7);
}

#[test]
fn create_missing_type_is_validation_exit_7() {
    // --type now optional at the clap layer; enforced at runtime.
    ab().args([
        "work-item",
        "create",
        "--title",
        "no type",
        "--dry-run",
        "--org",
        "o",
        "--project",
        "p",
        "--detect",
        "false",
        "--json",
    ])
    .env("ADO_PAT", "fake")
    .assert()
    .code(7);
}

/// #2: `--query id` on a mutation resolves the work item id (hoisted to the
/// envelope top level), instead of silently returning null.
#[test]
fn mutation_query_id_resolves_on_dry_run() {
    let assert = ab()
        .args([
            "work-item",
            "update",
            "55",
            "--state",
            "Active",
            "--dry-run",
            "--org",
            "o",
            "--project",
            "p",
            "--detect",
            "false",
            "--json",
            "--query",
            "id",
        ])
        .env("ADO_PAT", "fake")
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert_eq!(stdout.trim(), "55");
}

/// #3: `work-item update` accepts --description / --acceptance-criteria (with
/// @file expansion), not only -f.
#[test]
fn update_accepts_description_sugar_and_at_file() {
    let dir = std::env::temp_dir().join(format!("ab-upd-desc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("d.md");
    std::fs::write(&f, "New body from file").unwrap();
    let assert = ab()
        .args([
            "work-item",
            "update",
            "55",
            "--description",
            &format!("@{}", f.display()),
            "--dry-run",
            "--org",
            "o",
            "--project",
            "p",
            "--detect",
            "false",
            "--json",
        ])
        .env("ADO_PAT", "fake")
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let ops = v["result"].as_array().unwrap();
    assert!(ops.iter().any(
        |op| op["path"] == "/fields/System.Description" && op["value"] == "New body from file"
    ));
    std::fs::remove_dir_all(&dir).ok();
}

/// #5: --markdown converts description/acceptance-criteria to HTML.
#[test]
fn markdown_flag_converts_description_to_html() {
    let assert = ab()
        .args([
            "work-item",
            "create",
            "--type",
            "Bug",
            "--title",
            "T",
            "--description",
            "## Summary\n\nA **bold** point and `code`.",
            "--markdown",
            "--dry-run",
            "--org",
            "o",
            "--project",
            "p",
            "--detect",
            "false",
            "--json",
        ])
        .env("ADO_PAT", "fake")
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let desc = v["result"]["patch"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["path"] == "/fields/System.Description")
        .unwrap()["value"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(desc.contains("<h2>Summary</h2>"), "{desc}");
    assert!(desc.contains("<strong>bold</strong>"), "{desc}");
    assert!(desc.contains("<code>code</code>"), "{desc}");
}

#[test]
fn jmespath_query_filters_output() {
    let assert = ab()
        .args([
            "context",
            "detect",
            "--explain",
            "--org",
            "nrgmr",
            "--detect",
            "false",
            "--json",
            "--query",
            "organization[?selected].source | [0]",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    assert_eq!(stdout.trim(), "\"command-line flag\"");
}

#[test]
fn invalid_jmespath_is_validation_exit_7() {
    ab().args([
        "context", "show", "--json", "--query", "value[?", "--detect", "false",
    ])
    .assert()
    .code(7);
}

#[test]
fn output_formats_render() {
    for format in ["json", "jsonc", "yaml", "tsv", "table", "none"] {
        let assert = ab()
            .args(["context", "show", "--detect", "false", "-o", format])
            .assert()
            .success();
        if format == "none" {
            assert!(assert.get_output().stdout.is_empty());
        }
    }
}

#[test]
fn completions_generate_for_all_shells() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let assert = ab().args(["completion", shell]).assert().success();
        assert!(
            !assert.get_output().stdout.is_empty(),
            "{shell} completions empty"
        );
    }
}

#[test]
fn non_services_org_rejected_exit_7() {
    ab().args([
        "work-item",
        "show",
        "1",
        "--org",
        "https://tfs.corp.local/x",
        "--project",
        "p",
        "--detect",
        "false",
    ])
    .env("ADO_PAT", "fake")
    .assert()
    .code(7);
}

#[test]
fn configure_and_context_roundtrip() {
    let tmp = std::env::temp_dir().join(format!("ab-cfg-{}", std::process::id()));
    let cfg = tmp.join("config");
    std::fs::create_dir_all(&cfg).ok();
    // Write to an isolated config dir on every platform (Windows ignores XDG).
    let mut set = Command::cargo_bin("azure-boards").unwrap();
    set.env("AZURE_BOARDS_CONFIG_DIR", &cfg)
        .env("NO_COLOR", "1")
        .args([
            "configure",
            "--defaults",
            "organization=nrgmr",
            "project=Yellow Hat",
        ])
        .assert()
        .success();
    let mut show = Command::cargo_bin("azure-boards").unwrap();
    let assert = show
        .env_remove("ADO_ORG")
        .env_remove("ADO_PROJECT")
        .env("AZURE_BOARDS_CONFIG_DIR", &cfg)
        .env("NO_COLOR", "1")
        .args(["context", "show", "--detect", "false", "--json"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["organization"], "nrgmr");
    assert_eq!(v["project"], "Yellow Hat");
    assert_eq!(v["projectSource"], "user config");
    std::fs::remove_dir_all(&tmp).ok();
}
