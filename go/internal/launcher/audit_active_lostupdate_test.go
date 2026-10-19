package launcher

import (
	"fmt"
	"sync"
	"testing"
)

// TestRegisterActiveConnectionConcurrentNoLostUpdate is a regression guard for
// the in-process lost-update race on launcher-active.json. Several goroutines
// register distinct profiles at the same time; the load→modify→save sequence is
// not transactional on its own (saveActiveFile only makes each individual write
// atomic via temp+rename), so without activeFileMu serializing the whole
// region, concurrent goroutines read the same state and clobber each other's
// append — some profiles silently vanish. With the mutex, every registration
// must survive. Run with -race to also flag the underlying data race.
func TestRegisterActiveConnectionConcurrentNoLostUpdate(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	const n = 16
	var wg sync.WaitGroup
	errs := make([]error, n)
	wg.Add(n)
	for i := 0; i < n; i++ {
		go func(i int) {
			defer wg.Done()
			profile := fmt.Sprintf("p%02d", i)
			hubURL := fmt.Sprintf("http://127.0.0.1:%d/?token=t%02d", 47777+i, i)
			errs[i] = RegisterActiveConnection(profile, hubURL)
		}(i)
	}
	wg.Wait()

	for i, err := range errs {
		if err != nil {
			t.Fatalf("RegisterActiveConnection #%d: %v", i, err)
		}
	}

	// All registrations share this test process's PID but use distinct
	// profiles, so every one is a separate entry that must persist.
	got, err := collectActive(alwaysAlive, alwaysOK)
	if err != nil {
		t.Fatalf("collectActive: %v", err)
	}
	if len(got) != n {
		t.Fatalf("len: got %d, want %d (lost update dropped entries)", len(got), n)
	}
	seen := make(map[string]bool, n)
	for _, c := range got {
		seen[c.Profile] = true
	}
	for i := 0; i < n; i++ {
		profile := fmt.Sprintf("p%02d", i)
		if !seen[profile] {
			t.Errorf("profile %q missing from active connections", profile)
		}
	}
}

// TestRegisterAndCollectActiveConcurrentRMW exercises the cross-function race
// described in the finding: one set of goroutines registers connections while
// another set runs collectActive (which itself does a load→prune→save). All
// guards report alive/OK so collectActive must never drop a live entry. The
// activeFileMu lock is what keeps these two read-modify-write paths from
// interleaving and losing writes; -race surfaces any regression.
func TestRegisterAndCollectActiveConcurrentRMW(t *testing.T) {
	_, cleanup := setupTempHome(t)
	defer cleanup()

	const n = 12
	var wg sync.WaitGroup
	wg.Add(2 * n)
	for i := 0; i < n; i++ {
		go func(i int) {
			defer wg.Done()
			profile := fmt.Sprintf("c%02d", i)
			hubURL := fmt.Sprintf("http://127.0.0.1:%d/?token=t%02d", 48000+i, i)
			if err := RegisterActiveConnection(profile, hubURL); err != nil {
				t.Errorf("RegisterActiveConnection #%d: %v", i, err)
			}
		}(i)
		go func() {
			defer wg.Done()
			// Interleave reads that also rewrite the file when pruning.
			if _, err := collectActive(alwaysAlive, alwaysOK); err != nil {
				t.Errorf("collectActive: %v", err)
			}
		}()
	}
	wg.Wait()

	got, err := collectActive(alwaysAlive, alwaysOK)
	if err != nil {
		t.Fatalf("final collectActive: %v", err)
	}
	if len(got) != n {
		t.Fatalf("len: got %d, want %d (concurrent RMW lost entries)", len(got), n)
	}
}
