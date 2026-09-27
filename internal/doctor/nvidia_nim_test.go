package doctor

import (
	"strings"
	"testing"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/nvidianim"
)

func TestNVIDIANIMDoctorReportsDisabledWithoutReadingKey(t *testing.T) {
	const key = "synthetic-doctor-key"
	t.Setenv(nvidianim.APIKeyEnv, key)
	cfg := &config.Config{}

	check := nvidiaNIM(cfg)

	if check.Level != OK || !strings.Contains(check.Message, "DISABLED") {
		t.Fatalf("NVIDIA NIM check = %+v, want disabled status", check)
	}
	if strings.Contains(check.Message, key) || strings.Contains(check.Fix, key) {
		t.Fatal("Doctor exposed the NVIDIA API key while the route was disabled")
	}
}

func TestNVIDIANIMDoctorReportsMissingKeyWithoutNetworkCheck(t *testing.T) {
	home := t.TempDir()
	t.Setenv("USERPROFILE", home)
	t.Setenv("HOME", home)
	t.Setenv(nvidianim.APIKeyEnv, "")
	cfg := &config.Config{}
	cfg.NVIDIANIM.Enabled = true

	check := nvidiaNIM(cfg)

	if check.Level != Warn || !strings.Contains(check.Message, "NOT CONFIGURED") {
		t.Fatalf("NVIDIA NIM check = %+v, want not-configured status", check)
	}
}

func TestNVIDIANIMDoctorDoesNotClaimConnectivityOrExposeConfiguredKey(t *testing.T) {
	home := t.TempDir()
	t.Setenv("USERPROFILE", home)
	t.Setenv("HOME", home)
	const key = "synthetic-doctor-key"
	t.Setenv(nvidianim.APIKeyEnv, key)
	cfg := &config.Config{}
	cfg.NVIDIANIM.Enabled = true

	check := nvidiaNIM(cfg)

	if check.Level != Warn || !strings.Contains(check.Message, "not checked") {
		t.Fatalf("NVIDIA NIM check = %+v, want configured but untested status", check)
	}
	if check.Fix == "" || !strings.Contains(check.Fix, "/v1/models") {
		t.Fatalf("NVIDIA NIM fix = %q, want the safe /v1/models connection-test instruction", check.Fix)
	}
	if strings.Contains(check.Message, key) || strings.Contains(check.Fix, key) {
		t.Fatal("Doctor exposed the configured NVIDIA API key")
	}
}
