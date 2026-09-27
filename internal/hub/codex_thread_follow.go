package hub

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// Codex の会話の切り替えへの追従（2026-09-25・
// docs/local/bugfix_codex-approval-missed-after-thread-switch_2026-09-25.md）。
//
// Codex は同じプロセスの中で新しい会話を始めると（/new 等）、別の rollout ファイルへ
// 書き始める。Hub はセッションの rollout を「開始時刻から ±10 分」で探す
// （findCodexRolloutLog）ので、切り替え後の会話は窓の外になり、開始時の古い
// rollout を読み続ける。古いファイルは読めるので「読めないときは VT ミラーへ戻す」
// 退避も働かず、承認マーカーが何のログも残さずに消えていた。
//
// Stop フックで正確なパスを受け取る案は採っていない。フックの書式・置き場所・
// 信頼の登録（Codex の /hooks）が CLI ごとに違い、CLI の更新で黙って発火しなくなる。
// many-ai-cli は複数の AI CLI を束ねる道具なので、CLI 側の設定に頼らず、Codex が
// 自分で書くファイルだけから判断する。
//
// 乗り換え先の条件（すべて満たすもの）:
//   - 利用者が始めた会話（サブエージェントの rollout は同じ cwd で作られるので除く）
//   - 同じ CODEX_HOME・同じ cwd
//   - 今の会話より後に始まっている
//   - 今の会話のファイルが、その会話が始まった後も書かれ続けていない
//     （書かれ続けているなら、それは並行して動く別の Codex の会話）
//   - 同じ CODEX_HOME・cwd で動いている他のセッションが、読んでいない・読む見込みもない
//
// 同じ CODEX_HOME・cwd で他の Codex セッションが動いていると、条件を満たす会話が
// どちらのものか決められない。そのときは取り違えるより承認を出す側に倒し、
// 乗り換えずに供給元を VT ミラーへ戻す（codexThreadAmbiguous）。

// codexThreadFollowInterval は乗り換え先を探す間隔。rollout の先頭行を読むので
// 毎 poll（1 秒）には回さない。切り替えは会話の最初の発言で起きるので、
// 最後の承認マーカーが出るまでには十分に間に合う。
const codexThreadFollowInterval = 5 * time.Second

// codexThreadSwitchSlack は「今の会話のファイルが、新しい会話が始まった後も
// 書かれ続けているか」を判定するときの許容差。切り替えの瞬間に古いファイルへ
// 締めの記録が 1 行入ることがあるので、その分を見込む。
const codexThreadSwitchSlack = 5 * time.Second

// codexThreadFollowMaxDays は乗り換え先を探す日付ディレクトリを何日前まで遡るか。
const codexThreadFollowMaxDays = 2

// codexThreadPeer は同じ CODEX_HOME・cwd で動いている他の Codex セッション。
type codexThreadPeer struct {
	paths     []string
	startedAt time.Time
}

// followCodexThreadSwitch は、セッションの Codex が新しい会話へ切り替わっていたら
// NativeLogPath をその rollout へ向ける。NativeLogPath はトランスクリプトの解決で
// 開始時刻による検索より優先されるので、次の poll から新しい会話を読む。
// NativeLogPath を書き換えたときだけ true を返す。
func (s *Server) followCodexThreadSwitch(id int, currentPath string, now time.Time) bool {
	s.sessionsMu.Lock()
	ses := s.sessions[id]
	if ses == nil || ses.Provider != "codex" || currentPath == "" ||
		now.Sub(ses.codexThreadCheckedAt) < codexThreadFollowInterval {
		s.sessionsMu.Unlock()
		return false
	}
	ses.codexThreadCheckedAt = now
	root := codexHomeForSession(ses)
	cwd := ses.CWD
	var peers []codexThreadPeer
	for otherID, other := range s.sessions {
		if otherID == id || other == nil || other.Provider != "codex" || isTerminalSessionState(other.State) {
			continue
		}
		if !grokPathsEquivalent(codexHomeForSession(other), root) || !grokPathsEquivalent(other.CWD, cwd) {
			continue
		}
		peer := codexThreadPeer{}
		for _, path := range []string{other.agentChatPath, other.NativeLogPath} {
			if path != "" {
				peer.paths = append(peer.paths, path)
			}
		}
		peer.startedAt, _ = time.Parse(time.RFC3339, other.StartedAt)
		peers = append(peers, peer)
	}
	s.sessionsMu.Unlock()
	if root == "" || cwd == "" {
		return false
	}

	next, ambiguous := pickCodexThreadSwitch(root, cwd, currentPath, peers, now)

	s.sessionsMu.Lock()
	ses = s.sessions[id]
	if ses == nil {
		s.sessionsMu.Unlock()
		return false
	}
	wasAmbiguous := ses.codexThreadAmbiguous
	ses.codexThreadAmbiguous = ambiguous
	switched := next != "" && codexTranscriptPathAllowed(ses, next)
	if switched {
		ses.NativeLogPath = next
	}
	s.sessionsMu.Unlock()

	if s.logger != nil {
		switch {
		case switched:
			s.logger.Info("codex transcript followed to a new conversation",
				"session_id", id, "from", filepath.Base(currentPath), "to", filepath.Base(next))
		case ambiguous && !wasAmbiguous:
			// 無音で沈黙させないため、VT へ退避したことは必ず残す。
			s.logger.Warn("codex conversation switch is ambiguous; approval marker source fell back to VT mirror",
				"session_id", id, "path", filepath.Base(currentPath), "peers", len(peers))
		case !ambiguous && wasAmbiguous:
			s.logger.Info("codex conversation switch resolved; approval marker source back on transcript",
				"session_id", id)
		}
	}
	return switched
}

// pickCodexThreadSwitch は乗り換え先の rollout を返す。候補が無ければ ("", false)、
// 他のセッションとの取り違えを否定できなければ ("", true) を返す。
func pickCodexThreadSwitch(root, cwd, currentPath string, peers []codexThreadPeer, now time.Time) (string, bool) {
	current, ok := readCodexSessionMeta(currentPath)
	if !ok {
		return "", false
	}
	currentStart, err := time.Parse(time.RFC3339Nano, current.Payload.Timestamp)
	if err != nil {
		return "", false
	}
	info, err := os.Stat(currentPath)
	if err != nil {
		return "", false
	}
	currentLastWrite := info.ModTime()

	bestPath := ""
	var bestStart time.Time
	found := false
	for _, dir := range codexSessionDayDirs(root, currentStart, now) {
		entries, err := os.ReadDir(dir)
		if err != nil {
			continue
		}
		for _, e := range entries {
			if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
				continue
			}
			path := filepath.Join(dir, e.Name())
			if grokPathsEquivalent(path, currentPath) {
				continue
			}
			// 今の会話が始まった後に 1 度も書かれていないファイルは、後から始まった
			// 会話ではありえない。先頭行を読む前に stat だけで落とす。
			if fi, err := e.Info(); err != nil || fi.ModTime().Before(currentStart) {
				continue
			}
			meta, ok := readCodexSessionMeta(path)
			if !ok || !isCodexUserThread(meta) || !grokPathsEquivalent(meta.Payload.CWD, cwd) {
				continue
			}
			start, err := time.Parse(time.RFC3339Nano, meta.Payload.Timestamp)
			if err != nil || !start.After(currentStart) {
				continue
			}
			if currentLastWrite.After(start.Add(codexThreadSwitchSlack)) {
				continue
			}
			if codexThreadClaimedByPeer(path, start, peers) {
				continue
			}
			found = true
			if bestPath == "" || start.After(bestStart) {
				bestPath = path
				bestStart = start
			}
		}
	}
	if !found {
		return "", false
	}
	if len(peers) > 0 {
		return "", true
	}
	return bestPath, false
}

// isCodexUserThread は rollout が利用者の始めた会話かを返す。thread_source を
// 書かない古い Codex では、親の会話を持たないものを利用者の会話とみなす。
func isCodexUserThread(meta codexSessionMeta) bool {
	if meta.Payload.ParentThreadID != "" {
		return false
	}
	return meta.Payload.ThreadSource == "" || meta.Payload.ThreadSource == "user"
}

// codexThreadClaimedByPeer は、その rollout を他のセッションが読んでいるか、
// まだパスを決めていないセッションが開始時刻で拾う見込みがあるかを返す。
func codexThreadClaimedByPeer(path string, start time.Time, peers []codexThreadPeer) bool {
	for _, peer := range peers {
		for _, claimed := range peer.paths {
			if grokPathsEquivalent(claimed, path) {
				return true
			}
		}
		if len(peer.paths) == 0 && !peer.startedAt.IsZero() &&
			start.Sub(peer.startedAt).Abs() <= codexRolloutMatchWindow {
			return true
		}
	}
	return false
}

// codexSessionDayDirs は from の日から now の日までの sessions/YYYY/MM/DD を返す
// （codexThreadFollowMaxDays より前は見ない）。
func codexSessionDayDirs(root string, from, now time.Time) []string {
	start := from.Local()
	end := now.Local()
	if floor := end.AddDate(0, 0, -codexThreadFollowMaxDays); start.Before(floor) {
		start = floor
	}
	start = time.Date(start.Year(), start.Month(), start.Day(), 0, 0, 0, 0, start.Location())
	var dirs []string
	for day := start; !day.After(end); day = day.AddDate(0, 0, 1) {
		dirs = append(dirs, filepath.Join(root, "sessions",
			fmt.Sprintf("%04d", day.Year()), fmt.Sprintf("%02d", day.Month()), fmt.Sprintf("%02d", day.Day())))
	}
	return dirs
}
