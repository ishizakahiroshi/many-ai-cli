# Detached Session Grid 計画
> 最終更新: 2026-06-13(土) 07:44:37

## context配分

| context | 状態 | 担当範囲 | 主な対象 | 完了条件 |
|---|---|---|---|---|
| C1 | plan | 別窓 Session Grid の基盤を追加 | `web/src/index.html`, `web/src/app/detached-grid.ts`, `web/src/app/multi-pane.ts`, `web/src/app/state.ts`, `web/src/styles/*.css` | 既存 session id 群を指定して、Hub 本体とは別ウィンドウの grid view に表示できる |
| C2 | plan | 新規セッション起動時の Detached 設定 UI を追加 | `web/src/app/spawn-panel.ts`, `web/src/index.html`, `web/src/i18n/*.json`, `web/src/styles/spawn.css` | `＋ 新しいセッション` パネルで Open target / Grid layout / Detached launch plan を選び、起動して別窓で開ける |
| C3 | plan | Hub 本体から既存 AI セッションを別窓へ切り出す導線を追加 | `web/src/app/session-list.ts`, `web/src/app/settings.ts`, `web/src/app/multi-pane.ts`, `web/src/i18n/*.json` | session card / Multi / project group から選択 session を Detached Grid で開ける |
| C4 | plan | Shell を通常 session type として追加 | `internal/hub/spawn_handler.go`, `internal/wrapper/wrapper.go`, `internal/wrapper/pty_*.go`, `cmd/any-ai-cli/main.go`, `web/src/app/spawn-panel.ts` | `/api/spawn` から `provider:"shell"` を受け付け、Hub 内と Detached Grid の両方で通常 shell を表示・操作できる |
| C5 | plan | Detached Grid のプリセットと枚数指定を追加 | `internal/hub/spawn_handler.go`, `web/src/app/detached-grid-launcher.ts`, `web/src/app/user-prefs.ts`, `web/src/styles/*.css` | `Claude + Shell 2x2`, `Shell 2x2`, `Shell 3x3`, `Selected sessions`, `Project sessions` などのプリセットで別窓 grid を開ける |
| C6 | plan | AI / Shell 混在時の除外条件と UX を整える | `internal/hub/approval_handler.go`, `internal/hub/server.go`, `web/src/app/approval.ts`, `web/src/app/chat-history.ts`, `web/src/app/session-list.ts` | Shell では AI 固有機能が動かず、AI session では承認・Chat・Files/Git 連携が従来通り動く |
| C7 | plan | 検証・ドキュメント更新 | `docs/v0.2.0-any-ai-cli-design.md`, `README.md`, `README.ja.md`, 関連テスト | Windows で AI session と Shell session を別窓 grid に表示できることを確認し、公開説明とテストを更新する |

実行順序: C1 → C2 → C3 → C4 → C5 → C6 → C7

## 目的

ANY-AI-CLI Hub 本体を「管制塔」として残しつつ、AI エージェントや Shell の session 表示面だけを別ブラウザウィンドウに切り出せるようにする。

現在の運用では、左に ANY-AI-CLI Hub、右に Windows Terminal の 4 pane を並べている。この計画では右側を Shell 専用画面にするのではなく、Claude / Codex / Copilot / Cursor Agent / Shell を同じ session grid に載せられる「Detached Session Grid」として設計する。

## 方針

- 別窓は新しい session 管理を持たない。session の本体・状態・承認・終了・ログは Hub が持つ。
- Detached Grid は session を「表示するだけ」の view とする。
- 既存 Multi tab の xterm attach / resize / focus / slot 管理をできるだけ再利用する。
- Shell は新しい provider/session type として追加するが、Detached Grid 自体は Shell 専用にしない。
- 新規 session 起動時に `Open target` を選べるようにする。通常起動は Hub tab、別窓運用は Detached window、既存 grid へ追加する場合は Attach to current grid。
- 初期プリセットは Claude + Shell 2x2 / Shell 2x2 / Shell 3x3 を優先しつつ、既存 AI session の selected/project grid も同じ設計で扱う。
- Hub 本体は従来通り Terminal / Chat / Split / Multi / Files / Git を持つ。Detached Grid は補助的な表示モードとする。

## 用語

| 用語 | 意味 |
|---|---|
| Hub 本体 | 通常の ANY-AI-CLI UI。承認、session list、Files/Git/Chat、設定を持つ |
| Detached Grid | Hub 本体とは別ウィンドウで開く session grid view |
| grid session | Detached Grid に表示する既存 session。AI provider でも Shell でもよい |
| preset | Detached Grid を開くための起動設定。layout、count、cwd、session selection など |

## C1 詳細: 別窓 Session Grid の基盤

### 実装案

- SPA 内 view として `/?view=detached-grid&session_ids=1,2,3,4&layout=2x2&token=...` を追加する。
- 別 HTML entry を増やすより、初期段階では既存 bundle を使い、起動時に detached mode を判定して通常 sidebar / input panel / settings を隠す。
- 新規 `detached-grid.ts` を作成し、以下を担当する。
  - URL query の parse
  - layout 計算
  - session id list の固定
  - xterm mount / unmount
  - active pane focus
  - pane resize
- 既存 `MultiPaneManager` をそのまま使えるなら流用する。通常 Hub UI への依存が強い場合は、共通化できる部分だけ抽出し、Detached 専用 manager を作る。

### URL 案

```text
/?view=detached-grid&layout=2x2&session_ids=5,6,9,10&token=<token>
/?view=detached-grid&layout=3x3&preset=shell&count=9&cwd=<encoded>&token=<token>
```

### 完了条件

- 既存の running AI session を session id 指定で別ウィンドウ表示できる。
- 別ウィンドウ側で pane focus と入力が正しい session に届く。
- 別ウィンドウを閉じても session は Hub 側に残る。
- Hub 本体側の session list / approval / Files/Git は従来通り使える。

## C2 詳細: 新規セッション起動時の Detached 設定 UI

### 目的

Detached Grid の使い勝手は、別窓そのものより「セッションを開く時にどう指定するか」で決まる。既存の `＋ 新しいセッション` パネルに、別窓で開くための最小設定を追加する。

### UI 案

既存の Provider / Directory / Label / Model / provider-specific options は維持する。その下に Detached 関連の設定を追加する。

| 項目 | 候補 | 説明 |
|---|---|---|
| Open target | `Hub tab`, `Detached window`, `Attach to current grid` | セッション起動後にどこへ表示するか |
| Grid layout | `1x1`, `1x2`, `2x2`, `2x3`, `3x3` | Detached window / current grid の配置 |
| Preset | `Claude + Shell 2x2`, `Single large pane`, `Project sessions`, `Current Multi layout` | 起動・表示の組み合わせ |
| Detached launch plan | preview only | 起動時に作られる session と grid 表示の流れを表示 |

### 起動フロー案

`Claude + Shell 2x2` を選んだ場合:

1. 選択 provider の AI session を通常 spawn する。
2. 同じ cwd で Shell session を必要枚数 spawn する。
3. 返ってきた session id 群で Detached Grid URL を生成する。
4. `window.open()` で別窓を開く。
5. Hub 本体側にも session は通常通り表示する。

`Detached window` で単独 AI session を選んだ場合:

1. AI session を spawn する。
2. session id が登録されたら `layout=1x1` の Detached Grid を開く。
3. Hub 本体は管制塔として残す。

### 完了条件

- 新規セッションパネルに Open target / Grid layout / Preset / launch plan preview が表示される。
- `Hub tab` 選択時は従来と同じ起動になる。
- `Detached window` 選択時は起動後に別窓 grid が開く。
- `Attach to current grid` 選択時は既存 Detached Grid がある場合にそこへ pane 追加できる。初期実装で current grid 検出が難しい場合は disabled 表示でもよい。
- 既存 provider-specific options と risk confirmation の挙動を壊さない。

## C3 詳細: 既存 AI セッションを別窓へ切り出す導線

### 導線案

- session card context menu:
  - `Open in detached grid`
  - `Open project in detached grid`
- Multi tab toolbar:
  - `Detach current grid`
- project group header:
  - `Open running sessions in grid`
- selected/favorite sessions:
  - `Open selected in grid`

### UX 方針

- Detached Grid は session を所有しないため、閉じても session は終了しない。
- pane の close は「grid から外す」と「session を終了する」を明確に分ける。
- Hub 本体に戻れる `Open Hub` ボタンを toolbar に置く。
- approval は Hub 本体で処理する。ただし Detached Grid 側にも最小限の waiting badge は表示する。

## C4 詳細: Shell を通常 session type として追加

### 現状

`/api/spawn` は `claude` / `codex` / `copilot` / `cursor-agent` のみを許可している。起動時は `any-ai-cli wrap <provider>` を spawn し、wrapper 側で provider CLI を `startProcess(provider, args, cwd, cols, rows)` に渡している。

### 実装案

- provider 名は `shell` とする。
- `/api/spawn` の provider whitelist に `shell` を追加する。
- Shell では `model` / `route` / permission 系 flags を使わない。
- Shell では `EnvPresetFor`、last model 保存、Ollama route 判定を通さない。
- `wrapper.Run` の display map に `shell: "Shell"` を追加する。
- `resolveCmd` で `provider == "shell"` の場合だけ OS 既定 shell を解決する。
  - Windows: `pwsh.exe` を優先し、なければ `powershell.exe`、さらに必要なら `cmd.exe`。
  - Unix: `$SHELL` があればそれを使い、なければ `bash`、最後に `sh`。
- Shell の provider args は基本空にする。Shell profile 選択は後続。

### UI 案

- spawn provider に `Shell` を追加する。
- Shell 選択時は model input / provider-specific options を隠す。
- session card と summary に `Shell` provider chip を表示する。
- Multi / Detached Grid の pane badge に Shell アイコンを表示する。

## C5 詳細: Detached Grid のプリセットと枚数指定

### プリセット案

| preset | 内容 |
|---|---|
| `Selected sessions` | 選択中の AI/Shell session id をそのまま別窓 grid に表示 |
| `Project sessions` | 同じ project group の running/waiting session を別窓 grid に表示 |
| `Current Multi layout` | Hub の Multi tab 現在配置を別窓へ切り出す |
| `Claude + Shell 2x2` | 選択 provider の AI session 1枚と Shell session 3枚を起動し 2x2 に表示 |
| `Shell 2x2` | 指定 cwd で Shell session を 4枚起動し 2x2 に表示 |
| `Shell 3x3` | 指定 cwd で Shell session を 9枚起動し 3x3 に表示 |
| `Mixed workspace` | active AI session + Shell 複数枚を同じ grid に表示 |

### API 案

既存 session を開く場合は frontend の URL 生成のみでよい。

Shell を複数起動するプリセットは、UI 側から `/api/spawn` を count 回呼ぶだけでも実現できる。ただし配置安定性とエラー処理のため、必要なら後続で `POST /api/spawn-grid` を追加する。

request:

```json
{
  "preset": "shell",
  "layout": "2x2",
  "count": 4,
  "cwd": "C:\\dev\\github\\public\\any-ai-cli",
  "label_prefix": "shell"
}
```

response:

```json
{
  "ok": true,
  "layout": "2x2",
  "session_ids": [11, 12, 13, 14]
}
```

### 設定保存

- 直近 layout
- 直近 count
- 直近 detached window size は保存しない。OS の window manager に任せる。
- よく使う preset は user prefs に保存できるようにする。

## C6 詳細: AI / Shell 混在時の除外条件と UX

### Shell で無効にするもの

- approval rule injection / cleanup
- Go-side native approval detection
- frontend approval parser
- slash command picker
- `/model` quick command
- usage relay / token statusbar
- AI chat history extraction
- done summary push notification

### AI session で維持するもの

- approval action bar
- Chat / Split / Files / Git
- provider-specific model display
- usage link / token statusbar
- attach handling

### 実装案

- Go 側に `isAIProvider(provider)` または `providerSupportsApproval(provider)` の helper を置く。
- frontend 側に `isShellProvider(provider)` / `isAIProvider(provider)` helper を置く。
- provider 判定の散在を減らし、Shell 追加による approval 誤検出を避ける。
- Detached Grid 側では approval 操作そのものは最小限にし、Hub 本体への誘導を優先する。

## C7 詳細: 検証・ドキュメント更新

### 検証項目

- 新規セッション起動時に `Detached window` を選んで Claude / Codex session を別窓表示できる。
- 既存 Claude / Codex session を Detached Grid に表示できる。
- Detached Grid から AI session に入力できる。
- Detached Grid で AI session の resize が崩れない。
- Shell session を 1枚起動し、Hub 本体と Detached Grid の両方で表示できる。
- Shell 2x2 / 3x3 preset が指定 cwd で起動する。
- Shell と AI を混在表示できる。
- 別窓を閉じても session は終了しない。
- session close / Hub shutdown / reconnect grace が既存仕様と矛盾しない。

### テスト候補

- `internal/hub/spawn_handler.go` の provider validation と Shell 起動 body の単体テスト。
- `internal/wrapper/pty_*` の Shell command resolution テスト。OS 差分は build tag で分ける。
- Detached URL parse / layout parse の frontend test。
- `bun run check`。
- Go 側変更後は `go test ./...`。

### ドキュメント更新

- `docs/v0.2.0-any-ai-cli-design.md` に Detached Session Grid、新規セッション起動 UI、Shell provider を追記する。
- `README.md` / `README.ja.md` の Hub UI / Multi 説明に、別窓 grid と通常 shell session を追記する。

## 判断ログ

- Shell 専用の別窓ではなく、AI agent session も載せられる汎用 Detached Session Grid として設計する。
- Shell は grid の専用機能ではなく、Hub 管理下の通常 session type として追加する。
- 別窓は session を所有しない。表示と一括起動だけを担当する。
- 起動時の選択 UI が重要なため、既存 `＋ 新しいセッション` パネル内に Open target / Grid layout / Detached launch plan を追加する。
- 初期実装は既存 Multi の仕組みを再利用し、過度な window manager 機能は入れない。

## 関連モック

- `docs/shell-grid-mockup.html`: 初期の Shell Grid 見た目モック。実装時は Shell 専用ではなく Detached Session Grid へ読み替える。
