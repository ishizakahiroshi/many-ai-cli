package hub

import "testing"

func TestParseSSHServerIP(t *testing.T) {
	tests := []struct {
		name string
		conn string
		want string
	}{
		{"標準形式", "192.168.1.50 52344 192.168.1.1 22", "192.168.1.1"},
		{"IPv6", "fe80::1 52344 fd00::2 22", "fd00::2"},
		{"フィールド不足", "192.168.1.50 52344", ""},
		{"空文字", "", ""},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := parseSSHServerIP(tt.conn); got != tt.want {
				t.Errorf("parseSSHServerIP(%q) = %q, want %q", tt.conn, got, tt.want)
			}
		})
	}
}

func TestHostNetInfoSSH(t *testing.T) {
	t.Setenv("SSH_CONNECTION", "192.168.1.50 52344 192.168.1.1 22")
	ssh, ip := hostNetInfo()
	if !ssh {
		t.Error("SSH_CONNECTION 設定時に ssh=false")
	}
	if ip != "192.168.1.1" {
		t.Errorf("hostIP = %q, want %q", ip, "192.168.1.1")
	}
}

func TestHostNetInfoSSHClientOnly(t *testing.T) {
	// SSH_CONNECTION 無し + SSH_CLIENT 有り → SSH 判定のみ true、IP は NIC フォールバック
	t.Setenv("SSH_CONNECTION", "")
	t.Setenv("SSH_CLIENT", "192.168.1.50 52344 22")
	t.Setenv("SSH_TTY", "")
	ssh, _ := hostNetInfo()
	if !ssh {
		t.Error("SSH_CLIENT 設定時に ssh=false")
	}
}

func TestHostNetInfoHostLabelOverride(t *testing.T) {
	// ANY_AI_CLI_HOST_LABEL は SSH_CONNECTION のサーバ側 IP より優先される
	// （コンテナ内 sshd 経由だと SSH_CONNECTION がコンテナ IP になるため）
	t.Setenv("SSH_CONNECTION", "192.168.1.50 52344 172.19.0.2 22")
	t.Setenv("ANY_AI_CLI_HOST_LABEL", "203.0.113.10")
	ssh, ip := hostNetInfo()
	if !ssh {
		t.Error("SSH_CONNECTION 設定時に ssh=false")
	}
	if ip != "203.0.113.10" {
		t.Errorf("hostIP = %q, want %q", ip, "203.0.113.10")
	}
}

func TestHostNetInfoNonSSH(t *testing.T) {
	t.Setenv("SSH_CONNECTION", "")
	t.Setenv("SSH_CLIENT", "")
	t.Setenv("SSH_TTY", "")
	ssh, _ := hostNetInfo()
	if ssh {
		t.Error("SSH 環境変数なしで ssh=true")
	}
}

func TestResolveEnvMetaPriority(t *testing.T) {
	t.Setenv("ANY_AI_CLI_ENV_KIND", "vps")
	got := resolveEnvMeta("wsl", "windows-wsl", false, "host.example", true, "vps-tunnel")
	if got.Kind != "vps" || got.Label != "VPS" || got.HostLabel != "host.example" {
		t.Fatalf("env var should win, got %+v", got)
	}
}

func TestResolveEnvMetaInvalidExplicitFallsBackToLocal(t *testing.T) {
	t.Setenv("ANY_AI_CLI_ENV_KIND", "invalid")
	got := resolveEnvMeta("vps", "wsl", true, "", true, "vps-tunnel")
	if got.Kind != "local" {
		t.Fatalf("invalid explicit env kind = %q, want local", got.Kind)
	}
}

func TestResolveEnvMetaConfigAndHints(t *testing.T) {
	t.Run("config", func(t *testing.T) {
		got := resolveEnvMeta("wsl", "linux", false, "", false, "")
		if got.Kind != "wsl" {
			t.Fatalf("config env kind = %q, want wsl", got.Kind)
		}
	})
	t.Run("net hint kind", func(t *testing.T) {
		got := resolveEnvMeta("", "linux", false, "203.0.113.10", true, "vps-tunnel")
		if got.Kind != "vps-tunnel" || got.HostLabel != "203.0.113.10" {
			t.Fatalf("net hint env = %+v, want vps-tunnel with host", got)
		}
	})
	t.Run("net hint ssh fallback", func(t *testing.T) {
		got := resolveEnvMeta("", "linux", false, "", true, "")
		if got.Kind != "vps-tunnel" {
			t.Fatalf("net hint ssh env = %q, want vps-tunnel", got.Kind)
		}
	})
	t.Run("runtime wsl", func(t *testing.T) {
		got := resolveEnvMeta("", "windows-wsl", false, "", false, "")
		if got.Kind != "wsl" {
			t.Fatalf("runtime env = %q, want wsl", got.Kind)
		}
	})
	t.Run("ssh", func(t *testing.T) {
		got := resolveEnvMeta("", "linux", true, "", false, "")
		if got.Kind != "vps" {
			t.Fatalf("ssh env = %q, want vps", got.Kind)
		}
	})
	t.Run("default local", func(t *testing.T) {
		got := resolveEnvMeta("", "linux", false, "", false, "")
		if got.Kind != "local" {
			t.Fatalf("default env = %q, want local", got.Kind)
		}
	})
}
