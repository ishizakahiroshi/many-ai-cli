package hub

import (
	"context"
	"errors"
	"net/http"
	"path/filepath"
	"testing"
	"time"

	webpush "github.com/SherClockHolmes/webpush-go"
)

// auditPushStubHTTPClient is a webpush.HTTPClient whose Do can be made to
// succeed (status code) or fail (transport error) on demand.
type auditPushStubHTTPClient struct {
	status int
	err    error
	calls  int
}

func (c *auditPushStubHTTPClient) Do(*http.Request) (*http.Response, error) {
	c.calls++
	if c.err != nil {
		return nil, c.err
	}
	return &http.Response{StatusCode: c.status, Body: http.NoBody}, nil
}

// auditPushTestSubscription returns a subscription with a valid (public test
// vector) P256dh/Auth pair so that webpush payload encryption succeeds and the
// stub HTTPClient.Do is actually reached. These are sample public keys, not
// secrets.
func auditPushTestSubscription() pushSubscription {
	return pushSubscription{
		Endpoint: "https://updates.push.services.mozilla.com/wpush/v2/gAAAAA",
		Keys: webpush.Keys{
			P256dh: "BNNL5ZaTfK81qhXOx23-wewhigUeFb632jN6LvRWCFH1ubQr77FE_9qV1FuojuRmHP42zmf34rXgW80OvUVDgTk",
			Auth:   "zqbxT6JKstKSY9JKibZLSQ",
		},
	}
}

func auditPushNewManager(t *testing.T, client webpush.HTTPClient) *pushManager {
	t.Helper()
	priv, pub, err := webpush.GenerateVAPIDKeys()
	if err != nil {
		t.Fatalf("GenerateVAPIDKeys: %v", err)
	}
	return &pushManager{
		path: filepath.Join(t.TempDir(), pushStoreFilename),
		store: pushStoreFile{
			VAPIDPublicKey:  pub,
			VAPIDPrivateKey: priv,
			Subscriptions:   []pushSubscription{auditPushTestSubscription()},
		},
		httpClient: client,
		sent:       map[string]time.Time{},
	}
}

// TestSendApprovalDoesNotDedupWhenAllSendsFail verifies finding #30: when every
// subscription send fails (e.g. transient network outage), the approval ID must
// NOT be recorded in pm.sent, so a subsequent call with the same ID can retry.
func TestSendApprovalDoesNotDedupWhenAllSendsFail(t *testing.T) {
	client := &auditPushStubHTTPClient{err: errors.New("network down")}
	pm := auditPushNewManager(t, client)

	const id = "approval-xyz"
	pm.sendApproval(context.Background(), pushApprovalPayload{ID: id, Body: "approve?"})

	pm.mu.Lock()
	_, marked := pm.sent[id]
	pm.mu.Unlock()
	if marked {
		t.Fatalf("approval ID was dedup-marked despite all sends failing; retry would be suppressed")
	}
	if client.calls == 0 {
		t.Fatalf("expected at least one send attempt on first call")
	}

	// Second call with the same ID must attempt again (not early-return).
	before := client.calls
	pm.sendApproval(context.Background(), pushApprovalPayload{ID: id, Body: "approve?"})
	if client.calls <= before {
		t.Fatalf("second call did not retry after a total send failure (calls before=%d after=%d)", before, client.calls)
	}
}

// TestSendApprovalDedupsAfterSuccess verifies the normal-path behavior is
// preserved: once a send succeeds, the approval ID is recorded and a subsequent
// call with the same ID is suppressed (within the 1h dedup window).
func TestSendApprovalDedupsAfterSuccess(t *testing.T) {
	client := &auditPushStubHTTPClient{status: http.StatusCreated}
	pm := auditPushNewManager(t, client)

	const id = "approval-ok"
	pm.sendApproval(context.Background(), pushApprovalPayload{ID: id, Body: "approve?"})

	pm.mu.Lock()
	_, marked := pm.sent[id]
	pm.mu.Unlock()
	if !marked {
		t.Fatalf("approval ID was not dedup-marked after a successful send")
	}

	before := client.calls
	pm.sendApproval(context.Background(), pushApprovalPayload{ID: id, Body: "approve?"})
	if client.calls != before {
		t.Fatalf("duplicate approval was sent again despite a prior success (calls before=%d after=%d)", before, client.calls)
	}
}

// TestSendApprovalDoesNotDedupWhenSubscriptionExpired verifies that a Gone/
// NotFound response (subscription pruned, not a real delivery) is not counted as
// success, so the ID is not dedup-marked.
func TestSendApprovalDoesNotDedupWhenSubscriptionExpired(t *testing.T) {
	client := &auditPushStubHTTPClient{status: http.StatusGone}
	pm := auditPushNewManager(t, client)

	const id = "approval-gone"
	pm.sendApproval(context.Background(), pushApprovalPayload{ID: id, Body: "approve?"})

	pm.mu.Lock()
	_, marked := pm.sent[id]
	pm.mu.Unlock()
	if marked {
		t.Fatalf("approval ID was dedup-marked even though the only subscription was Gone")
	}
}
