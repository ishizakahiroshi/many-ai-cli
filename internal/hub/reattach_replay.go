package hub

// reattach_replay.go: wrapper 再接続時に「切断中に Hub が取りこぼした PTY
// バイト列」を確定するための計算。
//
// 背景（2026-08-02 の実測）:
// wrapper は出力キュー（ptyOutputQueueCapacity=64）が溢れると WS を意図的に
// 切断して 2 秒後に再接続する（internal/wrapper/wrapper.go の enqueue /
// reconnectSupervisor）。Codex はユーザー入力のたびにトランスクリプトを行単位で
// 描き直すため 1 秒間に数百チャンクを吐き、この切断が実際に頻発していた
// （セッション #1 で 1 日 11 回）。
//
// 切断中のバイト列は wrapper 側の replay リングバッファ（64KB）に残るが、
// 従来はそれを Hub の VT ミラーへ流し込むだけで、すでに接続済みのブラウザへは
// 一切送っていなかった。端末は「1 バイトも落ちない」前提のストリームなので、
// 絶対座標で部分再描画する TUI では以後の描画位置がずれ、画面が古いまま復帰
// できなくなる。

// reattachReplayGap は replay の末尾から「Hub が受信できていないぶん」だけを
// 切り出す。
//
// replay は直近 replayBufferLimit バイトの固定窓であり、その大半は切断前に
// Hub 経由で UI へ配信済み。全量を再配信すると xterm.js に同じ内容が二重描画
// される（docs/local/archive/v0.4.0/
// bugfix_codex-terminal-reconnect-replay-duplication_2026-07-06.md で一度踏んだ罠）。
//
// 取りこぼし量は「wrapper が PTY から読んだ累計 - Hub が受信した累計」で一意に
// 決まる。内容の重なり探索では決めない: 端末出力は同一フレームの再描画が多く、
// 最長一致は行き過ぎ（欠落）にも足りなさ（重複）にも倒れるうえ、どちらに倒れたかを
// 事後に検出できない。
//
// wrapperTotal <= 0 は「累計を申告しない旧 wrapper」。差分を特定できないので
// 何も返さない（当て推量で配信して画面を壊すより、従来どおり無配信にする）。
func reattachReplayGap(replay []byte, wrapperTotal, hubSeen int64) []byte {
	if len(replay) == 0 || wrapperTotal <= 0 {
		return nil
	}
	missing := wrapperTotal - hubSeen
	if missing <= 0 {
		return nil
	}
	if missing >= int64(len(replay)) {
		// 取りこぼしが replay 窓を超えた＝復元不能な穴がある。
		// 手元にある最大限として replay 全量を返す。
		return replay
	}
	return replay[int64(len(replay))-missing:]
}
