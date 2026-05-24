use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[test]
fn sync_previews_missing_sessions_without_writing_current_bucket() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    let desktop_dir = temp.path().join("Library/Application Support/Claude");
    let relink_dir = temp.path().join(".claude-relink");
    let project_dir = Path::new("/Users/demo/project");
    let transcript_dir = claude_dir.join("projects/-Users-demo-project");
    let old_bucket = desktop_dir.join("claude-code-sessions/old/org");
    let current_bucket = desktop_dir.join("claude-code-sessions/current/org");

    fs::create_dir_all(&transcript_dir).unwrap();
    fs::create_dir_all(&old_bucket).unwrap();
    fs::create_dir_all(&current_bucket).unwrap();
    fs::write(transcript_dir.join("a.jsonl"), "{}\n").unwrap();
    fs::write(
        desktop_dir.join("cowork-enabled-cli-ops.json"),
        r#"{"ownerAccountId":"current"}"#,
    )
    .unwrap();
    fs::write(
        old_bucket.join("local_old_a.json"),
        format!(
            r#"{{
  "sessionId": "local_old_a",
  "cliSessionId": "a",
  "cwd": "{}",
  "originCwd": "{}",
  "title": "Old A",
  "createdAt": 1000,
  "lastActivityAt": 2000,
  "lastFocusedAt": 3000
}}"#,
            project_dir.display(),
            project_dir.display()
        ),
    )
    .unwrap();

    Command::cargo_bin("claude-relink")
        .unwrap()
        .args([
            "sync",
            "--claude-dir",
            claude_dir.to_str().unwrap(),
            "--desktop-dir",
            desktop_dir.to_str().unwrap(),
            "--relink-dir",
            relink_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Library sessions: 1"))
        .stdout(predicate::str::contains("Missing in current account: 1"));

    let current_local_count = fs::read_dir(&current_bucket)
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            name.starts_with("local_") && name.ends_with(".json")
        })
        .count();
    assert_eq!(current_local_count, 0);
    assert!(relink_dir.join("library/sessions.jsonl").exists());
}
