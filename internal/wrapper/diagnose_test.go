package wrapper

import (
	"bytes"
	"errors"
	"os/exec"
	"strings"
	"testing"
)

func TestClassifyStartFailure_ExecNotFound(t *testing.T) {
	wrapped := &exec.Error{Name: "codex", Err: exec.ErrNotFound}
	if got := classifyStartFailure(wrapped); got != "exec_not_found" {
		t.Fatalf("classifyStartFailure(exec.ErrNotFound) = %q, want %q", got, "exec_not_found")
	}
}

func TestClassifyStartFailure_TextualNotFound(t *testing.T) {
	err := errors.New(`exec: "C:\\dev\\any-ai-cli\\codex": executable file not found in %PATH%`)
	if got := classifyStartFailure(err); got != "exec_not_found" {
		t.Fatalf("classifyStartFailure(textual) = %q, want %q", got, "exec_not_found")
	}
}

func TestClassifyStartFailure_OtherError(t *testing.T) {
	if got := classifyStartFailure(errors.New("ConPTY init failed: 0x80070005")); got != "" {
		t.Fatalf("classifyStartFailure(other) = %q, want empty", got)
	}
}

func TestClassifyStartFailure_Nil(t *testing.T) {
	if got := classifyStartFailure(nil); got != "" {
		t.Fatalf("classifyStartFailure(nil) = %q, want empty", got)
	}
}

func TestDiagnoseStartFailure_IncludesHintForExecNotFound(t *testing.T) {
	var buf bytes.Buffer
	err := &exec.Error{Name: "codex", Err: exec.ErrNotFound}
	diagnoseStartFailure(&buf, "codex", []string{"--ask-for-approval", "on-request"}, err)
	out := buf.String()
	for _, want := range []string{
		"any-ai-cli spawn diagnostic",
		"provider: codex",
		"PATH entries:",
		"Hint:",
		"any-ai-cli stop && any-ai-cli codex",
	} {
		if !strings.Contains(out, want) {
			t.Fatalf("diagnostic output missing %q\n---\n%s", want, out)
		}
	}
}

func TestDiagnoseStartFailure_NoHintForUnrelatedError(t *testing.T) {
	var buf bytes.Buffer
	err := errors.New("ConPTY init failed: 0x80070005")
	diagnoseStartFailure(&buf, "claude", nil, err)
	out := buf.String()
	if strings.Contains(out, "Hint:") {
		t.Fatalf("unexpected Hint section for unrelated error\n---\n%s", out)
	}
	if !strings.Contains(out, "any-ai-cli spawn diagnostic") {
		t.Fatalf("missing diagnostic header\n---\n%s", out)
	}
}
