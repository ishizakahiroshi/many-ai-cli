//go:build maidebug

package hub

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/sessionlog"
)

// 破損マーカーブロックの生データダンプ（観測専用）。
//
// 2026-08-28 再導入。v0.7.0 で撤去したが、撤去後も握り潰しは続いており
// （hub ログ実測: 2026-08-23〜08-27 の約 4 日で 172 件・option_start 119 /
// marker_leak 30 / duplicate_option 20 / box_rule 3）、原因の特定に生データが要る。
// 撤去時と違い、今回は maidebug ビルドタグを付けて既定の成果物には入れない。
// 収集の可否は従来どおり log.session_enabled で切る（同梱と収集は別のゲート）。
//
// classifyApprovalMarkerBlock は「壊れていたら捨てる」症状抑制で、なぜ Hub の
// VT ミラーだけが壊れるのかには手が付いていない。原因を後から突き合わせるには
// 次の 3 点が同時に要る。
//
//  1. Hub が描いた結果（marker.Block）
//  2. その入力になった PTY の生バイト列（ses.ptyBuf の末尾）
//  3. 描画時点のミラー寸法と、直近 resize からの位置関係
//
// 3 が要るのは、このミラーが実端末と違って resize で reflow しないため
// （vt_buffer.go Resize のコメント）。寸法ずれと resize 直後かどうかが
// 乖離の有力な説明変数になる。1 と 2 が揃えば、同じバイト列を同じ寸法へ
// 流し直して破損が再現するか（＝ミラーの実装が原因か、一過性の desync か）を
// 判定できる。
//
// この処理は抑止判定・配信には一切関与しない。失敗しても握り潰す。
const (
	// 1 事象あたりに残す PTY 生バイトの上限。マーカーブロックは長くても数 KB だが、
	// 直前の再描画・resize シーケンスまで遡れないと入力側の再現ができない。
	approvalCorruptPTYTailBytes = 64 * 1024
	// 1 日分のダンプファイルの上限。超えたらその日はそれ以上書かない。
	approvalCorruptDailyMaxBytes = 32 * 1024 * 1024
	// ダンプを保持する日数。
	approvalCorruptRetainDays = 7
)

// approvalCorruptSnapshot は sessionsMu 保持中に取る値のコピー。
// ptyBuf はリングバッファで呼び出し後も書き換わるため、必ず複製する。
type approvalCorruptSnapshot struct {
	ok           bool
	ptyTail      []byte
	vtCols       int
	vtRows       int
	lastSentCols int
	lastSentRows int
	// sinceResize は直近の VT resize から検出までの経過。マーカー抽出は
	// resize debounce の外でしか走らない（wrapper_loop.go）ので「debounce の残り」は
	// 常に 0 になり指標にならない。ミラーは resize で reflow しない実装なので、
	// debounce を抜けた直後がいちばん怪しい。resize 未発生なら負値のまま
	// 記録しない（-1）。
	sinceResize time.Duration
}

// 記録点は approval_marker.go に 2 つ残す（sharedFiles）。スナップショットは
// sessionsMu を保持したまま取らなければならず（ptyBuf はリングバッファ）、
// ファイル IO は解放後でなければならないので、2 点に分けて間を下の待避棚で繋ぐ。
// 棚ごとこのファイルにあるので、撤去すれば記録点 2 行以外は何も残らない。
func init() {
	registerProbeSink("approval-corrupt-snapshot", func(s *Server, args ...any) {
		if len(args) != 3 {
			return
		}
		id, _ := args[0].(int)
		ses, _ := args[1].(*session)
		at, _ := args[2].(time.Time)
		stashApprovalCorruptSnapshot(id, snapshotApprovalCorrupt(ses, at))
	})
	registerProbeSink("approval-corrupt-dump", func(s *Server, args ...any) {
		if len(args) != 5 {
			return
		}
		id, _ := args[0].(int)
		provider, _ := args[1].(string)
		reason, _ := args[2].(string)
		marker, _ := args[3].(*approvalMarkerBlock)
		at, _ := args[4].(time.Time)
		snap, ok := takeApprovalCorruptSnapshot(id)
		if !ok {
			return
		}
		s.dumpCorruptApprovalBlock(id, provider, reason, marker, snap, at)
	})
}

// 待避棚。記録点 2 つのあいだだけ値を持つ。両方 notify のときにしか呼ばれないので
// 溜まり続けることは無いが、取り出し側が来なかったときのために上限を置く。
var (
	approvalCorruptStashMu sync.Mutex
	approvalCorruptStash   = map[int]approvalCorruptSnapshot{}
)

const approvalCorruptStashMax = 64

func stashApprovalCorruptSnapshot(id int, snap approvalCorruptSnapshot) {
	approvalCorruptStashMu.Lock()
	defer approvalCorruptStashMu.Unlock()
	if len(approvalCorruptStash) >= approvalCorruptStashMax {
		for k := range approvalCorruptStash {
			delete(approvalCorruptStash, k)
			break
		}
	}
	approvalCorruptStash[id] = snap
}

func takeApprovalCorruptSnapshot(id int) (approvalCorruptSnapshot, bool) {
	approvalCorruptStashMu.Lock()
	defer approvalCorruptStashMu.Unlock()
	snap, ok := approvalCorruptStash[id]
	delete(approvalCorruptStash, id)
	return snap, ok
}

// snapshotApprovalCorrupt は sessionsMu を保持したまま呼ぶこと。
func snapshotApprovalCorrupt(ses *session, at time.Time) approvalCorruptSnapshot {
	if ses == nil {
		return approvalCorruptSnapshot{}
	}
	snap := approvalCorruptSnapshot{
		ok:           true,
		lastSentCols: ses.lastCols,
		lastSentRows: ses.lastRows,
		sinceResize:  -1,
	}
	if ses.vt != nil {
		snap.vtCols, snap.vtRows = ses.vt.cols, ses.vt.rows
	}
	// vtResizeDebounceUntil は resize 時刻 + vtResizeDebounce で設定される。
	if !ses.vtResizeDebounceUntil.IsZero() {
		snap.sinceResize = at.Sub(ses.vtResizeDebounceUntil.Add(-vtResizeDebounce))
	}
	tail := ses.ptyBuf
	if len(tail) > approvalCorruptPTYTailBytes {
		tail = tail[len(tail)-approvalCorruptPTYTailBytes:]
	}
	snap.ptyTail = append([]byte(nil), tail...)
	return snap
}

type approvalCorruptRecord struct {
	TS           string `json:"ts"`
	SessionID    int    `json:"session_id"`
	Provider     string `json:"provider"`
	Reason       string `json:"reason"`
	Sig          string `json:"sig"`
	VTCols       int    `json:"vt_cols"`
	VTRows       int    `json:"vt_rows"`
	LastSentCols int    `json:"last_sent_cols"`
	LastSentRows int    `json:"last_sent_rows"`
	// SinceResizeMS は直近の VT resize からの経過。-1 は resize 未発生。
	// 値が小さいレコードに破損が偏るなら、原因は「reflow しないミラー」側にある。
	SinceResizeMS int64  `json:"ms_since_last_resize"`
	BlockB64      string `json:"block_b64"`
	PTYTailB64    string `json:"pty_tail_b64"`
	PTYTailBytes  int    `json:"pty_tail_bytes"`
	// Masked=true は MaskSecrets がバイト列を書き換えたことを示す。桁位置が
	// ずれている可能性があるので、折り返し解析ではこのレコードを除外する。
	Masked bool `json:"masked"`
}

func (s *Server) dumpCorruptApprovalBlock(id int, provider, reason string, marker *approvalMarkerBlock, snap approvalCorruptSnapshot, at time.Time) {
	if !snap.ok || marker == nil || s.cfg == nil {
		return
	}
	// セッションログの opt-in に従う。MaskSecrets を通してはいるが、保存するのは
	// PTY の生バイト列で、マスクが取りこぼした秘密が残り得る点は .log と同じ。
	// 「ログの opt-in と無関係に入力由来のバイトを永続化しない」は v0.6.0 リリース前
	// 監査 A-01 の指摘そのものなので、同じ設定で一括して切れるようにする。
	if !s.cfg.Log.SessionEnabled {
		return
	}
	dir := strings.TrimSpace(s.cfg.Hub.LogDir)
	if dir == "" {
		return
	}
	dir = filepath.Join(dir, "approval-corrupt")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return
	}

	// 既存のセッション履歴と同じ扱いで secrets をマスクしてから保存する
	// （wrapper_loop.go の pty_data 経路と同じ方針）。
	maskedBlock := sessionlog.MaskSecrets(marker.Block)
	maskedTail := sessionlog.MaskSecrets(string(snap.ptyTail))
	sinceResizeMS := int64(-1)
	if snap.sinceResize >= 0 {
		sinceResizeMS = snap.sinceResize.Milliseconds()
	}
	rec := approvalCorruptRecord{
		TS:            at.Format(time.RFC3339Nano),
		SessionID:     id,
		Provider:      provider,
		Reason:        reason,
		Sig:           marker.Sig,
		VTCols:        snap.vtCols,
		VTRows:        snap.vtRows,
		LastSentCols:  snap.lastSentCols,
		LastSentRows:  snap.lastSentRows,
		SinceResizeMS: sinceResizeMS,
		BlockB64:      sessionlog.EncodeBase64([]byte(maskedBlock)),
		PTYTailB64:    sessionlog.EncodeBase64([]byte(maskedTail)),
		PTYTailBytes:  len(snap.ptyTail),
		Masked:        maskedBlock != marker.Block || maskedTail != string(snap.ptyTail),
	}
	line, err := json.Marshal(rec)
	if err != nil {
		return
	}

	path := filepath.Join(dir, at.Format("2006-01-02")+".jsonl")
	if fi, err := os.Stat(path); err == nil && fi.Size() >= approvalCorruptDailyMaxBytes {
		return
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	_, _ = f.Write(append(line, '\n'))
	_ = f.Close()

	pruneApprovalCorruptDumps(dir, at)
}

// pruneApprovalCorruptDumps は保持日数を超えた日次ファイルを削除する。
// ファイル名の日付で判定する（mtime だとコピー・同期で狂う）。
func pruneApprovalCorruptDumps(dir string, now time.Time) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return
	}
	var names []string
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".jsonl") {
			continue
		}
		day, err := time.Parse("2006-01-02", strings.TrimSuffix(e.Name(), ".jsonl"))
		if err != nil {
			continue
		}
		if now.Sub(day) > time.Duration(approvalCorruptRetainDays)*24*time.Hour {
			names = append(names, e.Name())
		}
	}
	sort.Strings(names)
	for _, n := range names {
		_ = os.Remove(filepath.Join(dir, n))
	}
}
