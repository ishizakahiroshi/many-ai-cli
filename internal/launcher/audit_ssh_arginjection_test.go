package launcher

import (
	"regexp"
	"strings"
	"testing"
)

// --- validateSSH: reject "-"-prefixed host (local ssh option injection) ---
//
// A crafted profile whose host begins with "-" (e.g. "-oProxyCommand=calc.exe")
// would be reinterpreted by the ssh client as a local option, running an
// arbitrary command on the local machine. Validation must reject it.
func TestValidateSSH_RejectsDashPrefixedHost(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name: "evil",
			Type: ProfileTypeSSH,
			Mode: SSHModeServe,
			Host: "-oProxyCommand=calc.exe",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for '-'-prefixed host, got nil")
	}
}

// The user portion is also positional ("user@host"), so a "-"-prefixed user
// must be rejected too.
func TestValidateSSH_RejectsDashPrefixedUser(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name: "evil",
			Type: ProfileTypeSSH,
			Mode: SSHModeServe,
			Host: "example.com",
			User: "-oProxyCommand=calc.exe",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for '-'-prefixed user, got nil")
	}
}

// The UI server calls Validate without normalizeProfile, so a "-"-prefixed user
// embedded as "user@host" in the Host field must still be caught. validateSSH
// normalizes a local copy before checking.
func TestValidateSSH_RejectsDashPrefixedUserInHost(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name: "evil",
			Type: ProfileTypeSSH,
			Mode: SSHModeServe,
			Host: "-oProxyCommand=calc.exe@example.com",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for '-'-prefixed user embedded in host, got nil")
	}
}

// A host that begins with "-" only after the "user@" split must also be caught.
func TestValidateSSH_RejectsDashPrefixedHostInUserAtHost(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name: "evil",
			Type: ProfileTypeSSH,
			Mode: SSHModeServe,
			Host: "root@-oProxyCommand=calc.exe",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for '-'-prefixed host in user@host, got nil")
	}
}

// --- validateSSH: reject whitespace / control chars in host ---

func TestValidateSSH_RejectsWhitespaceHost(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name: "evil",
			Type: ProfileTypeSSH,
			Mode: SSHModeServe,
			Host: "example.com extra",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for whitespace in host, got nil")
	}
}

func TestValidateSSH_RejectsControlCharHost(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name: "evil",
			Type: ProfileTypeSSH,
			Mode: SSHModeServe,
			Host: "example.com\nmalicious",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for control char in host, got nil")
	}
}

// --- validateSSH: reject "-"-prefixed binary / identity_file ---

func TestValidateSSH_RejectsDashPrefixedBinary(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name:   "evil",
			Type:   ProfileTypeSSH,
			Mode:   SSHModeServe,
			Host:   "example.com",
			Binary: "-rf",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for '-'-prefixed binary, got nil")
	}
}

func TestValidateSSH_RejectsDashPrefixedIdentityFile(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name:         "evil",
			Type:         ProfileTypeSSH,
			Mode:         SSHModeServe,
			Host:         "example.com",
			IdentityFile: "-oProxyCommand=calc.exe",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Fatal("expected error for '-'-prefixed identity_file, got nil")
	}
}

// --- validateSSH: legitimate values still pass ---
//
// Normal hostnames, IPs, user@host, default binary, and an identity file path
// containing spaces (legitimate on Windows) must not be rejected.
func TestValidateSSH_AllowsLegitimateValues(t *testing.T) {
	for _, p := range []Profile{
		{Name: "ip", Type: ProfileTypeSSH, Mode: SSHModeServe, Host: "198.51.100.10", User: "ubuntu"},
		{Name: "host", Type: ProfileTypeSSH, Mode: SSHModeServe, Host: "ubuntu@example.com"},
		{Name: "bin", Type: ProfileTypeSSH, Mode: SSHModeServe, Host: "h", Binary: "many-ai-cli"},
		{Name: "key", Type: ProfileTypeSSH, Mode: SSHModeServe, Host: "h", IdentityFile: `C:\Users\My Name\.ssh\key.pem`},
	} {
		pf := &ProfilesFile{Version: 1, Profiles: []Profile{p}}
		if err := Validate(pf); err != nil {
			t.Errorf("profile %q unexpectedly rejected: %v", p.Name, err)
		}
	}
}

// --- cleanupSSHOrphans pattern: regexp.QuoteMeta + ShellQuote layering ---
//
// The cleanup pkill pattern must (a) treat regex metacharacters in the binary
// literally (QuoteMeta) so pkill -f's ERE cannot widen the match, and (b) be a
// single shell token after ssh re-joins the remote command, so a binary with
// shell metacharacters cannot inject a separate remote command.
func TestCleanupOrphanPatternQuoting(t *testing.T) {
	// Mirror the construction in cleanupSSHOrphans.
	binary := "my.bin; touch /tmp/pwn #"
	quotedMeta := regexp.QuoteMeta(binary)

	// QuoteMeta must escape the regex-significant '.' so it is literal in ERE.
	if !strings.Contains(quotedMeta, `\.`) {
		t.Errorf("QuoteMeta did not escape '.': %q", quotedMeta)
	}

	pattern := quotedMeta + " serve --port 47777"
	shellArg := ShellQuote(pattern)

	// ShellQuote must wrap the whole pattern in single quotes so embedded shell
	// metacharacters (';', '#', spaces) cannot break out as a separate command.
	if !strings.HasPrefix(shellArg, "'") || !strings.HasSuffix(shellArg, "'") {
		t.Errorf("ShellQuote did not single-quote the pattern: %q", shellArg)
	}
	if !strings.Contains(shellArg, "touch /tmp/pwn") {
		t.Errorf("expected payload to remain quoted inside the pattern: %q", shellArg)
	}
}
