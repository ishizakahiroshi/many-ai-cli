//go:build windows && many_ai_v01_oracle

// This test belongs only to a privately materialized fault variant of the fixed
// Go headless package. Its one changed assignment call invokes the hook below.
// Repository Go sources and the fixed oracle are untouched. The Rust supervisor
// owns the private Job and every termination handle used by this fixture.
package headless

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync/atomic"
	"syscall"
	"testing"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

const v01GoSource = "d8fbf8598c3effd4e2f837e43ad6f0488c461de3"

var v01AttachmentCalls atomic.Uint32
var v01NativeAssignmentCalls atomic.Uint32
var v01AttachmentError atomic.Uint32

// v01FaultAssignProcessToJobObject is substituted for exactly one call in the
// private proc_windows.go copy. The missing JOB_OBJECT_ASSIGN_PROCESS right is
// deliberate fault injection; AssignProcessToJobObject itself is the real API.
// The original attachProcessJob retains and cleans up its full-access handles.
func v01FaultAssignProcessToJobObject(job, process windows.Handle) error {
	v01AttachmentCalls.Add(1)
	const jobObjectQuery = 0x0004 // JOB_OBJECT_QUERY; no assignment access.
	var queryJob windows.Handle
	if err := windows.DuplicateHandle(windows.CurrentProcess(), job,
		windows.CurrentProcess(), &queryJob, jobObjectQuery, false, 0); err != nil {
		return err
	}
	defer windows.CloseHandle(queryJob)
	v01NativeAssignmentCalls.Add(1)
	err := windows.AssignProcessToJobObject(queryJob, process)
	var nativeErr syscall.Errno
	if errors.As(err, &nativeErr) {
		v01AttachmentError.Store(uint32(nativeErr))
	}
	return err
}

type v01RunReply struct {
	result         Result
	err            error
	elapsed        time.Duration
	cleanupStarted bool
}

// TestV01WindowsOracle runs the unchanged Run body in the private fault variant.
// Its actual first attachment call records a native failure through the hook;
// this does not demonstrate natural failure in unmodified Go or Windows.
func TestV01WindowsOracle(t *testing.T) {
	root := os.Getenv("MANY_AI_V01_ROOT")
	if root == "" {
		t.Fatal("V01 oracle requires its isolated native supervisor")
	}
	if runtime.Version() != "go1.26.8" || runtime.GOOS != "windows" || runtime.GOARCH != "amd64" {
		t.Fatal("V01 oracle requires native Go 1.26.8 windows/amd64")
	}
	if !filepath.IsAbs(root) {
		t.Fatal("V01 root must be an absolute private fixture directory")
	}
	if info, err := os.Stat(root); err != nil || !info.IsDir() {
		t.Fatal("V01 root is not an existing directory")
	}
	fixture := os.Getenv("MANY_AI_V01_FIXTURE")
	if !filepath.IsAbs(fixture) {
		t.Fatal("V01 provider must be the absolute Rust fixture executable")
	}
	if info, err := os.Stat(fixture); err != nil || !info.Mode().IsRegular() {
		t.Fatal("V01 provider executable does not exist")
	}
	fixtureTest := os.Getenv("MANY_AI_V01_FIXTURE_TEST")
	if fixtureTest != "orchestration::headless::windows_contracts::native_headless_fixture" {
		t.Fatal("V01 provider test must be the fixed native Rust library fixture")
	}
	caseName := os.Getenv("MANY_AI_V01_CASE")
	if caseName != "cancel" && caseName != "deadline" {
		t.Fatal("V01 oracle case must be cancel or deadline")
	}
	timeoutMS, err := strconv.ParseInt(os.Getenv("MANY_AI_V01_TIMEOUT_MS"), 10, 64)
	if err != nil || timeoutMS < 1000 || timeoutMS > 30000 {
		t.Fatal("V01 timeout must be between 1000 and 30000 milliseconds")
	}
	timeout := time.Duration(timeoutMS) * time.Millisecond
	// The supervisor first associates this process with its private Job. No
	// descendants are admitted before that association has succeeded.
	var admission [1]byte
	admitted := make(chan error, 1)
	go func() {
		_, err := io.ReadFull(os.Stdin, admission[:])
		admitted <- err
	}()
	select {
	case err := <-admitted:
		if err != nil || admission[0] != 'G' {
			t.Fatal("missing private-Job admission byte")
		}
	case <-time.After(20 * time.Second):
		t.Fatal("private-Job admission was not received within its setup bound")
	}
	if v01AttachmentCalls.Load() != 0 || v01NativeAssignmentCalls.Load() != 0 {
		t.Fatal("V01 fault variant must start with no attachment attempts")
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	var stdoutReady, stderrReady, stdoutRetained, stderrRetained atomic.Bool
	callbackErrors := make(chan error, 1)
	reportCallbackError := func(err error) {
		if err != nil {
			select {
			case callbackErrors <- err:
			default:
			}
		}
	}
	emit := func(data []byte) {
		line := string(data)
		if strings.Contains(line, "V01_STDOUT_READY") {
			stdoutReady.Store(true)
		}
		if strings.Contains(line, "V01_STDERR_READY") {
			stderrReady.Store(true)
		}
		if strings.Contains(line, "V01_STDOUT_RETAINED") && stdoutRetained.CompareAndSwap(false, true) {
			reportCallbackError(v01Publish(root, "go-stdout-retained", []byte("observed by private Go fault-variant Run callback\n")))
		}
		if strings.Contains(line, "V01_STDERR_RETAINED") && stderrRetained.CompareAndSwap(false, true) {
			reportCallbackError(v01Publish(root, "go-stderr-retained", []byte("observed by private Go fault-variant Run callback\n")))
		}
	}
	spec := Spec{
		Exe:       fixture,
		Argv:      []string{"--exact", fixtureTest, "--nocapture"},
		PromptVia: "arg",
		Format:    "text",
		CWD:       root,
		Env:       v01ReplaceEnv(os.Environ(), "MANY_AI_V01_ROLE", "direct"),
	}
	if caseName == "deadline" {
		spec.Timeout = timeout
	}
	replies := make(chan v01RunReply, 1)
	started := time.Now()
	go func() {
		result, err := Run(ctx, spec, emit)
		elapsed := time.Since(started)
		cleanupStarted := v01Exists(root, "harness-cleanup")
		// Expose actual return before any handle waits or final receipt work.
		// Their latency must never look like Run still being blocked.
		reportCallbackError(v01Publish(root, "go-run-returned", []byte("actual private Go fault-variant Run returned\n")))
		replies <- v01RunReply{result: result, err: err, elapsed: elapsed, cleanupStarted: cleanupStarted}
	}()

	// Readiness must precede cancellation/the actual configured deadline. A
	// deadline that kills the shim before it starts its child is not V01 evidence.
	readyBound := 20 * time.Second
	if caseName == "deadline" {
		readyBound = timeout
	}
	readyUntil := started.Add(readyBound)
	for !stdoutReady.Load() || !stderrReady.Load() {
		select {
		case reply := <-replies:
			t.Fatalf("Run returned before the compound fixture was ready: %+v, %v", reply.result, reply.err)
		case err := <-callbackErrors:
			t.Fatal(err)
		default:
		}
		if !time.Now().Before(readyUntil) {
			t.Fatal("both native descendant streams were not ready before the setup deadline")
		}
		time.Sleep(5 * time.Millisecond)
	}
	directPID := v01ReadPID(t, root, "direct.pid")
	grandchildPID := v01ReadPID(t, root, "grandchild.pid")
	if directPID == grandchildPID || directPID == uint32(os.Getpid()) || grandchildPID == uint32(os.Getpid()) {
		t.Fatal("fixture process identities are not distinct")
	}
	direct := v01OpenLiveProcess(t, directPID)
	defer windows.CloseHandle(direct)
	grandchild := v01OpenLiveProcess(t, grandchildPID)
	defer windows.CloseHandle(grandchild)
	if err := v01VerifyCurrentJobMembers(uint32(os.Getpid()), directPID, grandchildPID); err != nil {
		t.Fatal(err)
	}

	// Run starts its reader callbacks only after the first attachment attempt.
	// Therefore both READY callbacks also establish that the actual injected
	// native assignment has returned, without a second diagnostic attempt.
	v01RequireNativeFault(t)
	if v01ProcessExited(t, direct) || v01ProcessExited(t, grandchild) {
		t.Fatal("compound fixture exited before its native attachment failure was recorded")
	}
	readyElapsed := time.Since(started)
	if readyElapsed >= readyBound {
		t.Fatal("native attachment failure was not recorded before the readiness deadline")
	}
	ready := map[string]any{
		"schema": 1, "go_source_sha": v01GoSource, "go_version": runtime.Version(), "case": caseName,
		"direct_pid": directPID, "grandchild_pid": grandchildPID,
		"variant":                 "private_go_attachment_fault_variant",
		"native_attachment_error": v01AttachmentError.Load(), "attachment_calls": v01AttachmentCalls.Load(),
		"native_assignment_calls":                     v01NativeAssignmentCalls.Load(),
		"original_run_attach_error_directly_observed": true, "attachment_fault_injected": true,
		"stdout_ready": stdoutReady.Load(), "stderr_ready": stderrReady.Load(),
		"run_start_elapsed_ms": readyElapsed.Milliseconds(), "deadline_ms": spec.Timeout.Milliseconds(),
	}
	v01PublishJSON(t, root, "go-ready.json", ready)

	// The external supervisor retains both handles before requesting cancel.
	// In the deadline case, Run's own unchanged timer performs cancellation.
	if caseName == "cancel" {
		until := time.Now().Add(20 * time.Second)
		for !v01Exists(root, "cancel-run") {
			select {
			case reply := <-replies:
				t.Fatalf("Run returned before supervisor cancellation: %+v, %v", reply.result, reply.err)
			case err := <-callbackErrors:
				t.Fatal(err)
			default:
			}
			if !time.Now().Before(until) {
				t.Fatal("supervisor did not request cancellation")
			}
			time.Sleep(5 * time.Millisecond)
		}
		cancel()
	}

	// A bounded test watchdog is failure containment, not the application's
	// deadline. The supervisor observes Run pending, then releases only its
	// retained grandchild handle. Its Job remains the final failure guard.
	var reply v01RunReply
	select {
	case reply = <-replies:
	case err := <-callbackErrors:
		t.Fatal(err)
	case <-time.After(timeout + 15*time.Second):
		t.Fatal("supervisor did not release the owned retained-pipe fixture")
	}
	// TerminateProcess is asynchronous. EOF may arrive just before the native
	// process object signals; retain the identities and bound that final wait.
	directExited := v01WaitProcessExited(t, direct)
	grandchildExited := v01WaitProcessExited(t, grandchild)
	v01RequireNativeFault(t)
	result := map[string]any{
		"schema": 1, "go_source_sha": v01GoSource, "go_version": runtime.Version(), "case": caseName,
		"variant":                 "private_go_attachment_fault_variant",
		"native_attachment_error": v01AttachmentError.Load(), "attachment_calls": v01AttachmentCalls.Load(),
		"native_assignment_calls":                     v01NativeAssignmentCalls.Load(),
		"original_run_attach_error_directly_observed": true, "attachment_fault_injected": true,
		"state": reply.result.State, "exit_code": reply.result.ExitCode,
		"timed_out": reply.result.TimedOut, "canceled": reply.result.Canceled,
		"run_return_ms":            reply.elapsed.Milliseconds(),
		"cleanup_release_observed": reply.cleanupStarted,
		"direct_exited":            directExited, "grandchild_exited": grandchildExited,
		"error": "",
	}
	if reply.err != nil {
		result["error"] = reply.err.Error()
	}
	v01PublishJSON(t, root, "go-result.json", result)
	if reply.err != nil || reply.result.State != "error" {
		t.Fatalf("unexpected actual Go Run result: %+v, %v", reply.result, reply.err)
	}
	if reply.result.TimedOut != (caseName == "deadline") || reply.result.Canceled != (caseName == "cancel") {
		t.Fatalf("actual Go cancellation/timeout classification differs: %+v", reply.result)
	}
	if !stdoutRetained.Load() || !stderrRetained.Load() || !result["cleanup_release_observed"].(bool) {
		t.Fatal("Run returned without the complete retained-pipes observation and harness release")
	}
	if !result["direct_exited"].(bool) || !result["grandchild_exited"].(bool) {
		t.Fatal("an owned native process is still running after Go Run returned")
	}
	select {
	case err := <-callbackErrors:
		t.Fatal(err)
	default:
	}
	// Return normally so the supervisor verifies actual test-process teardown.
}

func v01RequireNativeFault(t *testing.T) {
	t.Helper()
	if v01AttachmentCalls.Load() != 1 || v01NativeAssignmentCalls.Load() != 1 ||
		v01AttachmentError.Load() != uint32(windows.ERROR_ACCESS_DENIED) {
		t.Fatalf("private fault variant requires one actual native access-denied assignment: hook_calls=%d, native_calls=%d, error=%d",
			v01AttachmentCalls.Load(), v01NativeAssignmentCalls.Load(), v01AttachmentError.Load())
	}
}

func v01VerifyCurrentJobMembers(pids ...uint32) error {
	// Only three fixture processes are expected. A larger/foreign Job is a
	// setup error; never resize this buffer to inspect an arbitrary host Job.
	var list struct {
		assigned uint32
		listed   uint32
		pids     [16]uintptr
	}
	if err := windows.QueryInformationJobObject(0, windows.JobObjectBasicProcessIdList,
		uintptr(unsafe.Pointer(&list)), uint32(unsafe.Sizeof(list)), nil); err != nil {
		return fmt.Errorf("query private fixture Job members: %w", err)
	}
	if list.assigned != uint32(len(pids)) || list.listed != uint32(len(pids)) {
		return errors.New("private fixture Job does not contain exactly the three owned processes")
	}
	for _, pid := range pids {
		found := false
		for _, member := range list.pids[:list.listed] {
			found = found || member == uintptr(pid)
		}
		if !found {
			return errors.New("retained fixture process is outside the current private Job")
		}
	}
	return nil
}

func v01ReadPID(t *testing.T, root, name string) uint32 {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(root, name))
	if err != nil {
		t.Fatalf("read owned fixture PID: %v", err)
	}
	pid, err := strconv.ParseUint(strings.TrimSpace(string(data)), 10, 32)
	if err != nil || pid == 0 {
		t.Fatal("invalid owned fixture PID")
	}
	return uint32(pid)
}

func v01OpenLiveProcess(t *testing.T, pid uint32) windows.Handle {
	t.Helper()
	handle, err := windows.OpenProcess(windows.PROCESS_QUERY_LIMITED_INFORMATION|windows.SYNCHRONIZE, false, pid)
	if err != nil {
		t.Fatalf("open owned fixture process: %v", err)
	}
	if v01ProcessExited(t, handle) {
		windows.CloseHandle(handle)
		t.Fatal("owned fixture process exited before its handle was retained")
	}
	return handle
}

func v01ProcessExited(t *testing.T, handle windows.Handle) bool {
	t.Helper()
	status, err := windows.WaitForSingleObject(handle, 0)
	if err != nil {
		t.Fatalf("wait on retained fixture handle: %v", err)
	}
	if status != windows.WAIT_OBJECT_0 && status != uint32(windows.WAIT_TIMEOUT) {
		t.Fatalf("unexpected retained fixture handle wait status: %d", status)
	}
	return status == windows.WAIT_OBJECT_0
}

func v01WaitProcessExited(t *testing.T, handle windows.Handle) bool {
	t.Helper()
	status, err := windows.WaitForSingleObject(handle, 2000)
	if err != nil {
		t.Fatalf("wait for owned native process cleanup: %v", err)
	}
	if status != windows.WAIT_OBJECT_0 && status != uint32(windows.WAIT_TIMEOUT) {
		t.Fatalf("unexpected owned native cleanup wait status: %d", status)
	}
	return status == windows.WAIT_OBJECT_0
}

func v01ReplaceEnv(env []string, key, value string) []string {
	result := make([]string, 0, len(env)+1)
	for _, pair := range env {
		name, _, _ := strings.Cut(pair, "=")
		if !strings.EqualFold(name, key) {
			result = append(result, pair)
		}
	}
	return append(result, key+"="+value)
}

func v01Exists(root, name string) bool {
	_, err := os.Stat(filepath.Join(root, name))
	return err == nil
}

func v01Publish(root, name string, data []byte) error {
	temporary := filepath.Join(root, name+".tmp")
	if err := os.WriteFile(temporary, data, 0600); err != nil {
		return err
	}
	return os.Rename(temporary, filepath.Join(root, name))
}

func v01PublishJSON(t *testing.T, root, name string, value any) {
	t.Helper()
	data, err := json.MarshalIndent(value, "", "  ")
	if err == nil {
		err = v01Publish(root, name, append(data, '\n'))
	}
	if err != nil {
		t.Fatalf("publish synthetic oracle receipt: %v", err)
	}
}
