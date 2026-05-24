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

#[test]
fn sync_apply_writes_missing_session_with_backup_manifest() {
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
        current_bucket.join("local_existing.json"),
        r#"{"sessionId":"local_existing"}"#,
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
            "--apply",
            "--force-while-running",
            "--claude-dir",
            claude_dir.to_str().unwrap(),
            "--desktop-dir",
            desktop_dir.to_str().unwrap(),
            "--relink-dir",
            relink_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created files: 1"))
        .stdout(predicate::str::contains("Skipped existing: 0"));

    let mut local_files = fs::read_dir(&current_bucket)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with("local_") && name.ends_with(".json")
        })
        .collect::<Vec<_>>();
    local_files.sort();
    assert_eq!(local_files.len(), 2);

    let created_file = local_files
        .iter()
        .find(|path| path.file_name().unwrap() != "local_existing.json")
        .unwrap();
    let created_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(created_file).unwrap()).unwrap();
    let session_id = created_json["sessionId"].as_str().unwrap();
    assert!(session_id.starts_with("local_"));
    assert_ne!(session_id, "local_old_a");
    assert_eq!(created_json["cliSessionId"], "a");
    assert_eq!(
        created_file.file_name().unwrap().to_string_lossy(),
        format!("{session_id}.json")
    );

    let backups_dir = relink_dir.join("backups");
    let backup_root = fs::read_dir(&backups_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .unwrap();
    assert!(backup_root.join("manifest.json").exists());
    assert!(backup_root.join("current/org/local_existing.json").exists());
    assert!(!backup_root
        .join("current/org")
        .join(created_file.file_name().unwrap())
        .exists());

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(backup_root.join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["operation"], "sync");
    assert_eq!(manifest["targetAccountId"], "current");
    assert_eq!(manifest["targetOrgId"], "org");
    assert_eq!(
        manifest["desktopBucket"].as_str().unwrap(),
        current_bucket.to_string_lossy().as_ref()
    );
    assert_eq!(manifest["createdFiles"].as_array().unwrap().len(), 1);
    assert_eq!(
        manifest["createdFiles"][0].as_str().unwrap(),
        created_file.file_name().unwrap().to_string_lossy().as_ref()
    );
    assert!(manifest["skippedExisting"].as_array().unwrap().is_empty());
}

#[test]
fn restore_latest_replaces_current_bucket_with_backup_contents() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    let desktop_dir = temp.path().join("Library/Application Support/Claude");
    let relink_dir = temp.path().join(".claude-relink");
    let current_bucket = seed_sync_apply_fixture(&claude_dir, &desktop_dir);

    Command::cargo_bin("claude-relink")
        .unwrap()
        .args([
            "sync",
            "--apply",
            "--force-while-running",
            "--claude-dir",
            claude_dir.to_str().unwrap(),
            "--desktop-dir",
            desktop_dir.to_str().unwrap(),
            "--relink-dir",
            relink_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    let created_by_sync = fs::read_dir(&current_bucket)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.file_name().unwrap() != "local_existing.json")
        .unwrap();
    let post_backup_file = current_bucket.join("local_after_backup.json");
    fs::write(
        &post_backup_file,
        r#"{"sessionId":"local_after_backup","cliSessionId":"after-backup"}"#,
    )
    .unwrap();

    Command::cargo_bin("claude-relink")
        .unwrap()
        .args([
            "restore",
            "--latest",
            "--force-while-running",
            "--relink-dir",
            relink_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Restored files: 1"));

    assert!(current_bucket.join("local_existing.json").exists());
    assert!(!created_by_sync.exists());
    assert!(!post_backup_file.exists());
}

#[test]
fn restore_backup_replaces_current_bucket_from_explicit_backup_path() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    let desktop_dir = temp.path().join("Library/Application Support/Claude");
    let relink_dir = temp.path().join(".claude-relink");
    let current_bucket = seed_sync_apply_fixture(&claude_dir, &desktop_dir);

    Command::cargo_bin("claude-relink")
        .unwrap()
        .args([
            "sync",
            "--apply",
            "--force-while-running",
            "--claude-dir",
            claude_dir.to_str().unwrap(),
            "--desktop-dir",
            desktop_dir.to_str().unwrap(),
            "--relink-dir",
            relink_dir.to_str().unwrap(),
        ])
        .assert()
        .success();

    let backup_root = fs::read_dir(relink_dir.join("backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .unwrap();
    fs::write(
        current_bucket.join("local_after_backup.json"),
        r#"{"sessionId":"local_after_backup","cliSessionId":"after-backup"}"#,
    )
    .unwrap();

    Command::cargo_bin("claude-relink")
        .unwrap()
        .args([
            "restore",
            "--backup",
            backup_root.to_str().unwrap(),
            "--force-while-running",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Restored files: 1"));

    let mut local_files = fs::read_dir(&current_bucket)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("local_") && name.ends_with(".json"))
        .collect::<Vec<_>>();
    local_files.sort();
    assert_eq!(local_files, vec!["local_existing.json"]);
}

fn seed_sync_apply_fixture(claude_dir: &Path, desktop_dir: &Path) -> std::path::PathBuf {
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
        current_bucket.join("local_existing.json"),
        r#"{"sessionId":"local_existing"}"#,
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

    current_bucket
}
