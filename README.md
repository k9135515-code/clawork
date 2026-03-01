# Clawork

Clawork は、Rust + Tauri で構築したローカル優先のデスクトップ AI エージェントです。  
Cowork 系の安全制御と OpenClaw 系の常駐自動化を両立しつつ、Operator セッションで「計画 → 承認 → 実行」を追跡できます。

## 主な機能

- Sandbox/Elevated の二段階権限（承認トークン付き）
- ローカル制御 API（`X-Clawork-Token` 必須）
- CLI / GUI / Local API の共通ロジック
- WASM スキル実行（WASI + JSON stdin/stdout）
- MCP ツール呼び出し（stdio セッション）
- メモリ保存・検索（埋め込み + LIKE フォールバック）
- Daily Briefing / proactive suggestions
- Telegram / WhatsApp 送受信経路（Webhook + polling）
- Operator Cockpit（セッション、タイムライン、承認キュー、定期タスク）
- Domain Policy（ドメイン単位の action 制御）
- Connector OAuth 状態管理（Workspace Pack の土台）

## リポジトリ構成

- `apps/desktop/src-tauri`: デスクトップ backend（Tauri + Local API）
- `apps/desktop/ui`: Svelte 5 ダッシュボード
- `bin/clawork-cli`: CLI クライアント
- `crates/clawork-core`: 共通型/trait
- `crates/clawork-capabilities`: 権限エンジン
- `crates/clawork-daemon`: heartbeat / cron
- `crates/clawork-skills`: WASM スキルランタイム
- `crates/clawork-operator`: Operator DB / セッション / 承認 / タスク / policy / connector
- `crates/clawork-*`: adapters, memory, audit, mcp, llm, fs, browser, office

## Local API

デフォルト: `http://127.0.0.1:4747`  
必須ヘッダ:

```text
X-Clawork-Token: <data/cli.token の値>
```

### 既存 API（抜粋）

- `GET /v1/status`
- `GET /v1/tasks`
- `POST /v1/actions/approve`
- `POST /v1/auth/token/issue`
- `POST /v1/auth/token/revoke`
- `POST /v1/auth/token/rotate`
- `POST /v1/skills/run`
- `POST /v1/messages/send`
- `GET /v1/messages/inbound`
- `GET /v1/mail/inbox/unreplied`
- `POST /v1/research/jobs`
- `GET /v1/research/jobs/{id}`
- `GET /v1/research/jobs/{id}/report`
- `POST /v1/media/image`
- `POST /v1/media/video`
- `POST /v1/fs/operate`
- `POST /v1/mcp/call`
- `POST /v1/browser/navigate`
- `POST /v1/office/excel`
- `POST /v1/office/upload`
- `POST /v1/memory/store`
- `GET /v1/memory/recent`
- `POST /v1/memory/search`
- `GET /v1/briefing`
- `GET /v1/suggestions`

`/v1/status` には `init_errors`（`audit` / `memory` / `operator` などの初期化失敗理由）が含まれます。

### Operator / Policy / Connector API

- `POST /v1/operator/sessions`
- `GET /v1/operator/sessions`
- `GET /v1/operator/sessions/{id}`
- `POST /v1/operator/sessions/{id}/plan`
- `POST /v1/operator/sessions/{id}/run`
- `GET /v1/operator/sessions/{id}/timeline`
- `GET /v1/operator/approvals/pending`
- `POST /v1/operator/actions/{id}/approve`
- `POST /v1/operator/actions/{id}/reject`
- `POST /v1/operator/tasks`
- `GET /v1/operator/tasks`
- `POST /v1/operator/tasks/{id}/run-now`
- `POST /v1/operator/projects`
- `GET /v1/operator/projects`
- `GET /v1/operator/projects/{id}/artifacts`
- `POST /v1/connectors/{provider}/oauth/start`
- `POST /v1/connectors/{provider}/oauth/callback`
- `GET /v1/connectors/status`
- `POST /v1/connectors/google/sheets/append`
- `POST /v1/connectors/google/drive/create`
- `POST /v1/connectors/notion/search`
- `POST /v1/connectors/notion/page/create`
- `POST /v1/connectors/slack/post`
- `POST /v1/connectors/slack/history`
- `POST /v1/policies/domain`
- `GET /v1/policies/domain`

## CLI 例

```bash
clawork status
clawork ask "statusを表示して"
clawork ask "fs write data/note.txt :: 今日のメモを保存して"
clawork ask "briefingを見せて"
clawork ask "search rust tauri mcp"
clawork ask "browse openai.com"
clawork ask "workspaceの接続状況を確認して"
clawork ask "artifact 今週のメモ :: - done: ci stabilized"
clawork ask "llm このプロジェクトの次の改善点を3つ提案して"
clawork ask "send telegram 123456 :: 進捗を送って" --dry-run
clawork ops health
clawork approve confirm-xxxx
clawork token issue --ttl-seconds 3600
clawork token revoke <token>
clawork token rotate

clawork skill list
clawork skill run echo '{"text":"hello"}'

clawork operator session create "Workspace follow-up" "Summarize pending work"
clawork operator session list
clawork operator session plan <session-id> '["Collect context","Draft actions","Wait approval"]'
clawork operator session run <session-id>
clawork operator session timeline <session-id>

clawork operator approvals list
clawork operator approvals approve <action-id>

clawork operator task create "Morning check" "0 */30 * * * * *" "Generate short briefing"
clawork operator task list
clawork operator project create "RAG Validation" --description "chunking + retrieval quality"
clawork operator project list
clawork operator project artifacts <project-id>

clawork connectors status
clawork connectors auth google
clawork connectors google-sheets-append <spreadsheet_id> '[[\"url\",\"score\",\"cost\"]]' --sheet-name Sheet1 --value-input-option USER_ENTERED
clawork connectors google-drive-create "weekly-report.txt" "hello from clawork" --mime-type text/plain
clawork connectors auth notion
clawork connectors notion-search "RAG"
clawork connectors notion-page-create "Weekly Notes" --content "decisions and follow-ups"
clawork connectors auth slack
clawork connectors slack-post C123456 "daily standup summary"
clawork connectors slack-history C123456 --limit 20
clawork mail unreplied 10
clawork research create "Compare RAG chunking strategies" '[\"https://example.com/a\",\"https://example.com/b\"]' --title "RAG chunking review" --project-id <project-id>
clawork research show <job-id>
clawork research report <job-id>
clawork media image "cinematic product photo, soft light" --provider openai --model gpt-image-1 --size 1024x1024 --project-id <project-id>
clawork media video "camera dolly through modern office" --provider sora --seconds 12 --project-id <project-id>
clawork mcp connector.notion.search '{"query":"RAG","page_size":10}' --route local
clawork mcp connector.slack.post '{"channel":"C123456","text":"from mcp local route"}' --route local
clawork mcp tools/list '{}' --route remote

clawork policy domain set example.com low "network_call,message_send" ""
clawork policy domain list
```

`research create` と `media image/video` に `--project-id` を指定すると、生成物が `project_artifacts` に citation 付きで自動登録されます。
`mcp` は `--route local|remote` を指定できます。`remote` は `CLAWORK_REMOTE_MCP_URL` が必要です。
`ask` は自然言語から実行計画を作って操作します。`CLAWORK_OPENAI_API_KEY` があれば LLM 計画、未設定時はルールベース計画にフォールバックします。
`mcp --route remote` のレスポンス契約は固定です:

```json
{
  "ok": true,
  "payload": { "...": "..." },
  "error": null
}
```

この形以外は `validation_error` になります。
`token rotate` は既存トークンを失効し、新しいトークンを `data/cli.token` に保存します。`token issue` は期限付きトークン発行（デフォルト1時間）です。

## 環境変数（主要）

- `CLAWORK_LOCAL_API_ADDR`（例: `127.0.0.1:4747`）
- `CLAWORK_CLI_TOKEN` / `CLAWORK_CLI_TOKEN_FILE`
- `CLAWORK_ALLOWED_PATHS`
- `CLAWORK_MCP_COMMAND`, `CLAWORK_MCP_ARGS`
- `CLAWORK_REMOTE_MCP_URL`（`clawork mcp --route remote` 用）
- `CLAWORK_OPENAI_API_KEY`, `CLAWORK_OPENAI_EMBEDDING_MODEL`
- `CLAWORK_TELEGRAM_BOT_TOKEN`
- `CLAWORK_TELEGRAM_WEBHOOK_SECRET`
- `CLAWORK_WHATSAPP_ACCESS_TOKEN`, `CLAWORK_WHATSAPP_PHONE_ID`
- `CLAWORK_WHATSAPP_VERIFY_TOKEN`, `CLAWORK_WHATSAPP_APP_SECRET`
- `CLAWORK_SLACK_WEBHOOK_URL`（Slack 送信）
- `CLAWORK_SLACK_BOT_TOKEN`, `CLAWORK_SLACK_POLL_CHANNELS`（Slack 受信）
- `CLAWORK_GOOGLE_ACCESS_TOKEN`（Google API 直接利用する場合の上書き）
- `CLAWORK_NOTION_ACCESS_TOKEN`, `CLAWORK_NOTION_VERSION`
- `CLAWORK_MEDIA_IMAGE_ENDPOINT`, `CLAWORK_MEDIA_VIDEO_ENDPOINT`
- `CLAWORK_MEDIA_ENDPOINT_<PROVIDER>_IMAGE`, `CLAWORK_MEDIA_ENDPOINT_<PROVIDER>_VIDEO`
- `CLAWORK_MEDIA_API_KEY`, `CLAWORK_MEDIA_API_KEY_<PROVIDER>`
- `CLAWORK_ENABLE_IMESSAGE`（macOS のみ）
- `CLAWORK_INBOUND_AUTORUN`（`1/true` で受信チャット自動実行）
- `CLAWORK_INBOUND_REPLY`（`1/true` でチャットへ実行結果返信）
- `CLAWORK_INBOUND_AUTO_ELEVATE_TTL_SECONDS`（自動実行時に Elevated を有効化する秒数）
- `CLAWORK_OPENAI_MODEL_PLANNING`（自然言語計画用モデル上書き）
- `CLAWORK_OPENAI_MODEL_PROMPT`（`llm_prompt` 用モデル上書き）

### チャット経由で自然言語実行（Telegram/WhatsApp）

1. 受信経路（Webhook または poll）を設定
2. 自動実行を有効化

```powershell
$env:CLAWORK_INBOUND_AUTORUN="1"
$env:CLAWORK_INBOUND_REPLY="1"
# 危険操作も自動実行したい場合のみ（任意）
$env:CLAWORK_INBOUND_AUTO_ELEVATE_TTL_SECONDS="1800"
```

3. チャットで `clawork ` または `/clawork ` プレフィックス付きで送信

例:
- `clawork status`
- `/clawork briefingを見せて`
- `clawork fs write data/note.txt :: これはチャットからのメモ`
- `clawork search rust async runtime`
- `clawork browse github.com`
- `clawork artifact 週次サマリ :: 今週の実施事項`

## インストール（GitHub Releases）

`CLAWORK_REPO` に実リポジトリ（`owner/repo`）を指定して実行します。

Linux/macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/<owner>/<repo>/main/scripts/install.sh | CLAWORK_REPO=<owner/repo> bash
```

Windows (PowerShell):

```powershell
iwr https://raw.githubusercontent.com/<owner>/<repo>/main/scripts/install.ps1 -UseBasicParsing | iex
# 事前に: $env:CLAWORK_REPO="<owner/repo>"
```

installer は release asset の `*.sha256` を検証します。検証を無効化する場合のみ
`CLAWORK_SKIP_CHECKSUM=1` を指定してください（非推奨）。
release には `SHA256SUMS.txt` も添付され、publish 前に workflow で構成整合を検証します。
あわせて `preflight-release-gate.md` / `release-readiness-gate.md` を同梱し、判定根拠を追跡できます。

## 開発

UI:

```bash
cd apps/desktop/ui
npm ci
npm run build
```

Desktop:

```bash
cd apps/desktop/src-tauri
cargo tauri dev
```

最小E2Eスモーク（Windows想定）:

```powershell
./scripts/smoke-local-api.ps1
```

このスモークでは以下を検証します:
- daemon start/stop/restart と status 整合
- 承認フロー付き `fs write/read` の往復
- Daily Briefing 取得と rationale フィールド
- MCP remote 失敗時の `not_configured` エラーコード
- （利用可能時）operator session create/plan/run/timeline
- （利用可能時）memory store/search/recent

プレリリース前チェック（Windows想定）:

```powershell
./scripts/preflight-release.ps1
```

レポートを明示出力する場合:

```powershell
./scripts/preflight-release.ps1 -ReportPath reports/preflight-v1.md
```

CLI サイズ予算（既定 15MB）を変更する場合:

```powershell
./scripts/preflight-release.ps1 -MaxCliSizeMb 12
```

GitHub Actions でも `preflight-report` workflow から同等チェックを実行し、
artifact `preflight-report` として判定レポートを取得できます。

タグ `v*` で走る `release` workflow は、先に `preflight-gate`（Windows）を通過した場合のみ
artifact build / publish に進みます。

ローカルで最終 readiness 判定を作る場合:

```powershell
./scripts/release-readiness.ps1 -PreflightReportPath reports/preflight-latest.md -ReportPath reports/release-readiness-latest.md
```

preflight と readiness を順にまとめて実行する場合:

```powershell
./scripts/run-release-gates.ps1 -SkipReleaseBuild
```

今回のPR-9実行例（ローカル判定レポート）:

```powershell
./scripts/preflight-release.ps1 -SkipReleaseBuild -ReportPath reports/preflight-pr9.md
./scripts/release-readiness.ps1 -PreflightReportPath reports/preflight-pr9.md -ReportPath reports/release-readiness-pr9.md
```

## 注意事項

- Node は `>=22 <23` 前提（Windows の Vite/Rollup 異常終了回避）
- iMessage は macOS 限定、他 OS では既定で非表示
- Connector OAuth は現在「状態管理」の土台実装（最小スコープ）
- Domain Policy は action 判定前に適用（blocked_actions 優先）
- Rust toolchain は `1.88.x` を前提
- `operator` / `memory` のDB初期化に失敗する環境では、smoke script内の該当検証は自動で skip
