//go:build windows

package securefile

import (
	"fmt"
	"os"

	"golang.org/x/sys/windows"
)

// RestrictFile はファイル `path` の DACL を「current user + SYSTEM +
// Administrators のみ Full Control」に明示制限する（継承は切る）。
// path は既に存在している必要がある（存在確認は呼び出し元 or ここで stat）。
func RestrictFile(path string) error {
	if _, err := os.Stat(path); err != nil {
		return fmt.Errorf("securefile.RestrictFile stat: %w", err)
	}
	return restrictACL(path, false)
}

// EnsurePrivateDir はディレクトリ `path` を作成し、DACL を「current user +
// SYSTEM + Administrators のみ Full Control」に制限する（サブディレクトリ・
// ファイルにも継承する）。
func EnsurePrivateDir(path string) error {
	if err := os.MkdirAll(path, 0o700); err != nil {
		return fmt.Errorf("securefile.EnsurePrivateDir mkdir: %w", err)
	}
	return restrictACL(path, true)
}

// restrictACL は path の DACL を上記 3 SID のみに置き換える。
// isDir=true なら子への継承を有効にする。
func restrictACL(path string, isDir bool) error {
	// 1. 現在のプロセスユーザーの SID を取得
	var token windows.Token
	if err := windows.OpenProcessToken(windows.CurrentProcess(), windows.TOKEN_QUERY, &token); err != nil {
		return fmt.Errorf("open process token: %w", err)
	}
	defer token.Close()
	tokenUser, err := token.GetTokenUser()
	if err != nil {
		return fmt.Errorf("get token user: %w", err)
	}
	userSID := tokenUser.User.Sid

	// 2. well-known SIDs (SYSTEM / BUILTIN\Administrators)
	systemSID, err := windows.CreateWellKnownSid(windows.WinLocalSystemSid)
	if err != nil {
		return fmt.Errorf("create system sid: %w", err)
	}
	adminsSID, err := windows.CreateWellKnownSid(windows.WinBuiltinAdministratorsSid)
	if err != nil {
		return fmt.Errorf("create admins sid: %w", err)
	}

	// 3. 継承モード決定 (Windows API の EXPLICIT_ACCESS.Inheritance は uint32)
	var inheritMode uint32 = uint32(windows.NO_INHERITANCE)
	if isDir {
		inheritMode = uint32(windows.SUB_CONTAINERS_AND_OBJECTS_INHERIT)
	}

	// 4. 3 SID それぞれに GENERIC_ALL の grant ACE を作る
	entries := []windows.EXPLICIT_ACCESS{
		makeGrantACE(userSID, windows.TRUSTEE_IS_USER, inheritMode),
		makeGrantACE(systemSID, windows.TRUSTEE_IS_WELL_KNOWN_GROUP, inheritMode),
		makeGrantACE(adminsSID, windows.TRUSTEE_IS_WELL_KNOWN_GROUP, inheritMode),
	}

	// 5. ACL を組み立てる
	dacl, err := windows.ACLFromEntries(entries, nil)
	if err != nil {
		return fmt.Errorf("build dacl: %w", err)
	}

	// 6. SetNamedSecurityInfo で
	//    - DACL_SECURITY_INFORMATION: DACL を上書き
	//    - PROTECTED_DACL_SECURITY_INFORMATION: 親ディレクトリからの ACE 継承を切る
	secInfo := windows.SECURITY_INFORMATION(windows.DACL_SECURITY_INFORMATION | windows.PROTECTED_DACL_SECURITY_INFORMATION)
	if err := windows.SetNamedSecurityInfo(path, windows.SE_FILE_OBJECT, secInfo, nil, nil, dacl, nil); err != nil {
		return fmt.Errorf("set named security info: %w", err)
	}
	return nil
}

// makeGrantACE は 1 つの SID に対する GENERIC_ALL grant ACE を構築する。
func makeGrantACE(sid *windows.SID, trusteeType windows.TRUSTEE_TYPE, inheritMode uint32) windows.EXPLICIT_ACCESS {
	return windows.EXPLICIT_ACCESS{
		AccessPermissions: windows.GENERIC_ALL,
		AccessMode:        windows.GRANT_ACCESS,
		Inheritance:       inheritMode,
		Trustee: windows.TRUSTEE{
			TrusteeForm:  windows.TRUSTEE_IS_SID,
			TrusteeType:  trusteeType,
			TrusteeValue: windows.TrusteeValueFromSID(sid),
		},
	}
}
