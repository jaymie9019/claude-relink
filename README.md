# claude-relink

`claude-relink` syncs local Claude Code session visibility into the currently logged-in Claude Desktop account.

It keeps an account-neutral local session library at `~/.claude-relink/library` and uses that library to create missing Claude Desktop `local_*.json` visibility records for the current account.

It does not recover deleted transcripts, cloud attachments, or account-restricted remote resources. It does not modify `~/.claude/projects/**/*.jsonl`. Always quit Claude Desktop before applying a sync or restoring a backup.

## Basic Workflow

```bash
claude-relink sync
# quit Claude Desktop
claude-relink sync --apply
claude-relink restore --latest
```

`sync` is read-only by default. It refreshes `~/.claude-relink/library` and previews what the current Claude Desktop account is missing. `sync --apply` writes only missing index records into the current Claude Desktop account bucket, after creating a backup under `~/.claude-relink/backups`.

## Inspect and Rebuild

```bash
claude-relink library inspect
claude-relink library rebuild
```

`library inspect` prints library counts, source bucket counts, and missing transcript records. `library rebuild` deletes and recreates `~/.claude-relink/library/sessions.jsonl` from local Claude Desktop indexes and Claude Code transcripts. It does not write Claude Desktop bucket files.

## Filters

```bash
claude-relink sync --project /Users/jaymie/projects/grokx
claude-relink sync --from-account <old-account-id> --from-org <old-org-id>
```

`--project` limits the preview or apply to sessions whose `cwd` or `originCwd` matches that project. `--from-account` and `--from-org` filter which source account bucket should be used for the sync plan, while the neutral library is still refreshed from all local sources.

## Path Overrides

```bash
claude-relink sync \
  --claude-dir ~/.claude \
  --desktop-dir "$HOME/Library/Application Support/Claude" \
  --relink-dir ~/.claude-relink

claude-relink restore --backup ~/.claude-relink/backups/<timestamp>
```

Use `--account-id` and `--org-id` only when Claude Desktop has multiple possible target buckets and the current one cannot be inferred.
