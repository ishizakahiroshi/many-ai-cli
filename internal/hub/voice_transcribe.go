package hub

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime/multipart"
	"net"
	"net/http"
	neturl "net/url"
	"strings"
	"time"
	"unicode"

	"many-ai-cli/internal/config"
)

const voiceTranscribeMaxBytes = 25 * 1024 * 1024

type voiceTranscribeResponse struct {
	OK   bool   `json:"ok"`
	Text string `json:"text"`
}

type whisperProxyError struct {
	status int
	code   string
	detail string
}

func (e whisperProxyError) Error() string { return e.detail }

func (s *Server) handleVoiceTranscribe(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	cfg := s.snapshotCfg().Voice.Whisper
	if cfg.Managed {
		managedCfg, err := s.ensureManagedWhisper(r.Context())
		if err != nil {
			var proxyErr whisperProxyError
			if errors.As(err, &proxyErr) {
				writeJSONError(w, proxyErr.status, proxyErr.code, proxyErr.detail)
				return
			}
			writeJSONError(w, http.StatusBadGateway, "whisper_start_failed", err.Error())
			return
		}
		cfg = managedCfg
	}
	audio, err := io.ReadAll(io.LimitReader(r.Body, voiceTranscribeMaxBytes+1))
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("read audio failed", err))
		return
	}
	if len(audio) > voiceTranscribeMaxBytes {
		writeJSONError(w, http.StatusRequestEntityTooLarge, "audio_too_large", "audio too large")
		return
	}
	text, err := transcribeWithWhisper(r.Context(), cfg, audio)
	if err != nil {
		var proxyErr whisperProxyError
		if errors.As(err, &proxyErr) {
			writeJSONError(w, proxyErr.status, proxyErr.code, proxyErr.detail)
			return
		}
		writeJSONError(w, http.StatusBadGateway, "whisper_failed", err.Error())
		return
	}
	if isWhisperHallucination(text, cfg.HallucinationPhrases) {
		writeJSONError(w, http.StatusUnprocessableEntity, "hallucination", "transcript matched a known hallucination phrase")
		return
	}
	writeJSON(w, voiceTranscribeResponse{OK: true, Text: text})
}

// normalizeWhisperTranscript は幻聴フィルタの比較用に表記ゆれを吸収する
// （小文字化・空白除去・末尾の句読点や括弧類の除去）。
func normalizeWhisperTranscript(text string) string {
	var b strings.Builder
	for _, r := range strings.ToLower(text) {
		if unicode.IsSpace(r) {
			continue
		}
		b.WriteRune(r)
	}
	return strings.TrimRight(b.String(), "。．.!！?？、,，'\"“”‘’「」『』（）()[]{}")
}

func isWhisperHallucination(text string, phrases []string) bool {
	normalized := normalizeWhisperTranscript(text)
	if normalized == "" {
		return false
	}
	for _, phrase := range phrases {
		if normalized == normalizeWhisperTranscript(phrase) {
			return true
		}
	}
	return false
}

func transcribeWithWhisper(parent context.Context, cfg config.VoiceWhisperConfig, audio []byte) (string, error) {
	serverURL := strings.TrimSpace(cfg.ServerURL)
	if serverURL == "" {
		return "", whisperProxyError{status: http.StatusBadRequest, code: "whisper_not_configured", detail: "whisper server_url is not configured"}
	}
	if len(audio) == 0 {
		return "", whisperProxyError{status: http.StatusBadRequest, code: "bad_request", detail: "empty audio"}
	}
	timeout := time.Duration(cfg.TimeoutSeconds) * time.Second
	if timeout <= 0 {
		timeout = 60 * time.Second
	}
	ctx, cancel := context.WithTimeout(parent, timeout)
	defer cancel()
	client := &http.Client{Timeout: timeout}

	paths := whisperRequestPaths(cfg.RequestPath)
	var lastErr error
	for i, path := range paths {
		text, status, err := postWhisper(ctx, client, cfg, serverURL, path, audio)
		if err == nil {
			return text, nil
		}
		if status == http.StatusNotFound && i == 0 && strings.TrimSpace(cfg.RequestPath) == "" {
			lastErr = err
			continue
		}
		return "", err
	}
	if lastErr != nil {
		return "", lastErr
	}
	return "", whisperProxyError{status: http.StatusBadGateway, code: "whisper_failed", detail: "whisper failed"}
}

func whisperRequestPaths(requestPath string) []string {
	requestPath = strings.TrimSpace(requestPath)
	if requestPath != "" {
		return []string{requestPath}
	}
	return []string{"/v1/audio/transcriptions", "/inference"}
}

func postWhisper(ctx context.Context, client *http.Client, cfg config.VoiceWhisperConfig, serverURL, requestPath string, audio []byte) (string, int, error) {
	var body bytes.Buffer
	mw := multipart.NewWriter(&body)
	part, err := mw.CreateFormFile("file", "audio.wav")
	if err != nil {
		return "", 0, err
	}
	if _, err := part.Write(audio); err != nil {
		return "", 0, err
	}
	language := strings.TrimSpace(cfg.Language)
	if language != "" && !strings.EqualFold(language, "auto") {
		_ = mw.WriteField("language", language)
	}
	_ = mw.WriteField("response_format", "json")
	if requestPath == "/v1/audio/transcriptions" {
		_ = mw.WriteField("model", "whisper-1")
	}
	if err := mw.Close(); err != nil {
		return "", 0, err
	}

	target, err := whisperTargetURL(serverURL, requestPath)
	if err != nil {
		return "", 0, whisperProxyError{status: http.StatusBadGateway, code: "whisper_failed", detail: err.Error()}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, target, &body)
	if err != nil {
		return "", 0, err
	}
	req.Header.Set("Content-Type", mw.FormDataContentType())
	resp, err := client.Do(req)
	if err != nil {
		if isTimeoutError(err) || errors.Is(ctx.Err(), context.DeadlineExceeded) {
			return "", 0, whisperProxyError{status: http.StatusGatewayTimeout, code: "whisper_timeout", detail: "whisper request timed out"}
		}
		return "", 0, whisperProxyError{status: http.StatusBadGateway, code: "whisper_unreachable", detail: errorDetail("whisper unreachable", err)}
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		detailBytes, _ := io.ReadAll(io.LimitReader(resp.Body, 4096))
		detail := strings.TrimSpace(string(detailBytes))
		if detail == "" {
			detail = fmt.Sprintf("whisper returned HTTP %d", resp.StatusCode)
		}
		return "", resp.StatusCode, whisperProxyError{status: http.StatusBadGateway, code: "whisper_failed", detail: detail}
	}
	var data map[string]any
	if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<20)).Decode(&data); err != nil {
		return "", resp.StatusCode, whisperProxyError{status: http.StatusBadGateway, code: "whisper_failed", detail: errorDetail("decode whisper response failed", err)}
	}
	text, _ := data["text"].(string)
	text = strings.TrimSpace(text)
	if text == "" {
		return "", resp.StatusCode, whisperProxyError{status: http.StatusBadGateway, code: "whisper_failed", detail: "whisper response missing text"}
	}
	return text, resp.StatusCode, nil
}

func whisperTargetURL(serverURL, requestPath string) (string, error) {
	u, err := neturl.Parse(strings.TrimSpace(serverURL))
	if err != nil {
		return "", err
	}
	base := strings.TrimRight(u.Path, "/")
	if !strings.HasPrefix(requestPath, "/") {
		requestPath = "/" + requestPath
	}
	u.Path = base + requestPath
	u.RawQuery = ""
	u.Fragment = ""
	return u.String(), nil
}

func isTimeoutError(err error) bool {
	var netErr net.Error
	return errors.As(err, &netErr) && netErr.Timeout()
}
