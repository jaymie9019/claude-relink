# Claude Relink CLI - 账号无关本地库方案

## 1. 背景

Claude Code 的真实对话内容主要保存在本机：

```text
~/.claude/projects/<project-bucket>/<cliSessionId>.jsonl
```

Claude Desktop 展示 Claude Code 项目历史时，并不是直接枚举这些 JSONL transcript，而是读取按账号和组织隔离的本地索引：

```text
~/Library/Application Support/Claude/claude-code-sessions/<accountId>/<organizationId>/local_<desktopSessionId>.json
```

索引文件通过 `cliSessionId` 指向真实 transcript。重新登录 Claude Desktop、切换账号、切换组织、更新 Desktop 或迁移本地数据后，当前账号 bucket 可能没有旧账号下的 `local_*.json`。结果是：

- `~/.claude/projects/.../*.jsonl` 仍然存在。
- `claude --resume` 或 `claude --continue` 仍可能找到旧会话。
- Claude Desktop 当前账号的项目历史列表看不到这些会话。

旧设计以“按项目 diagnose/repair”为主，使用成本偏高。新设计以“账号无关本地库 + 一次 sync”为主：工具维护一份自己的会话索引库，当前 Claude Desktop 账号只是同步目标。

## 2. 产品目标

第一版做一个 Rust CLI：

```text
claude-relink
```

核心能力：

- 建立账号无关的本地会话索引库：`~/.claude-relink/library/`。
- 扫描本机 Claude Desktop 各账号 bucket 和 Claude Code transcript，刷新中立库。
- 自动识别当前 Claude Desktop account/org bucket。
- `sync` 默认只读：刷新中立库并预览当前账号缺失哪些会话。
- `sync --apply`：退出 Claude Desktop 后，把缺失会话索引写入当前账号 bucket。
- 每次写入前备份当前账号 bucket。
- `restore --latest` 或 `restore --backup <path>` 回滚写入。
- 支持高级过滤：按项目、按旧账号来源同步。

## 3. 非目标

V1 不做这些事情：

- 不修改 `~/.claude/projects/.../*.jsonl` transcript 正文。
- 不保存完整对话正文到 `~/.claude-relink/library/`。
- 不让 Claude Desktop 直接读取一个伪造的共享 bucket。
- 不用软链接把多个 Desktop account/org bucket 指向同一个目录。
- 不绕过 Claude 账号权限。
- 不恢复远端附件、云端文件、服务端状态。
- 不实现后台守护同步。
- 不做跨机器同步。
- 不做 GUI。
- 不承诺兼容 Anthropic 后续私有 schema 变更，只做 schema 探测和保守写入。

## 4. 用户心智模型

推荐心智模型：

```text
Claude Code transcript 是真实对话。
~/.claude-relink/library 是本机账号无关会话目录。
Claude Desktop 当前账号 bucket 是展示目标。
```

用户日常只需要记住：

```bash
claude-relink sync
# 退出 Claude Desktop
claude-relink sync --apply
claude-relink restore --latest
```

## 5. 本机数据结构

### 5.1 Transcript bucket

项目路径会被 Claude Code 转换为 bucket 名，例如：

```text
/Users/jaymie/projects/grokx
=> ~/.claude/projects/-Users-jaymie-projects-grokx/
```

目录内文件：

```text
106ff30b-4213-4abb-82d5-c1fa82fdb772.jsonl
68bddfd2-dd55-4941-b46a-ce6001e5594f.jsonl
```

文件名去掉 `.jsonl` 后就是 `cliSessionId`。

### 5.2 Claude Desktop index bucket

macOS 默认路径：

```text
~/Library/Application Support/Claude/claude-code-sessions/<accountId>/<organizationId>/
```

索引文件：

```text
local_f827f428-0d13-42d1-82b9-3b0835323b43.json
```

V1 依赖字段：

- `sessionId`
- `cliSessionId`
- `cwd`
- `originCwd`
- `createdAt`
- `lastActivityAt`
- `lastFocusedAt`
- `title`
- `isArchived`

其他字段优先从源 index 样本继承，避免伪造过多私有 schema。

### 5.3 账号无关本地库

默认目录：

```text
~/.claude-relink/library/
```

建议结构：

```text
~/.claude-relink/
  library/
    sessions.jsonl
    sources.json
    state.json
  backups/
    <timestamp>/
      manifest.json
      <accountId>/<organizationId>/
  logs/
    YYYY-MM-DD.log
```

`sessions.jsonl` 每行是一条中立会话记录：

```json
{
  "cliSessionId": "106ff30b-4213-4abb-82d5-c1fa82fdb772",
  "transcriptPath": "~/.claude/projects/-Users-jaymie-projects-grokx/106ff30b-4213-4abb-82d5-c1fa82fdb772.jsonl",
  "cwd": "/Users/jaymie/projects/grokx",
  "originCwd": "/Users/jaymie/projects/grokx",
  "title": "Grok OAuth proxy CLI",
  "createdAt": 1779465052103,
  "lastActivityAt": 1779538862499,
  "lastFocusedAt": 1779539021793,
  "completedTurns": 30,
  "sourceIndexes": [
    {
      "accountId": "old-account",
      "orgId": "old-org",
      "path": "~/Library/Application Support/Claude/claude-code-sessions/old-account/old-org/local_x.json"
    }
  ],
  "rawIndexTemplate": {
    "model": "claude-opus-4-7",
    "effort": "xhigh",
    "permissionMode": "bypassPermissions",
    "remoteMcpServersConfig": []
  },
  "updatedAt": "2026-05-24T18:00:00Z"
}
```

中立库不保存完整对话，只保存同步所需元数据和来源索引摘要。

## 6. CLI 设计

### 6.1 顶层命令

```bash
claude-relink sync
claude-relink sync --apply
claude-relink sync --project /Users/jaymie/projects/grokx
claude-relink sync --from-account <accountId> --from-org <organizationId>
claude-relink restore --latest
claude-relink restore --backup <backup-dir>
claude-relink library inspect
claude-relink library rebuild
```

### 6.2 sync

默认只读。执行：

1. 扫描 Claude Desktop 所有 account/org bucket。
2. 扫描 `~/.claude/projects/**/*.jsonl` transcript。
3. 刷新 `~/.claude-relink/library/sessions.jsonl`。
4. 识别当前 account/org bucket。
5. 计算当前账号缺失的 `cliSessionId`。
6. 输出同步计划，不写入 Desktop bucket。

示例输出：

```text
Current Desktop bucket:
~/Library/Application Support/Claude/claude-code-sessions/current-account/current-org

Library sessions: 115
Already visible in current account: 1
Missing in current account: 114
Skipped because transcript is missing: 0
Skipped because source index is invalid: 0

Next:
  Quit Claude Desktop
  claude-relink sync --apply
```

### 6.3 sync --apply

执行写入。

前置检查：

- Claude Desktop 是否仍在运行。
- 当前 Desktop bucket 是否存在。
- 当前 bucket 是否可写。
- 中立库是否可读。
- 每条待同步记录的 `transcriptPath` 是否仍存在。
- 当前账号是否已存在同一 `cliSessionId`。

如果 Claude Desktop 正在运行，默认拒绝写入：

```text
Claude Desktop appears to be running.
Quit Claude Desktop fully before applying sync.
Use --force-while-running only if you know what you are doing.
```

写入步骤：

1. 创建备份目录。
2. 拷贝当前 Desktop bucket 到备份目录。
3. 为每条缺失记录生成新的 `sessionId = local_<uuid-v4>`。
4. 继承中立库记录里的 `cwd/title/time/model/permissionMode` 等字段。
5. 设置 `cliSessionId` 为原 transcript ID。
6. 写入临时文件。
7. `fsync` 后原子 rename 为最终 `local_*.json`。
8. 写入 backup manifest。
9. 输出同步报告。

### 6.4 sync --project

只同步某个项目路径对应的会话。`--project` 是过滤条件，不是主流程入口。

```bash
claude-relink sync --project /Users/jaymie/projects/grokx
claude-relink sync --project /Users/jaymie/projects/grokx --apply
```

路径匹配规则：

- 对 `--project` 做 canonicalize。
- 匹配中立库记录里的 `cwd` 或 `originCwd`。
- 不反推时，允许高级参数 `--transcript-bucket <path>`。

### 6.5 sync --from-account / --from-org

当本机存在多个旧账号来源时，用来源过滤：

```bash
claude-relink sync --from-account <old-account-id> --from-org <old-org-id>
```

如果自动发现多个来源且没有明确最佳来源，`sync` 应展示候选并继续预览；`sync --apply` 必须要求用户显式指定来源或确认全部来源。

### 6.6 restore

恢复最近一次或指定备份：

```bash
claude-relink restore --latest
claude-relink restore --backup ~/.claude-relink/backups/2026-05-24T180000Z
```

默认恢复前也要求 Claude Desktop 退出。

### 6.7 library inspect

只读查看中立库摘要：

```text
Library: ~/.claude-relink/library
Sessions: 115
Projects: 8
Source buckets: 2
Missing transcript records: 0
Last refresh: 2026-05-24T18:00:00Z
```

### 6.8 library rebuild

删除并重建中立库。它只影响 `~/.claude-relink/library`，不影响 Claude Desktop bucket 和 transcript。

## 7. 当前账号识别

默认读取：

```text
~/Library/Application Support/Claude/cowork-enabled-cli-ops.json
```

如果存在 `ownerAccountId`：

1. 优先选择该 account 下的 org bucket。
2. 如果该 account 下只有一个 org bucket，直接选择。
3. 如果该 account 下有多个 org bucket，要求用户传：

```bash
--account-id <accountId> --org-id <organizationId>
```

如果没有 owner account：

- `sync` 可列出候选并给出预览。
- `sync --apply` 必须要求显式传当前目标 bucket。

禁止默认写入多个目标 bucket。

## 8. 中立库刷新策略

刷新来源：

1. Claude Desktop 所有 `claude-code-sessions/<account>/<org>/local_*.json`。
2. Claude Code 所有 `~/.claude/projects/**/*.jsonl`。

合并规则：

- 主键：`cliSessionId`。
- 如果多个 source index 指向同一 `cliSessionId`：
  - 保留所有来源到 `sourceIndexes`。
  - `title` 优先选择非空且最近活动时间最新的 source index。
  - `rawIndexTemplate` 优先选择最近活动时间最新的 source index。
- 如果只有 transcript、没有 source index：
  - 可以创建最小记录，但默认标记为 `metadataQuality = "transcript-only"`。
  - 同步时仍可写入，但 dry-run 输出应提示字段质量较低。
- 如果 source index 有 `cliSessionId`，但 transcript 不存在：
  - 保留到诊断计数中，但默认不同步。

## 9. 写回当前账号策略

同步到当前账号时：

- 不复用旧 `sessionId`。
- 新建 `sessionId = local_<uuid-v4>`。
- 文件名为 `<sessionId>.json`。
- `cliSessionId` 保持原值。
- `cwd/originCwd/title/createdAt/lastActivityAt/lastFocusedAt/isArchived` 来自中立库。
- 其他字段从 `rawIndexTemplate` 继承。
- 如果当前账号已有任意 index 的 `cliSessionId` 等于目标记录，则跳过。

最小写入字段：

```json
{
  "sessionId": "local_<uuid-v4>",
  "cliSessionId": "<cliSessionId>",
  "cwd": "<cwd>",
  "originCwd": "<originCwd>",
  "createdAt": 1779465052103,
  "lastActivityAt": 1779538862499,
  "lastFocusedAt": 1779539021793,
  "title": "<title>",
  "titleSource": "auto",
  "isArchived": false
}
```

## 10. 安全策略

### 10.1 默认只读

`sync`、`library inspect` 默认只读。未传 `--apply` 时不写 Desktop bucket。

### 10.2 不冒充 Desktop 内部共享目录

`~/.claude-relink/library` 是工具自己的中立索引库，不是 Claude Desktop 的内部 bucket，也不通过软链接注入 Desktop。

### 10.3 备份优先

每次 `sync --apply` 前备份当前目标 bucket：

```text
~/.claude-relink/backups/<timestamp>/<accountId>/<organizationId>/
```

备份 manifest：

```json
{
  "createdAt": "2026-05-24T18:00:00Z",
  "toolVersion": "0.1.0",
  "operation": "sync",
  "targetAccountId": "current-account",
  "targetOrgId": "current-org",
  "desktopBucket": ".../claude-code-sessions/current-account/current-org",
  "createdFiles": ["local_....json"],
  "skippedExisting": 1
}
```

### 10.4 Claude Desktop 运行检测

macOS 下可通过 `pgrep` 检测：

```text
Claude.app
/Applications/Claude.app/...
```

如果运行中，默认停止 `sync --apply` 和 `restore`。

### 10.5 原子写入

每个 JSON 写入流程：

1. 写到 `.<filename>.tmp`
2. flush
3. fsync
4. rename 到最终文件名

### 10.6 审计日志

每次操作输出 summary，并可选写：

```text
~/.claude-relink/logs/YYYY-MM-DD.log
```

## 11. Rust 技术设计

### 11.1 推荐依赖

```toml
[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
chrono = { version = "0.4", features = ["serde"] }
dirs = "6"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }
walkdir = "2"
fs_extra = "1"
tempfile = "3"
thiserror = "2"

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"
```

### 11.2 模块结构

```text
src/
  main.rs
  lib.rs
  cli.rs
  paths.rs
  desktop_index.rs
  transcript.rs
  library.rs
  sync.rs
  backup.rs
  restore.rs
  process.rs
  report.rs
```

职责：

- `cli.rs`：`clap` 参数定义和命令分发。
- `paths.rs`：默认路径、当前账号解析、Desktop bucket 列表。
- `desktop_index.rs`：读取和生成 `local_*.json`。
- `transcript.rs`：扫描 transcript 路径和基础 metadata。
- `library.rs`：中立库 record、读写、刷新和合并。
- `sync.rs`：计算同步计划和执行 apply。
- `backup.rs`：备份 bucket 和 manifest。
- `restore.rs`：恢复备份。
- `process.rs`：Claude Desktop 运行检测。
- `report.rs`：人类可读输出。

### 11.3 核心数据结构

```rust
struct DesktopBucket {
    account_id: String,
    org_id: String,
    path: PathBuf,
    local_index_count: usize,
}

struct DesktopIndex {
    session_id: String,
    cli_session_id: Option<String>,
    cwd: Option<PathBuf>,
    origin_cwd: Option<PathBuf>,
    path: PathBuf,
    raw: serde_json::Value,
}

struct LibrarySession {
    cli_session_id: String,
    transcript_path: Option<PathBuf>,
    cwd: Option<PathBuf>,
    origin_cwd: Option<PathBuf>,
    title: Option<String>,
    created_at_ms: Option<i64>,
    last_activity_at_ms: Option<i64>,
    last_focused_at_ms: Option<i64>,
    completed_turns: Option<u32>,
    source_indexes: Vec<SourceIndex>,
    raw_index_template: serde_json::Value,
    updated_at: chrono::DateTime<chrono::Utc>,
}

struct SyncPlan {
    target_bucket: DesktopBucket,
    library_sessions: Vec<LibrarySession>,
    already_visible: Vec<DesktopIndex>,
    missing: Vec<LibrarySession>,
    skipped_missing_transcript: Vec<LibrarySession>,
    skipped_invalid_source: Vec<LibrarySession>,
}
```

## 12. 同步算法

```text
sync():
  desktop_buckets = list_desktop_buckets()
  source_indexes = scan all desktop buckets
  transcripts = scan ~/.claude/projects/**/*.jsonl
  library = refresh_library(source_indexes, transcripts)

  target_bucket = resolve_current_desktop_bucket()
  target_indexes = scan_desktop_indexes(target_bucket)
  target_cli_ids = target_indexes.map(cliSessionId)

  missing = library.sessions
    .filter(has transcriptPath that exists)
    .filter(cliSessionId not in target_cli_ids)
    .filter(project/source filters if provided)

  return SyncPlan
```

```text
apply(plan):
  ensure Claude Desktop not running
  backup target bucket

  for session in plan.missing:
    json = build_current_account_index(session)
    atomic_write(target_bucket / (json.sessionId + ".json"), json)

  write manifest
  print summary
```

## 13. 测试计划

### 13.1 单元测试

- 当前 account/org bucket 解析。
- Desktop bucket 列表扫描。
- `local_*.json` 读取缺字段时不 panic。
- transcript 路径扫描。
- 中立库按 `cliSessionId` 合并多个 source index。
- 中立库跳过 transcript 缺失记录。
- sync plan 的 missing/already/skipped 计算。
- 生成写回 JSON 时创建新 `sessionId`，保留 `cliSessionId`。

### 13.2 集成测试

使用 `tempfile` 构造假 home：

```text
tmp/
  .claude/projects/-Users-demo-project/
    a.jsonl
    b.jsonl
  Library/Application Support/Claude/
    cowork-enabled-cli-ops.json
    claude-code-sessions/
      old-account/old-org/local_old_a.json
      current-account/current-org/local_current_b.json
```

验证：

- `sync` 生成中立库并报告 missing = 1。
- `sync` 不写当前账号 bucket。
- `sync --apply` 写入一个新 `local_*.json`。
- 写入文件使用新 `sessionId`，但 `cliSessionId == a`。
- `restore --latest` 恢复 apply 前的当前账号 bucket。
- `library inspect` 输出 sessions/source bucket/project 计数。

### 13.3 手工验收

在真实机器上：

1. 登录 Claude Desktop 当前账号。
2. 执行 `claude-relink sync`。
3. 确认 missing 数量符合预期。
4. 退出 Claude Desktop。
5. 执行 `claude-relink sync --apply`。
6. 重启 Claude Desktop。
7. 打开项目历史，确认旧会话出现。
8. 如异常，执行 `claude-relink restore --latest`。

## 14. 失败处理

常见错误：

- 找不到当前 Desktop bucket：提示打开 Claude Desktop 并登录一次，或传 `--account-id --org-id`。
- 多个当前 org bucket：要求显式传 `--account-id --org-id`。
- 多个旧来源且 apply 不明确：要求传 `--from-account --from-org` 或确认同步全部来源。
- Claude Desktop 正在运行：要求退出。
- transcript 缺失：跳过该 session 并计数。
- JSON 解析失败：跳过坏 index，记录 warning，不中断整体 sync。
- 写入失败：保留备份路径，输出已写入文件列表。

## 15. V0.1 成功标准

- 用户只需要记住 `sync`、`sync --apply`、`restore --latest`。
- `sync` 默认只读，能刷新中立库并预览当前账号缺失会话。
- `sync --apply` 能把旧账号可用会话索引补到当前账号 bucket。
- 不修改任何 `~/.claude/projects/**/*.jsonl`。
- 不把 Claude Desktop 内部目录做成共享 bucket。
- 每次 apply 前都有可恢复备份。
- 公开仓库只包含代码、公开 spec、README 和测试；本地 plan/HTML/research 页面保持 git ignored。
