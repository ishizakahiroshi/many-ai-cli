package dotsbridge

import (
	"sync"
	"time"
)

// FakeTransport compares the common state contracts only, not MCP/Slack protocols.
type FakeTransport struct {
	mu      sync.Mutex
	Route   Route
	effects map[string]string
	calls   int
	Outcome DeliveryOutcome
}

func NewFakeTransport(route Route) *FakeTransport {
	return &FakeTransport{Route: route, effects: map[string]string{}, Outcome: Delivered}
}
func (t *FakeTransport) Receive(s *Store, e Envelope, now time.Time) (Receipt, error) {
	if e.Source != t.Route {
		return Receipt{}, ErrHeld
	}
	return s.Ingest(e, now)
}
func (t *FakeTransport) Send(o Outbox) DeliveryOutcome {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.calls++
	if o.Route != t.Route {
		return NotSent
	}
	if t.Outcome == NotSent {
		return NotSent
	}
	if previous, ok := t.effects[o.ID]; ok && previous != o.Payload {
		return Uncertain
	}
	t.effects[o.ID] = o.Payload
	return t.Outcome
}
func (t *FakeTransport) Counts() (calls, effects int) {
	t.mu.Lock()
	defer t.mu.Unlock()
	return t.calls, len(t.effects)
}
