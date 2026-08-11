package hub

// stale_watch.go: 稼働中 Hub の実行ファイルがディスク上で差し替わったこと
// （= make build したが Hub を再起動していない）を検知して UI へ通知する。
//
// 判定そのものは buildinfo.go の binaryGuard が持つ。ここが足すのは「いつ気づくか」。
// 従来の告知経路は 2 本あったが、どちらも実際に踏む場面を外していた:
//
//   - Web の常設バナー(#stale-binary-banner)はページ読み込み時の /api/info 1 回だけで
//     評価される。ダッシュボードを開いたまま make build すると、リロードするまで
//     永久に出ない（実測 2026-08-08: Hub 起動 12:57:19 → UI 接続 12:57:20 →
//     exe 差し替え 12:57:23。以後 5 時間近く古い Hub のまま気づけなかった）
//   - wrapper 側の staleguard は走行中セッションがあると logger.Warn を出すだけで、
//     出力先はログファイルなので画面には出ない
//
// 定期ポーリングは置かない。exe を差し替えるのは開発者だけで、リリース利用者には
// 一生 false のままの値を数十秒おきに stat し続けることになるため。代わりに、
// 既に走っている /api/info の判定に相乗りして状態変化の瞬間だけ配信する。
// /api/info は「ダッシュボードのページ読み込み」と「新規セッション起動」
//（internal/wrapper/staleguard.go が wrapper 起動ごとに必ず叩く）で呼ばれるので、
// 差し替え後に最初にセッションを立てた時点で、開きっぱなしのダッシュボードにも
// バナーが出る。常駐コストはゼロ。

import "many-ai-cli/internal/proto"

// noteStaleBinary は /api/info が算出した stale 値を受け取り、前回から変化して
// いたときだけ UI へ broadcast する。毎回配信するとバナーが作り直され続け、
// 承認バーで起きたのと同型の点滅を招くため、変化の瞬間だけに絞る。
//
// stale→false（元のバイナリへ戻した等）も配信し、バナーが出たまま固着しないようにする。
func (s *Server) noteStaleBinary(stale bool) {
	if !s.noteStaleBinaryChanged(stale) {
		return
	}
	if stale {
		s.logger.Warn("running Hub is a STALE binary; restart it to apply the rebuild")
	} else {
		s.logger.Info("running Hub matches the on-disk binary again")
	}
	value := stale
	// broadcast は UI 数ぶんの WS 書き込みを伴うので HTTP ハンドラを待たせない。
	s.safeGo("broadcast_binary_stale", func() {
		s.broadcast(proto.Message{Type: "binary_stale", BinaryStale: &value})
	})
}

// noteStaleBinaryChanged は直近の通知状態を stale で更新し、変化したかを返す。
// 配信判断だけを切り出してあるのは、時間や WS 接続に依存せずテストするため。
func (s *Server) noteStaleBinaryChanged(stale bool) bool {
	s.staleBinaryMu.Lock()
	defer s.staleBinaryMu.Unlock()
	if stale == s.staleBinaryNotified {
		return false
	}
	s.staleBinaryNotified = stale
	return true
}
