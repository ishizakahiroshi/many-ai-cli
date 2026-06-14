package hub

import (
	"context"
	"time"
)

// OpenSSH Server 状態検知（plan_mobile-connect-flow-redesign.md C2）。
//
// SSH 経路（スマホ側で ssh -L ローカルフォワードを張ってから 127.0.0.1 を開く）は
// PC 側に OpenSSH Server が導入・起動されている必要がある。Hub が自分側の
// sshd 状態を検知し、未導入なら有効化導線フラグを返す。
//
// 方針:
//   - OS 固有の検知は build tag で分離（Windows=Get-Service sshd 相当 /
//     unix=systemctl is-active sshd or sshd プロセス）。
//   - 検知 exec は必ず timeout 付き（CLI/サービス不在で degrade・500 にしない）。
//   - 検知できない（CLI 不在・権限不足等）場合は unknown に落とし、案内は出すが
//     エラーにはしない。

// sshd 状態機械の値。
const (
	sshdStateNotInstalled = "not_installed" // OpenSSH Server コンポーネント/バイナリが無い
	sshdStateStopped      = "stopped"       // 導入済みだがサービス停止中
	sshdStateRunning      = "running"       // 起動中（接続可能）
	sshdStateUnknown      = "unknown"       // 検知不能（権限不足・degrade）
)

const sshdProbeTimeout = 5 * time.Second

// sshdState は OpenSSH Server の自己診断結果。
type sshdState struct {
	State string // 状態機械の値（sshdState*）
}

// installed は OpenSSH Server が導入済み（stopped/running）かを返す。
func (st sshdState) installed() bool {
	return st.State == sshdStateRunning || st.State == sshdStateStopped
}

// running は sshd が起動中かを返す。
func (st sshdState) running() bool {
	return st.State == sshdStateRunning
}

// sshdProber は OpenSSH Server 状態検知を抽象化する（テストでモック注入する）。
type sshdProber interface {
	// probe は timeout 付きで sshd 状態を検知する。検知不能なら unknown を返す。
	probe(ctx context.Context) sshdState
}

// sshdProberFor は Server に注入された prober、未設定なら OS 実装を返す。
func (s *Server) sshdProberFor() sshdProber {
	if s.sshdProber != nil {
		return s.sshdProber
	}
	return osSSHDProber{}
}

// probeSSHD は OpenSSH Server 状態を検知する。CLI/サービス不在でも 500 にせず
// degrade（not_installed / unknown）する。
func (s *Server) probeSSHD(ctx context.Context) sshdState {
	probeCtx, cancel := context.WithTimeout(ctx, sshdProbeTimeout)
	defer cancel()
	return s.sshdProberFor().probe(probeCtx)
}
