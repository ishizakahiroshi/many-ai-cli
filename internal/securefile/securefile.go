// Package securefile は「Hub のホーム配下に置かれる token / secret 等の on-disk
// 資産のアクセス範囲を、当該ユーザー本人 + SYSTEM + Administrators だけに絞る」
// ためのごく小さな OS 抽象を提供する。
//
// C6 (plan_audit_score_s_promotion_2026-07-05.md): `os.Chmod(path, 0o600)` は
// Windows では READONLY 属性しか触らず NTFS DACL を狭めない (A3-1 finding)。
// このパッケージは build tag で OS 別実装を持ち、Windows は `SetNamedSecurityInfo`
// で DACL を明示制限、非 Windows は既存の `os.Chmod` に相当する no-op に倒す。
//
// 保存経路（config.yaml / push_store.json / launcher-profiles.yaml /
// approval_rules の central dir）は書き込み直後に本パッケージを呼ぶ。
// 失敗しても書き込み自体は既に成功しているため、呼び出し元は warning ログのみで
// 続行する（config が読めない事故を避ける）。
//
// 実装内訳:
//   - securefile_windows.go: `golang.org/x/sys/windows` で DACL 制限
//   - securefile_other.go:   非 Windows は no-op (既存の Chmod で十分な想定)
package securefile
