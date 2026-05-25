use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
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

#[cfg(unix)]
#[test]
fn sync_apply_ignores_stale_claude_helper_processes() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    let desktop_dir = temp.path().join("Library/Application Support/Claude");
    let relink_dir = temp.path().join(".claude-relink");
    seed_sync_apply_fixture(&claude_dir, &desktop_dir);
    let path = fake_pgrep_path(
        temp.path(),
        r#"#!/bin/sh
if [ "$1" = "-x" ] && [ "$2" = "Claude" ]; then
  exit 1
fi
if [ "$1" = "-f" ] && [ "$2" = "Claude.app" ]; then
  printf '%s\n' '27360 /Applications/Claude.app/Contents/Helpers/chrome-native-host chrome-extension://example/'
  printf '%s\n' '80604 /Applications/Claude.app/Contents/Frameworks/Electron Framework.framework/Helpers/chrome_crashpad_handler --database=/Users/demo/Library/Application Support/Claude/Crashpad'
  exit 0
fi
exit 2
"#,
    );

    Command::cargo_bin("claude-relink")
        .unwrap()
        .env("PATH", path)
        .args([
            "sync",
            "--apply",
            "--claude-dir",
            claude_dir.to_str().unwrap(),
            "--desktop-dir",
            desktop_dir.to_str().unwrap(),
            "--relink-dir",
            relink_dir.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created files: 1"));
}

#[cfg(unix)]
#[test]
fn sync_apply_blocks_when_claude_main_process_is_running() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    let desktop_dir = temp.path().join("Library/Application Support/Claude");
    let relink_dir = temp.path().join(".claude-relink");
    seed_sync_apply_fixture(&claude_dir, &desktop_dir);
    let path = fake_pgrep_path(
        temp.path(),
        r#"#!/bin/sh
if [ "$1" = "-x" ] && [ "$2" = "Claude" ]; then
  printf '%s\n' '99496 Claude'
  exit 0
fi
if [ "$1" = "-f" ] && [ "$2" = "Claude.app" ]; then
  exit 1
fi
exit 2
"#,
    );

    Command::cargo_bin("claude-relink")
        .unwrap()
        .env("PATH", path)
        .args([
            "sync",
            "--apply",
            "--claude-dir",
            claude_dir.to_str().unwrap(),
            "--desktop-dir",
            desktop_dir.to_str().unwrap(),
            "--relink-dir",
            relink_dir.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Claude Desktop appears to be running",
        ));
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

#[test]
fn library_inspect_prints_counts_after_sync_preview() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    let desktop_dir = temp.path().join("Library/Application Support/Claude");
    let relink_dir = temp.path().join(".claude-relink");
    seed_sync_apply_fixture(&claude_dir, &desktop_dir);

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
        .success();

    Command::cargo_bin("claude-relink")
        .unwrap()
        .args([
            "library",
            "--relink-dir",
            relink_dir.to_str().unwrap(),
            "inspect",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "Library: {}",
            relink_dir.join("library").display()
        )))
        .stdout(predicate::str::contains("Sessions: 1"))
        .stdout(predicate::str::contains("Projects: 1"))
        .stdout(predicate::str::contains("Source buckets: 1"))
        .stdout(predicate::str::contains("Missing transcript records: 0"))
        .stdout(predicate::str::contains("Last refresh: "));
}

#[test]
fn library_rebuild_recreates_sessions_jsonl_without_changing_desktop_bucket_files() {
    let temp = tempdir().unwrap();
    let claude_dir = temp.path().join(".claude");
    let desktop_dir = temp.path().join("Library/Application Support/Claude");
    let relink_dir = temp.path().join(".claude-relink");
    let current_bucket = seed_sync_apply_fixture(&claude_dir, &desktop_dir);
    let old_bucket = desktop_dir.join("claude-code-sessions/old/org");

    let before_current_count = count_local_indexes(&current_bucket);
    let before_old_count = count_local_indexes(&old_bucket);
    let old_index = old_bucket.join("local_old_a.json");
    let old_index_before = fs::read_to_string(&old_index).unwrap();
    fs::create_dir_all(relink_dir.join("library")).unwrap();
    fs::write(relink_dir.join("library/sessions.jsonl"), "stale\n").unwrap();

    Command::cargo_bin("claude-relink")
        .unwrap()
        .args([
            "library",
            "--claude-dir",
            claude_dir.to_str().unwrap(),
            "--desktop-dir",
            desktop_dir.to_str().unwrap(),
            "--relink-dir",
            relink_dir.to_str().unwrap(),
            "rebuild",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Library rebuilt. Sessions: 1"));

    let sessions_path = relink_dir.join("library/sessions.jsonl");
    let sessions_text = fs::read_to_string(sessions_path).unwrap();
    assert!(sessions_text.contains("\"cliSessionId\":\"a\""));
    assert!(!sessions_text.contains("stale"));
    assert_eq!(count_local_indexes(&current_bucket), before_current_count);
    assert_eq!(count_local_indexes(&old_bucket), before_old_count);
    assert_eq!(fs::read_to_string(old_index).unwrap(), old_index_before);
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

fn count_local_indexes(bucket: &Path) -> usize {
    fs::read_dir(bucket)
        .unwrap()
        .filter(|entry| {
            let name = entry.as_ref().unwrap().file_name();
            let name = name.to_string_lossy();
            name.starts_with("local_") && name.ends_with(".json")
        })
        .count()
}

#[cfg(unix)]
fn fake_pgrep_path(temp: &Path, script: &str) -> String {
    let bin_dir = temp.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let pgrep = bin_dir.join("pgrep");
    fs::write(&pgrep, script).unwrap();
    let mut permissions = fs::metadata(&pgrep).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&pgrep, permissions).unwrap();

    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    format!("{}:{}", bin_dir.display(), existing_path.to_string_lossy())
}
