# claude-relink

`claude-relink` syncs local Claude Code session visibility into the currently logged-in Claude Desktop account.

It does not recover deleted transcripts, cloud attachments, or account-restricted remote resources. It does not modify `~/.claude/projects/**/*.jsonl`. Always quit Claude Desktop before applying a sync or restoring a backup.

## Basic Workflow

```bash
claude-relink sync
# quit Claude Desktop
claude-relink sync --apply
claude-relink restore --latest
```

`sync` is read-only by default. It refreshes `~/.claude-relink/library` and previews what the current Claude Desktop account is missing.

## Advanced

```bash
claude-relink sync --project /Users/jaymie/projects/grokx
claude-relink sync --from-account <old-account-id> --from-org <old-org-id>
claude-relink library inspect
claude-relink library rebuild
```
