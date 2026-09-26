package hub

import (
	"errors"
	"net/http"
	"testing"

	"many-ai-cli/internal/nvidianim"
)

func TestNVIDIANIMTestErrorClassificationIsGenericAndSecretFree(t *testing.T) {
	tests := []struct {
		name string
		err  error
		want string
	}{
		{name: "bad request", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusBadRequest}, want: "request_rejected"},
		{name: "unprocessable request", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusUnprocessableEntity}, want: "request_rejected"},
		{name: "invalid key", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusUnauthorized}, want: "unauthorized"},
		{name: "trial entitlement", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusPaymentRequired}, want: "payment_required"},
		{name: "permission or terms", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusForbidden}, want: "forbidden"},
		{name: "unavailable model", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusNotFound}, want: "not_found"},
		{name: "HTTP timeout", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusRequestTimeout}, want: "timeout"},
		{name: "client timeout", err: nvidianim.ErrCatalogTimeout, want: "timeout"},
		{name: "rate limit", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusTooManyRequests}, want: "rate_limited"},
		{name: "service unavailable", err: &nvidianim.CatalogHTTPError{StatusCode: http.StatusServiceUnavailable}, want: "server_error"},
		{name: "DNS or TLS failure", err: errors.New("synthetic DNS or TLS detail with secret-like text"), want: "connection_failed"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			if got := nvidiaNIMTestErrorCode(tc.err); got != tc.want {
				t.Fatalf("error code = %q, want %q", got, tc.want)
			}
		})
	}
}
