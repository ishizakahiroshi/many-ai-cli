package launcher

import (
	"os"
	"path/filepath"
	"testing"
)

// setupTempHome creates a temporary home directory with a .any-ai-cli subdir,
// sets HOME (and USERPROFILE on Windows) to it, and returns a cleanup func.
func setupTempHome(t *testing.T) (string, func()) {
	t.Helper()
	tmp := t.TempDir()
	aacDir := filepath.Join(tmp, ".any-ai-cli")
	if err := os.MkdirAll(aacDir, 0o700); err != nil {
		t.Fatal(err)
	}
	origHome := os.Getenv("HOME")
	origUserProfile := os.Getenv("USERPROFILE")
	os.Setenv("HOME", tmp)
	os.Setenv("USERPROFILE", tmp)
	cleanup := func() {
		os.Setenv("HOME", origHome)
		os.Setenv("USERPROFILE", origUserProfile)
	}
	return tmp, cleanup
}

// --- LoadProfiles / SaveProfiles round-trip ---

func TestLoadSaveRoundTrip(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	pf := &ProfilesFile{
		Version:  1,
		LastUsed: "sakura-vps",
		Profiles: []Profile{
			{
				Name:    "WSL (Ubuntu)",
				Type:    ProfileTypeWSL,
				Distro:  "",
				Binary:  "any-ai-cli",
				CWD:     "~",
				HubPort: 0,
			},
			{
				Name:     "sakura-vps",
				Type:     ProfileTypeSSH,
				Mode:     SSHModeServe,
				Host:     "153.127.0.1",
				User:     "ubuntu",
				SSHPort:  10022,
				Binary:   "any-ai-cli",
				HubPort:  47777,
			},
		},
	}

	if err := SaveProfiles(pf); err != nil {
		t.Fatalf("SaveProfiles: %v", err)
	}

	loaded, err := LoadProfiles()
	if err != nil {
		t.Fatalf("LoadProfiles: %v", err)
	}

	if loaded.Version != pf.Version {
		t.Errorf("Version: got %d, want %d", loaded.Version, pf.Version)
	}
	if loaded.LastUsed != pf.LastUsed {
		t.Errorf("LastUsed: got %q, want %q", loaded.LastUsed, pf.LastUsed)
	}
	if len(loaded.Profiles) != len(pf.Profiles) {
		t.Fatalf("len(Profiles): got %d, want %d", len(loaded.Profiles), len(pf.Profiles))
	}
	for i, want := range pf.Profiles {
		got := loaded.Profiles[i]
		if got.Name != want.Name {
			t.Errorf("Profiles[%d].Name: got %q, want %q", i, got.Name, want.Name)
		}
		if got.Type != want.Type {
			t.Errorf("Profiles[%d].Type: got %q, want %q", i, got.Type, want.Type)
		}
		if got.HubPort != want.HubPort {
			t.Errorf("Profiles[%d].HubPort: got %d, want %d", i, got.HubPort, want.HubPort)
		}
	}
}

// --- File-not-found / empty file ---

func TestLoadProfiles_FileNotExist(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	pf, err := LoadProfiles()
	if err != nil {
		t.Fatalf("LoadProfiles on missing file: %v", err)
	}
	if pf.Version != 1 {
		t.Errorf("Version: got %d, want 1", pf.Version)
	}
	if len(pf.Profiles) != 0 {
		t.Errorf("expected empty Profiles, got %d", len(pf.Profiles))
	}
}

func TestLoadProfiles_EmptyFile(t *testing.T) {
	tmpHome, cleanup := setupTempHome(t)
	defer cleanup()

	path := filepath.Join(tmpHome, ".any-ai-cli", profilesFile)
	if err := os.WriteFile(path, []byte(""), 0o600); err != nil {
		t.Fatal(err)
	}

	pf, err := LoadProfiles()
	if err != nil {
		t.Fatalf("LoadProfiles on empty file: %v", err)
	}
	if pf.Version != 1 {
		t.Errorf("Version: got %d, want 1", pf.Version)
	}
}

// --- last_used pointing to non-existent profile is tolerated ---

func TestLoadProfiles_LastUsedMissing(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	pf := &ProfilesFile{
		Version:  1,
		LastUsed: "does-not-exist",
		Profiles: []Profile{
			{Name: "wsl-local", Type: ProfileTypeWSL},
		},
	}
	if err := SaveProfiles(pf); err != nil {
		t.Fatal(err)
	}

	loaded, err := LoadProfiles()
	if err != nil {
		t.Fatalf("LoadProfiles: %v", err)
	}
	if loaded.LastUsed != "does-not-exist" {
		t.Errorf("LastUsed should be preserved even if name not in profiles")
	}
}

// --- Unknown version is an error ---

func TestLoadProfiles_UnknownVersion(t *testing.T) {
	tmpHome, cleanup := setupTempHome(t)
	defer cleanup()

	path := filepath.Join(tmpHome, ".any-ai-cli", profilesFile)
	if err := os.WriteFile(path, []byte("version: 999\nprofiles: []\n"), 0o600); err != nil {
		t.Fatal(err)
	}

	_, err := LoadProfiles()
	if err == nil {
		t.Fatal("expected error for unknown version, got nil")
	}
}

// --- Validate: valid profiles pass ---

func TestValidate_Valid(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{
			{Name: "wsl-local", Type: ProfileTypeWSL},
			{Name: "vps-serve", Type: ProfileTypeSSH, Mode: SSHModeServe, HubPort: 47777},
			{
				Name:         "vps-tunnel",
				Type:         ProfileTypeSSH,
				Mode:         SSHModeTunnel,
				HubPort:      47801,
				TokenCommand: "echo token123",
			},
		},
	}
	if err := Validate(pf); err != nil {
		t.Errorf("unexpected error: %v", err)
	}
}

// --- Validate: duplicate name ---

func TestValidate_DuplicateName(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{
			{Name: "dup", Type: ProfileTypeWSL},
			{Name: "dup", Type: ProfileTypeSSH, Mode: SSHModeServe},
		},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for duplicate name, got nil")
	}
}

// --- Validate: empty name ---

func TestValidate_EmptyName(t *testing.T) {
	pf := &ProfilesFile{
		Version:  1,
		Profiles: []Profile{{Name: "", Type: ProfileTypeWSL}},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for empty name, got nil")
	}
}

// --- Validate: unknown type ---

func TestValidate_UnknownType(t *testing.T) {
	pf := &ProfilesFile{
		Version:  1,
		Profiles: []Profile{{Name: "p1", Type: "docker"}},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for unknown type, got nil")
	}
}

// --- Validate: mode on WSL profile is rejected ---

func TestValidate_ModeOnWSL(t *testing.T) {
	pf := &ProfilesFile{
		Version:  1,
		Profiles: []Profile{{Name: "p1", Type: ProfileTypeWSL, Mode: SSHModeServe}},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for mode on wsl profile, got nil")
	}
}

// --- Validate: unknown SSH mode ---

func TestValidate_UnknownMode(t *testing.T) {
	pf := &ProfilesFile{
		Version:  1,
		Profiles: []Profile{{Name: "p1", Type: ProfileTypeSSH, Mode: "direct", HubPort: 47777}},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for unknown mode, got nil")
	}
}

// --- Validate: hub_port out of range ---

func TestValidate_HubPortOutOfRange(t *testing.T) {
	pf := &ProfilesFile{
		Version:  1,
		Profiles: []Profile{{Name: "p1", Type: ProfileTypeWSL, HubPort: 99999}},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for hub_port out of range, got nil")
	}
}

// --- Validate: tunnel with hub_port=0 is rejected ---

func TestValidate_TunnelHubPortZero(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name:         "p1",
			Type:         ProfileTypeSSH,
			Mode:         SSHModeTunnel,
			HubPort:      0,
			TokenCommand: "echo token",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for tunnel hub_port=0, got nil")
	}
}

// --- Validate: tunnel with empty token_command is rejected ---

func TestValidate_TunnelEmptyTokenCommand(t *testing.T) {
	pf := &ProfilesFile{
		Version: 1,
		Profiles: []Profile{{
			Name:         "p1",
			Type:         ProfileTypeSSH,
			Mode:         SSHModeTunnel,
			HubPort:      47801,
			TokenCommand: "",
		}},
	}
	if err := Validate(pf); err == nil {
		t.Error("expected error for tunnel with empty token_command, got nil")
	}
}

// --- normalizeProfile: "user@host" parsing ---

func TestNormalizeProfile_UserAtHost(t *testing.T) {
	p := Profile{
		Name: "vps",
		Type: ProfileTypeSSH,
		Host: "ubuntu@153.127.0.1",
		User: "old-user",
	}
	normalizeProfile(&p)
	if p.User != "ubuntu" {
		t.Errorf("User: got %q, want %q", p.User, "ubuntu")
	}
	if p.Host != "153.127.0.1" {
		t.Errorf("Host: got %q, want %q", p.Host, "153.127.0.1")
	}
}

func TestNormalizeProfile_NoAt(t *testing.T) {
	p := Profile{
		Name: "vps",
		Type: ProfileTypeSSH,
		Host: "153.127.0.1",
		User: "ubuntu",
	}
	normalizeProfile(&p)
	if p.User != "ubuntu" {
		t.Errorf("User should be unchanged: got %q", p.User)
	}
	if p.Host != "153.127.0.1" {
		t.Errorf("Host should be unchanged: got %q", p.Host)
	}
}

// normalizeProfile should not touch WSL profiles.
func TestNormalizeProfile_WSLIgnored(t *testing.T) {
	p := Profile{
		Name: "wsl",
		Type: ProfileTypeWSL,
		Host: "ubuntu@somehost", // unusual but should not be parsed for WSL
	}
	normalizeProfile(&p)
	if p.Host != "ubuntu@somehost" {
		t.Errorf("WSL Host should not be modified, got %q", p.Host)
	}
}
