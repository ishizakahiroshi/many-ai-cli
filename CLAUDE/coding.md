# ai-cli-hub コーディング規約

> 最終更新: 2026-05-07(木) 19:24:03

`ai-cli-hub` は単一 Go バイナリ（Hub 常駐 + ラッパー）+ Vue 3 フロント（`go:embed`）。設計書: [../docs/ai-cli-hub-design-v0.1.0.md](../docs/ai-cli-hub-design-v0.1.0.md)

## 言語別コーディング規約

### Go

- **文字コード:** UTF-8（BOM なし）
- **インデント:** タブ（`gofmt` 準拠）
- **ファイル名:** スネークケース（例: `session.go`, `pty_unix.go`, `pty_windows.go`）
- **識別子:** Go 慣習（パブリック=パスカルケース、プライベート=キャメルケース）
- **OS固有コードは build tag で分離:**
  ```go
  //go:build unix
  // +build unix
  ```
  - ファイル名 suffix（`_unix.go` / `_windows.go` / `_darwin.go` / `_linux.go`）でも分離可
- **パス操作:** `filepath.Join` / `os.UserHomeDir()` / `os.UserConfigDir()` を使う。`/` ハードコード禁止
- **エラー:** `fmt.Errorf("...: %w", err)` で wrap。生 `err.Error()` を文字列結合で返さない
- **ログ:** `log/slog`（Go 1.21+ 標準）。構造化フィールドで出す（`slog.Info("session registered", "session_id", id, "provider", p)`）
- **同時実行:** Hub 内のセッション管理・承認キューは `sync.Mutex` または `chan` で保護。グローバル可変状態は避ける
- **PTY 出力バッファ:** ANSI エスケープを除去してから承認パターンマッチ（生ログは別途保存）

### Vue 3 / TypeScript

- **構成:** `web/` 配下、Vite ビルド、`web/dist/` を `go:embed` で取り込む
- **ファイル名:** コンポーネントはパスカルケース（`ApprovalCard.vue`, `SessionList.vue`）、それ以外はケバブケース or キャメルケース
- **Composition API + `<script setup lang="ts">`** を標準
- **状態管理:** `web/src/stores/hub.ts`（Pinia or 軽量 reactive store）。WebSocket 接続・セッション一覧・承認キューを一元管理
- **スタイル:** Tailwind CSS。スコープが必要なときだけ `<style scoped>`
- **WS メッセージ型:** `proto/messages.go` と TypeScript 型定義を二重管理しない方針（生成 or 手動同期、決まり次第ここに追記）

## ディレクトリ責務

| ディレクトリ | 役割 |
|---|---|
| `cmd/ai-cli-hub/` | サブコマンドディスパッチ（`serve` / `wrap` / `shell-init` / `stop` / `status`）のみ。ロジックは `internal/` |
| `internal/hub/` | HTTP サーバ・WebSocket・セッション管理・承認キュー |
| `internal/wrapper/` | PTY ラップ・出力監視・承認検出・Hub への送信 |
| `internal/shell/` | `shell-init` 出力（bash/zsh/PowerShell 用シェル関数） |
| `internal/proto/` | WebSocket メッセージの Go 構造体定義（`type:"register"` 等） |
| `internal/config/` | YAML 読み込み・デフォルト生成（`~/.ai-cli-hub/config.yaml`） |
| `internal/log/` | 構造化ログ（JSONL）+ PTY 生ログの書き出し |
| `web/` | Vue 3 フロントソース。`web/dist/` がビルド成果物（`go:embed` 対象） |

## 共通実装の使い方

新規実装前に既存共通化を確認すること。以下は実装が進んだら追記する暫定スケルトン。

| リソース | 置き場所 | 用途 |
|---|---|---|
| WS メッセージ型 | `internal/proto/messages.go` | ラッパー ⇄ Hub ⇄ ブラウザ で共有 |
| 設定読み込み | `internal/config/config.go` | デフォルトマージ・YAML パース |
| PTY 抽象 | `internal/wrapper/pty_unix.go` / `pty_windows.go` | OS 差異吸収 |
| 承認検出 | `internal/wrapper/detector.go` | パターンマッチ・デバウンス・ANSI 除去 |
| ログ出力 | `internal/log/log.go` | JSONL + 生 PTY ログ |

## 設計上の制約（実装時に守る）

- **バインドは `127.0.0.1` 固定。`0.0.0.0` / 外部 IP へバインドしない**
- **トークンなしのリクエストは 401 で弾く**（`?token=` または `Authorization: Bearer` どちらか）
- **永続シェル設定（`.bashrc` / `.zshrc` / PowerShell プロファイル）を改変しない**。透過化は `AI_HUB_AUTO=1` + `eval "$(ai-cli-hub shell-init)"` のオプトインのみ
- **CLI プロセスを孤児化しない**（ラッパー終了時は子 CLI にもシグナル伝播）
- **PTY の生バイト列を WS の JSON にそのまま入れない**（base64 エンコード）

## 承認検出の実装方針（detector.go）

- ANSI エスケープを除去してからパターンマッチ
- 同一プロンプトの再描画はデバウンス（直近 500ms は同一として扱う）
- 「カーソルが特定行で停止している」を確定条件に追加
- パターンは `config.yaml` の `providers.<provider>.patterns` で外部化（CLI バージョンアップ追従）
- 承認解決後は `approval_resolved` を Hub に送り、UI のカードを自動消去

## 日付・時刻表示

標準形式: `2026-05-06(水) 10:38:55`（曜日は日本語）

ログ・UI 表示は ISO 8601（`2026-05-06T10:38:55+09:00`）を内部保持し、UI 表示時のみ日本語形式に変換。`Date.toLocaleString()` はブラウザロケール依存のため使用禁止。

## セキュリティ規約

- **入力サニタイズ:** PTY からの出力は ANSI 除去後にパターンマッチ。コマンドパス文字列はバリデーション
- **トークン生成:** `crypto/rand` で 32 byte 以上の乱数を base64 化
- **CSRF:** WebSocket は同一オリジン制限 + トークン検証で保護
- **設定ファイル権限:** `~/.ai-cli-hub/config.yaml` は本人読み書きのみ（Unix: `0600`）
- **ログ書き込み:** PII やシークレットを含む承認内容は `risk:high` のときマスクする方針（実装時に詳細決定）

## テスト方針

- **Go:** `go test ./...` で単体テスト。PTY 関連は OS 別 build tag で分岐したテストファイル（`_unix_test.go` / `_windows_test.go`）
- **Vue:** Vitest（コンポーネント単体）+ Playwright（E2E、Hub 起動 → モックラッパー → UI 操作の通しテスト）
- **手動検証:** 4 ペイン（Claude × 2 / Codex × 2）並列起動 + Hub UI を別画面で常時表示、設計書 §9 のレイアウト通りに動くか確認

## ai-cli-hub 固有の禁止事項

- 既存の `claude` / `codex` バイナリを PATH 上で乗っ取らない（`gemini` は wrap 対象外につき関与しない）
- グローバルな環境変数（システム環境）を書き換えない
- 外部ネットワーク（インターネット）への通信を行わない
- 設定ファイル・ログ以外をユーザーホーム外に書き込まない
