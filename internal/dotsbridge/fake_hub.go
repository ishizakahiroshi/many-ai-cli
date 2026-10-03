package dotsbridge

import (
	"context"
	"errors"
	"fmt"
	"sync"
)

// FakeHub has no sockets, processes, credentials, provider dependency or timers.
// It preserves canonical request IDs while aliasing POSTs to an active run.
type FakeHub struct {
	mu           sync.Mutex
	instance     string
	runs         map[string]Run
	aliases      map[string]string
	capabilities map[string]runIdentity
	next         int
	posts        int
	loseResponse bool
	hidden       bool
	beforePost   func()
}

func NewFakeHub(instance string) *FakeHub {
	return &FakeHub{instance: instance, runs: map[string]Run{}, aliases: map[string]string{}, capabilities: map[string]runIdentity{}}
}
func (h *FakeHub) List(_ context.Context, routine string) ([]Run, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	out := []Run{}
	if !h.hidden {
		for _, r := range h.runs {
			if r.RoutineID == routine {
				out = append(out, r)
			}
		}
	}
	return out, nil
}
func (h *FakeHub) Post(_ context.Context, routine, request string) (Run, error) {
	h.mu.Lock()
	hook := h.beforePost
	h.beforePost = nil
	h.mu.Unlock()
	if hook != nil {
		hook()
	}
	h.mu.Lock()
	defer h.mu.Unlock()
	h.posts++
	if id, ok := h.aliases[routine+":"+request]; ok {
		return h.response(h.runs[id])
	}
	for _, r := range h.runs {
		if r.RoutineID == routine && r.active() {
			h.aliases[routine+":"+request] = r.ID
			return h.response(r)
		}
	}
	r, err := h.add(routine, request, 0)
	if err != nil {
		return Run{}, err
	}
	return h.response(r)
}
func (h *FakeHub) response(r Run) (Run, error) {
	if h.loseResponse {
		return Run{}, errors.New("simulated POST response loss")
	}
	return r, nil
}
func (h *FakeHub) add(routine, request string, session int) (Run, error) {
	h.next++
	if session == 0 {
		session = h.next
	}
	r := Run{ID: fmt.Sprintf("fake-run-%d", h.next), RoutineID: routine, RequestID: request, HubInstanceID: h.instance, SessionID: session, Status: "running"}
	key, err := opaque()
	if err != nil {
		return Run{}, err
	}
	h.runs[r.ID] = r
	h.aliases[routine+":"+request] = r.ID
	h.capabilities[key] = r.identity()
	return r, nil
}
func (h *FakeHub) Get(_ context.Context, id string) (Run, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	r, ok := h.runs[id]
	if !ok || h.hidden {
		return Run{}, ErrHeld
	}
	return r, nil
}
func (h *FakeHub) VerifyConsumer(_ context.Context, c ConsumerCapability) (Run, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	identity, ok := h.capabilities[c.key]
	if !ok {
		return Run{}, ErrConsumer
	}
	r, ok := h.runs[identity.RunID]
	if !ok || r.identity() != identity {
		return Run{}, ErrConsumer
	}
	return r, nil
}

// consumer simulates the trusted launcher's private handoff to the actual child.
// It is deliberately unexported; no event/CLI/request can ask for a capability.
func (h *FakeHub) consumer(id string) (ConsumerCapability, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	r, ok := h.runs[id]
	if !ok {
		return ConsumerCapability{}, ErrConsumer
	}
	for key, identity := range h.capabilities {
		if identity == r.identity() {
			return ConsumerCapability{key}, nil
		}
	}
	return ConsumerCapability{}, ErrConsumer
}
func (h *FakeHub) Finish(id string, available bool) error {
	h.mu.Lock()
	defer h.mu.Unlock()
	r, ok := h.runs[id]
	if !ok {
		return ErrHeld
	}
	r.Status = "finished"
	r.ResultAvailable = available
	h.runs[id] = r
	return nil
}
func (h *FakeHub) Manual(routine, request string, session int) (Run, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	return h.add(routine, request, session)
}
func (h *FakeHub) LosePOSTResponse(v bool) { h.mu.Lock(); defer h.mu.Unlock(); h.loseResponse = v }
func (h *FakeHub) HideHistory(v bool)      { h.mu.Lock(); defer h.mu.Unlock(); h.hidden = v }
func (h *FakeHub) BeforePOST(f func())     { h.mu.Lock(); defer h.mu.Unlock(); h.beforePost = f }
func (h *FakeHub) Counts() (posts, runs int) {
	h.mu.Lock()
	defer h.mu.Unlock()
	return h.posts, len(h.runs)
}
