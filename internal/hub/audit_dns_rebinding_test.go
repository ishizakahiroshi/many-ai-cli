package hub

import (
	"context"
	"errors"
	"net"
	"strings"
	"testing"
	"time"
)

// F14 の回帰テスト。privateNetworkBlockingDialContext が DNS を二度引かず、
// 検証済み IP を net.JoinHostPort して next に渡すことを保証する。DNS
// rebinding（TTL=0 の攻撃者ドメインが public→private の順で応答を切り替える）
// による SSRF 迂回を防ぐための seam。

// withStubLookup は lookupIPAddr を一時的に差し替え、defer で戻す。
func withStubLookup(t *testing.T, stub func(ctx context.Context, host string) ([]net.IPAddr, error)) {
	t.Helper()
	orig := lookupIPAddr
	lookupIPAddr = stub
	t.Cleanup(func() { lookupIPAddr = orig })
}

// TestPrivateNetworkBlockingDialContext_DialsResolvedIPLiteral は
// 検証済み public IP がそのまま dial address（IP リテラル + port）として
// next へ渡ることを assert する。
func TestPrivateNetworkBlockingDialContext_DialsResolvedIPLiteral(t *testing.T) {
	withStubLookup(t, func(_ context.Context, _ string) ([]net.IPAddr, error) {
		return []net.IPAddr{{IP: net.ParseIP("8.8.8.8")}}, nil
	})

	var captured string
	next := func(_ context.Context, _, address string) (net.Conn, error) {
		captured = address
		return nil, errors.New("stub: no real dial")
	}

	dial := privateNetworkBlockingDialContext(next)
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	_, _ = dial(ctx, "tcp", "example.com:443")

	if captured != "8.8.8.8:443" {
		t.Fatalf("next was called with %q; expected IP literal 8.8.8.8:443 to prevent DNS rebinding TOCTOU", captured)
	}
}

// TestPrivateNetworkBlockingDialContext_BlocksPrivateIP は
// resolver が private IP を返す場合、next が呼ばれずに block error で
// 弾かれることを assert する（既存 SSRF ガードの基本動作）。
func TestPrivateNetworkBlockingDialContext_BlocksPrivateIP(t *testing.T) {
	withStubLookup(t, func(_ context.Context, _ string) ([]net.IPAddr, error) {
		return []net.IPAddr{{IP: net.ParseIP("192.168.1.5")}}, nil
	})

	nextCalled := false
	next := func(_ context.Context, _, _ string) (net.Conn, error) {
		nextCalled = true
		return nil, nil
	}

	dial := privateNetworkBlockingDialContext(next)
	_, err := dial(context.Background(), "tcp", "attacker.example.com:443")
	if err == nil {
		t.Fatal("expected private network block error, got nil")
	}
	if !strings.Contains(err.Error(), "private network host blocked") {
		t.Fatalf("expected private network block error, got %q", err)
	}
	if nextCalled {
		t.Fatal("next must not be invoked when resolved IP is private")
	}
}

// TestPrivateNetworkBlockingDialContext_DualStackFallback は
// 複数 IP が返った場合、最初の dial が失敗すると次の IP でリトライすることを
// assert する（IPv6/IPv4 fallback）。address は各回 IP リテラルであること。
func TestPrivateNetworkBlockingDialContext_DualStackFallback(t *testing.T) {
	withStubLookup(t, func(_ context.Context, _ string) ([]net.IPAddr, error) {
		return []net.IPAddr{
			{IP: net.ParseIP("2001:4860:4860::8888")},
			{IP: net.ParseIP("8.8.8.8")},
		}, nil
	})

	var attempts []string
	next := func(_ context.Context, _, address string) (net.Conn, error) {
		attempts = append(attempts, address)
		if len(attempts) == 1 {
			return nil, errors.New("stub: IPv6 unreachable")
		}
		return nil, errors.New("stub: IPv4 attempt captured")
	}

	dial := privateNetworkBlockingDialContext(next)
	_, _ = dial(context.Background(), "tcp", "example.com:443")

	if len(attempts) != 2 {
		t.Fatalf("expected 2 dial attempts (dual-stack fallback), got %d: %v", len(attempts), attempts)
	}
	if attempts[0] != "[2001:4860:4860::8888]:443" {
		t.Errorf("first attempt should be IPv6 IP literal, got %q", attempts[0])
	}
	if attempts[1] != "8.8.8.8:443" {
		t.Errorf("second attempt should be IPv4 IP literal, got %q", attempts[1])
	}
}

// TestPrivateNetworkBlockingDialContext_BlocksBeforeAnyDial は
// 複数 IP のうち 1 つでも private があれば block error で早期棄却し、
// 他 IP に fallback しないことを assert する（多層防御）。
func TestPrivateNetworkBlockingDialContext_BlocksBeforeAnyDial(t *testing.T) {
	withStubLookup(t, func(_ context.Context, _ string) ([]net.IPAddr, error) {
		return []net.IPAddr{
			{IP: net.ParseIP("8.8.8.8")},
			{IP: net.ParseIP("127.0.0.1")},
		}, nil
	})

	nextCalled := false
	next := func(_ context.Context, _, _ string) (net.Conn, error) {
		nextCalled = true
		return nil, nil
	}

	dial := privateNetworkBlockingDialContext(next)
	_, err := dial(context.Background(), "tcp", "example.com:443")
	if err == nil {
		t.Fatal("expected block error when any resolved IP is private, got nil")
	}
	if nextCalled {
		t.Fatal("next must not be invoked when any resolved IP is private")
	}
}

// TestPrivateNetworkBlockingDialContext_EmptyResolution は
// resolver が空の結果を返した場合に明示的なエラーで終了し、next が
// 呼ばれないことを assert する。
func TestPrivateNetworkBlockingDialContext_EmptyResolution(t *testing.T) {
	withStubLookup(t, func(_ context.Context, _ string) ([]net.IPAddr, error) {
		return nil, nil
	})

	nextCalled := false
	next := func(_ context.Context, _, _ string) (net.Conn, error) {
		nextCalled = true
		return nil, nil
	}

	dial := privateNetworkBlockingDialContext(next)
	_, err := dial(context.Background(), "tcp", "example.com:443")
	if err == nil {
		t.Fatal("expected error for empty DNS resolution, got nil")
	}
	if nextCalled {
		t.Fatal("next must not be invoked when DNS resolution is empty")
	}
}
