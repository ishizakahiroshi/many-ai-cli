package wrapper

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"time"
)

// 子セッションへの委譲を AI へ伝えるための、セッション所有の一時プロンプトファイル。
//
// なぜ承認ルールファイル（approval_rules.go）に置かないか:
//
// あちらは ~/.many-ai-cli/approval-rules.md を生成し、利用者の CLAUDE.md へ import 行を
// 追記して読ませる仕組みで、載っているのは「AI と Hub のあいだの書式の約束事」
// （承認マーカー・完了サマリ・orchestration エラー行の解釈）だけである。委譲は書式ではなく
// 機能の案内なので責務が違う。加えてあの経路は 3 つの副作用を持つ:
//
//  1. 全セッション共通の 1 ファイルなので、セッションごとに載せる / 載せないを選べない
//  2. 常駐の文脈に永続的に載るため、委譲を一度も使わないセッションも毎回読む
//  3. 利用者の CLAUDE.md / AGENTS.md を書き換える（注入・除去・置き去り検査が要る）
//
// `claude --append-system-prompt-file <path>` はこの 3 つすべてを回避する。セッション単位で
// 渡せて、AI にターンを消費させず、利用者のファイルを一切触らない。
//
// ON / OFF が止めるのは「AI が委譲を知るかどうか」であって、委譲できるかどうかではない。
// `orchestrate` サブコマンド自体は全セッションで動く（wrapper.go の providerExtraEnv）。
// **安全策と混同しないこと。** 実際の歯止めは spawn 時の確認ダイアログ側にある
// （config の spawn_confirm_mode・既定 on。ブラウザ未接続と期限切れは拒否）。
// したがってこのスイッチの目的は常駐文脈のコスト削減であり、記憶させてよい。
//
// 対応 provider（2026-08-29 に 6 CLI へ `--help` を実行して確認）:
//
//	claude  --append-system-prompt-file <path>  … 一時ファイルを渡す
//	grok    --rules <RULES>                     … 本文を直接渡す（help: "Extra rules to
//	                                              append to the system prompt"）
//
// grok の `--system-prompt-override` は**置換**なので使わない。CLI 本体の system prompt を
// 消してしまう。追記の `--rules` だけを使う。
//
// 残り 4 つにセッション単位の口は無い。codex は `-c key=value` を持つが指示ファイルを差す
// キーの有無が未確認、opencode は cwd の `opencode.json` を書き換える経路しかなく（利用者の
// リポジトリを触るうえ 1 cwd 1 セッション）、copilot と cursor-agent はファイル方式だけ
// （`--no-custom-instructions` が AGENTS.md 等を読んでいることを示す / `.cursor/rules`）。
// この 4 つはファイル注入方式で別途対応する（plan の C7）。
const delegationPromptText = `many-ai-cli: you can delegate work to child AI sessions on this machine.

- Spawn a child: many-ai-cli orchestrate spawn --role <role> "<prompt>"
- Instruct a live child: many-ai-cli orchestrate send --role <role> "<text>"
  (spawning again for a role that already has a live child is rejected with 409 — use send)
- Run a plan file through implementation -> review -> fix rounds: many-ai-cli orchestrate relay --plan <path>
- Details: many-ai-cli orchestrate --help

Do not call the Hub HTTP API or handle its auth token directly; these subcommands do it for you.

Which AI runs the child: pass --provider only when the user named one. The exact accepted values are
claude, codex, copilot, cursor-agent, opencode and grok — all lowercase, and nothing else is accepted
("Codex", "ChatGPT", "GPT-5" are rejected). When the user did not name one, omit --provider: the Hub
reuses whatever ran that role last time, and falls back to this session's own provider. The user can
change it in the approval dialog, and that choice is remembered for the role.

Use this when the user asks for delegation, or when the work is clearly faster in parallel.
Do not spawn on your own initiative for ordinary tasks: every child costs a separate budget and
context. Ask the user in one line if you are unsure. Start a relay only when the user explicitly
asks for it — the relay drives several children over multiple rounds without asking again.
`

// DelegationEnvName は Hub が spawn 時に渡す ON / OFF。"1" のときだけ有効。
// Hub 経由でないセッション（`many-ai-cli wrap claude` を手で起動した場合）は
// 未設定になるので、呼び出し側が設定ファイルの既定へフォールバックする。
const DelegationEnvName = "MANY_AI_CLI_DELEGATION"

const (
	delegationPromptPrefix = "aac-delegation-"
	delegationPromptSuffix = ".md"
	// プロバイダのプロセスは wrapper より少しだけ長く生き残ることがある。
	// 起動途中の CLI が読む可能性のあるファイルを、素早い再起動が消さない長さにする
	// （usage_hooks.go の claudeSessionSettingsStaleAfter と同じ理由・同じ値）。
	delegationPromptStaleAfter = 24 * time.Hour
)

var delegationPromptNameRe = regexp.MustCompile(`^aac-delegation-u([0-9a-f]{16})-s([0-9]+)-p([0-9]+)\.md$`)

// DelegationPromptEnabled は env の指定を、無ければ設定ファイルの既定を返す。
// env が明示されていればそちらが常に勝つ（Hub がセッションごとに決めた値）。
func DelegationPromptEnabled(configDefault bool) bool {
	switch os.Getenv(DelegationEnvName) {
	case "1":
		return true
	case "0":
		return false
	default:
		return configDefault
	}
}

// cleanupStaleDelegationPrompts は、この版の命名規則・この OS ユーザーが作ったファイルだけを
// 消す。生きている PID・新しい mtime・別 owner・symlink・解釈できない名前は触らない
// （usage_hooks.go の同名処理と同じ方針。走査対象の正規表現だけが違う）。
func cleanupStaleDelegationPrompts(dir string, now time.Time) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	ownerID := claudeSessionSettingsOwnerID()
	for _, entry := range entries {
		match := delegationPromptNameRe.FindStringSubmatch(entry.Name())
		if match == nil || match[1] != ownerID {
			continue
		}
		pid, err := strconv.Atoi(match[3])
		if err != nil || pid <= 0 || processAlive(pid) {
			continue
		}
		path := filepath.Join(dir, entry.Name())
		info, err := os.Lstat(path)
		if err != nil || !info.Mode().IsRegular() || now.Sub(info.ModTime()) < delegationPromptStaleAfter {
			continue
		}
		_ = os.Remove(path)
	}
}

// DelegationProviderArgs は、その provider へ委譲案内を渡すために足す引数を返す。
// 対応していない provider では nil を返す（呼び出し側は何も足さない）。
// cleanup は一時ファイルを作った場合のみ非 nil。
//
// provider ごとに渡し方が違うので、分岐をここへ閉じ込める。wrapper.go 側は
// 「返ってきた引数を足して、cleanup を defer する」だけにする。
func DelegationProviderArgs(provider string, sessionID int) (args []string, cleanup func(), err error) {
	switch provider {
	case "claude":
		path, cleanupFile, writeErr := WriteDelegationPromptFile(sessionID)
		if writeErr != nil {
			return nil, nil, writeErr
		}
		return []string{"--append-system-prompt-file", path}, cleanupFile, nil
	case "grok":
		// grok は本文を値として受け取るので一時ファイルは要らない。
		return []string{"--rules", delegationPromptText}, nil, nil
	default:
		return nil, nil, nil
	}
}

// WriteDelegationPromptFile は委譲案内を wrapper 所有の一時ファイルへ書き出し、
// そのパスと後始末関数を返す。`claude --append-system-prompt-file <path>` に渡して使う。
func WriteDelegationPromptFile(sessionID int) (path string, cleanup func(), err error) {
	cleanupStaleDelegationPrompts(os.TempDir(), time.Now())
	name := fmt.Sprintf("%su%s-s%d-p%d%s", delegationPromptPrefix, claudeSessionSettingsOwnerID(), sessionID, os.Getpid(), delegationPromptSuffix)
	path = filepath.Join(os.TempDir(), name)
	// 共用 /tmp での symlink 追従を防ぐ: 既存を除去してから O_EXCL で排他生成する
	// （usage_hooks.go の AUDIT-4 と同じ理由）。秘密は含まないが、他ユーザーが張った
	// symlink 経由で任意ファイルを truncate されないことは同じく必要。
	_ = os.Remove(path)
	f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o600) // #nosec G304 -- 固定名の wrapper 専用 temp
	if err != nil {
		return "", nil, fmt.Errorf("create delegation prompt: %w", err)
	}
	if _, err := f.WriteString(delegationPromptText); err != nil {
		_ = f.Close()
		_ = os.Remove(path)
		return "", nil, fmt.Errorf("write delegation prompt: %w", err)
	}
	if err := f.Close(); err != nil {
		_ = os.Remove(path)
		return "", nil, fmt.Errorf("close delegation prompt: %w", err)
	}
	cleanup = func() { _ = os.Remove(path) }
	return path, cleanup, nil
}
