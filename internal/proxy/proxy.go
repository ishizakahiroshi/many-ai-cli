// Package proxy は wrap 対象 CLI（Claude Code / Codex CLI）の API リクエストを
// Hub 内で透過プロキシし、payload を構造化して Sink へ渡す。
//
// ルーティング:
//   POST /anthropic/v1/messages           → https://api.anthropic.com/v1/messages
//   POST /openai/v1/chat/completions      → https://api.openai.com/v1/chat/completions
//   POST /openai/v1/responses             → https://api.openai.com/v1/responses
//   GET  /healthz                         → 204 (起動確認用)
//
// CLI 側は `ANTHROPIC_BASE_URL=http://127.0.0.1:<port>/anthropic` /
// `OPENAI_BASE_URL=http://127.0.0.1:<port>/openai/v1` を env で受け取る。
//
// 注:
//   - TLS MITM・CA 配布は行わない。本プロキシは BASE_URL 差し替えだけで成立する
//     provider のみ対象。
//   - Authorization ヘッダーは Sink へ渡す構造化データに含めない（記録経路を作らない）。
//   - response body は client へ素通ししつつ、size cap 付きで複製して Sink へ渡す。
//     SSE ストリーミングは chunk を結合してから 1 メッセージとして Sink に通知する。
package proxy

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"strings"
	"time"
)

const (
	// maxCaptureBytes は Sink へ渡す request/response body の上限。
	// これを超えた分は捕捉せず、Truncated=true で通知する。
	maxCaptureBytes = 1 << 20 // 1 MiB

	upstreamAnthropic = "https://api.anthropic.com"
	upstreamOpenAI    = "https://api.openai.com"
)

// Provider は本プロキシが扱う API 系統。
type Provider string

const (
	ProviderAnthropic Provider = "anthropic"
	ProviderOpenAI    Provider = "openai"
)

// CapturedTurn は 1 リクエスト/レスポンスの組。Sink へ渡される。
type CapturedTurn struct {
	Provider Provider
	// SessionToken: spawn 時に Hub が生成し env (MANY_AI_CLI_PROXY_TOKEN) で wrapper に渡し、
	// CLI 側 BASE_URL の path 要素 `/s/<token>/anthropic` として埋め込まれる token。
	// 空ならセッション未紐付け（旧 BASE_URL / 既存セッションからの呼び出し）。
	SessionToken string
	Endpoint     string // "/v1/messages" など
	ReceivedAt   time.Time
	DurationMS   int64

	StatusCode int

	// RequestBody / ResponseBody は raw JSON（SSE は結合後の JSON 配列風テキスト）。
	// 上限 maxCaptureBytes を超えた場合は truncate される。
	RequestBody  []byte
	ResponseBody []byte
	Truncated    bool

	// IsStream: response が text/event-stream だった場合 true。
	// ResponseBody は SSE chunk を順序保持で 1 行 JSON ずつ連結したものになる
	// （`data: ` プレフィックスは除去済み、`[DONE]` は除外）。
	IsStream bool

	// RequestErr / ResponseErr: 上流通信エラーや body 読み取りエラー（任意）。
	RequestErr  string
	ResponseErr string
}

// Sink は CapturedTurn を受け取る。Hub 側は session ring buffer に push する。
// 実装は non-blocking 推奨（プロキシ側で goroutine を切るが、長時間ブロックは詰まりの原因）。
type Sink interface {
	OnTurn(turn CapturedTurn)
}

// SinkFunc は Sink を関数で実装するためのアダプタ。
type SinkFunc func(turn CapturedTurn)

func (f SinkFunc) OnTurn(turn CapturedTurn) { f(turn) }

// Server は内蔵プロキシ。Hub の Run() から起動する。
type Server struct {
	listener net.Listener
	httpSrv  *http.Server
	logger   *slog.Logger
	sink     Sink
	port     int
}

// New は 127.0.0.1 で listen する Server を返す。port=0 で空きポート自動割当。
// 設計どおり外部公開しない（127.0.0.1 固定）。
func New(port int, sink Sink, logger *slog.Logger) (*Server, error) {
	if sink == nil {
		sink = SinkFunc(func(CapturedTurn) {})
	}
	if logger == nil {
		logger = slog.Default()
	}
	addr := fmt.Sprintf("127.0.0.1:%d", port)
	ln, err := net.Listen("tcp", addr)
	if err != nil {
		return nil, fmt.Errorf("proxy listen: %w", err)
	}
	s := &Server{
		listener: ln,
		logger:   logger,
		sink:     sink,
		port:     ln.Addr().(*net.TCPAddr).Port,
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(http.StatusNoContent) })
	// セッション未紐付けルート（既存挙動・互換用）
	mux.Handle("/anthropic/", s.makeReverseProxy(ProviderAnthropic, upstreamAnthropic, "/anthropic"))
	mux.Handle("/openai/", s.makeReverseProxy(ProviderOpenAI, upstreamOpenAI, "/openai"))
	// セッション紐付けルート: /s/<token>/anthropic/... → 上流 + Sink に SessionToken 伝搬
	mux.Handle("/s/", s.makeSessionReverseProxy())
	s.httpSrv = &http.Server{
		Handler:           mux,
		ReadHeaderTimeout: 10 * time.Second,
	}
	return s, nil
}

func (s *Server) Port() int { return s.port }

// Serve は listener を Serve する。ブロッキング。
func (s *Server) Serve() error {
	err := s.httpSrv.Serve(s.listener)
	if errors.Is(err, http.ErrServerClosed) {
		return nil
	}
	return err
}

// Shutdown は graceful に停止する。
func (s *Server) Shutdown(ctx context.Context) error {
	return s.httpSrv.Shutdown(ctx)
}

// makeReverseProxy は指定 provider 用のリバースプロキシハンドラを返す。
// pathPrefix は CLI 側 BASE_URL の prefix（"/anthropic" など）。
func (s *Server) makeReverseProxy(provider Provider, upstreamRaw, pathPrefix string) http.Handler {
	upstreamURL, _ := url.Parse(upstreamRaw)

	rp := &httputil.ReverseProxy{
		FlushInterval: -1, // SSE のため即時 flush
		Director: func(req *http.Request) {
			req.URL.Scheme = upstreamURL.Scheme
			req.URL.Host = upstreamURL.Host
			req.Host = upstreamURL.Host
			// /anthropic/v1/messages → /v1/messages
			req.URL.Path = strings.TrimPrefix(req.URL.Path, pathPrefix)
			if req.URL.Path == "" {
				req.URL.Path = "/"
			}
			// User-Agent は CLI のものを保持
		},
		ModifyResponse: func(resp *http.Response) error {
			// 何もしない。body の捕捉は ResponseWriter ラッパーで行う。
			return nil
		},
		ErrorHandler: func(w http.ResponseWriter, r *http.Request, err error) {
			s.logger.Warn("proxy upstream error", "provider", provider, "path", r.URL.Path, "err", err)
			http.Error(w, "upstream error: "+err.Error(), http.StatusBadGateway)
		},
	}

	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		startedAt := time.Now()
		endpoint := strings.TrimPrefix(r.URL.Path, pathPrefix)

		// 1. request body を読み取り（上限つき）→ 上流へ転送するため bytes.NewReader で巻き戻す
		reqBody, reqTruncated, reqErr := readCappedBody(r.Body, maxCaptureBytes)
		_ = r.Body.Close()
		if reqErr != nil {
			s.logger.Warn("proxy read req body", "err", reqErr)
		}
		// 上流転送用に body を巻き戻す。truncated 時は読み切ったところまで送る
		// （超過時は本来上流が拒否するが、ここで止めるとデバッグしにくいので素通し）。
		r.Body = io.NopCloser(bytes.NewReader(reqBody))
		r.ContentLength = int64(len(reqBody))

		// 2. response 捕捉用 ResponseWriter ラッパー
		cap := &captureWriter{
			ResponseWriter: w,
			buf:            &bytes.Buffer{},
			maxBytes:       maxCaptureBytes,
		}

		rp.ServeHTTP(cap, r)

		// 3. response body 確定 → Sink へ送信（非同期、ノンブロッキング）
		respBytes, respTruncated := cap.captured()
		isStream := strings.Contains(cap.Header().Get("Content-Type"), "text/event-stream")
		var respBody []byte
		if isStream {
			respBody = joinSSE(respBytes)
		} else {
			respBody = respBytes
		}

		turn := CapturedTurn{
			Provider:     provider,
			Endpoint:     endpoint,
			ReceivedAt:   startedAt,
			DurationMS:   time.Since(startedAt).Milliseconds(),
			StatusCode:   cap.statusCode,
			RequestBody:  reqBody,
			ResponseBody: respBody,
			Truncated:    reqTruncated || respTruncated,
			IsStream:     isStream,
		}
		if reqErr != nil {
			turn.RequestErr = reqErr.Error()
		}

		go s.dispatchSink(turn)
	})
}

// makeSessionReverseProxy は `/s/<token>/<provider>/...` を受けて、token を抽出した上で
// provider 別のリバースプロキシに dispatch する。Sink にも token を伝搬する。
func (s *Server) makeSessionReverseProxy() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		// path: /s/<token>/<provider>/<rest...>
		rest := strings.TrimPrefix(r.URL.Path, "/s/")
		slash := strings.IndexByte(rest, '/')
		if slash <= 0 {
			http.Error(w, "missing session token", http.StatusBadRequest)
			return
		}
		token := rest[:slash]
		afterToken := "/" + rest[slash+1:]
		var provider Provider
		var upstreamRaw, prefix string
		switch {
		case strings.HasPrefix(afterToken, "/anthropic"):
			provider = ProviderAnthropic
			upstreamRaw = upstreamAnthropic
			prefix = "/anthropic"
		case strings.HasPrefix(afterToken, "/openai"):
			provider = ProviderOpenAI
			upstreamRaw = upstreamOpenAI
			prefix = "/openai"
		default:
			http.Error(w, "unknown provider segment", http.StatusBadRequest)
			return
		}
		// path を /<provider>/... に縮めて、内部の通常ハンドラと同じ流れに乗せる
		r.URL.Path = afterToken
		// SessionToken を context に積むのではなく、後段で参照しやすいよう URL Header に詰める
		r.Header.Set("X-Many-Ai-Cli-Proxy-Token", token)
		s.proxyOnceWithToken(w, r, provider, upstreamRaw, prefix, token)
	})
}

// proxyOnceWithToken は makeReverseProxy 相当の処理を 1 回ぶん回し、Sink に token を伴って通知する。
// makeReverseProxy とのコード重複が大きいが、ReverseProxy の Director が中で req を書き換えるため
// 分離した方が読みやすい。
func (s *Server) proxyOnceWithToken(w http.ResponseWriter, r *http.Request, provider Provider, upstreamRaw, pathPrefix, token string) {
	upstreamURL, _ := url.Parse(upstreamRaw)
	rp := &httputil.ReverseProxy{
		FlushInterval: -1,
		Director: func(req *http.Request) {
			req.URL.Scheme = upstreamURL.Scheme
			req.URL.Host = upstreamURL.Host
			req.Host = upstreamURL.Host
			req.URL.Path = strings.TrimPrefix(req.URL.Path, pathPrefix)
			if req.URL.Path == "" {
				req.URL.Path = "/"
			}
			req.Header.Del("X-Many-Ai-Cli-Proxy-Token") // 上流へは送らない
		},
		ErrorHandler: func(w http.ResponseWriter, r *http.Request, err error) {
			s.logger.Warn("proxy upstream error", "provider", provider, "path", r.URL.Path, "err", err)
			http.Error(w, "upstream error: "+err.Error(), http.StatusBadGateway)
		},
	}
	startedAt := time.Now()
	endpoint := strings.TrimPrefix(r.URL.Path, pathPrefix)
	reqBody, reqTruncated, reqErr := readCappedBody(r.Body, maxCaptureBytes)
	_ = r.Body.Close()
	r.Body = io.NopCloser(bytes.NewReader(reqBody))
	r.ContentLength = int64(len(reqBody))
	cap := &captureWriter{ResponseWriter: w, buf: &bytes.Buffer{}, maxBytes: maxCaptureBytes}
	rp.ServeHTTP(cap, r)
	respBytes, respTruncated := cap.captured()
	isStream := strings.Contains(cap.Header().Get("Content-Type"), "text/event-stream")
	var respBody []byte
	if isStream {
		respBody = joinSSE(respBytes)
	} else {
		respBody = respBytes
	}
	turn := CapturedTurn{
		Provider:     provider,
		SessionToken: token,
		Endpoint:     endpoint,
		ReceivedAt:   startedAt,
		DurationMS:   time.Since(startedAt).Milliseconds(),
		StatusCode:   cap.statusCode,
		RequestBody:  reqBody,
		ResponseBody: respBody,
		Truncated:    reqTruncated || respTruncated,
		IsStream:     isStream,
	}
	if reqErr != nil {
		turn.RequestErr = reqErr.Error()
	}
	go s.dispatchSink(turn)
}

func (s *Server) dispatchSink(turn CapturedTurn) {
	defer func() {
		if rec := recover(); rec != nil {
			s.logger.Warn("proxy sink panic", "panic", rec)
		}
	}()
	s.sink.OnTurn(turn)
}

// readCappedBody は body を最大 maxBytes 読む。超過分は破棄して truncated=true を返す。
// 上流送出には ここで読んだ範囲しか渡らないため、超過時の上流動作はベストエフォート。
func readCappedBody(rc io.ReadCloser, maxBytes int) ([]byte, bool, error) {
	if rc == nil {
		return nil, false, nil
	}
	limited := io.LimitReader(rc, int64(maxBytes)+1)
	b, err := io.ReadAll(limited)
	truncated := false
	if len(b) > maxBytes {
		b = b[:maxBytes]
		truncated = true
		// 残りを drain して TCP 上の半端を防ぐ（無視）
		_, _ = io.Copy(io.Discard, rc)
	}
	return b, truncated, err
}

// captureWriter は http.ResponseWriter をラップして status と body を捕捉する。
// http.Flusher / http.Hijacker を必要に応じて委譲し、SSE を素通しさせる。
type captureWriter struct {
	http.ResponseWriter
	statusCode int
	buf        *bytes.Buffer
	maxBytes   int
	truncated  bool
}

func (c *captureWriter) WriteHeader(code int) {
	c.statusCode = code
	c.ResponseWriter.WriteHeader(code)
}

func (c *captureWriter) Write(p []byte) (int, error) {
	if c.statusCode == 0 {
		c.statusCode = http.StatusOK
	}
	if !c.truncated {
		remain := c.maxBytes - c.buf.Len()
		if remain > 0 {
			if len(p) <= remain {
				c.buf.Write(p)
			} else {
				c.buf.Write(p[:remain])
				c.truncated = true
			}
		} else {
			c.truncated = true
		}
	}
	return c.ResponseWriter.Write(p)
}

func (c *captureWriter) Flush() {
	if f, ok := c.ResponseWriter.(http.Flusher); ok {
		f.Flush()
	}
}

func (c *captureWriter) captured() ([]byte, bool) {
	return c.buf.Bytes(), c.truncated
}

// joinSSE は SSE 生バイト列から `data: ` 行を抽出し、JSON を改行区切りで連結する。
// `[DONE]` は除外。Anthropic Messages SSE / OpenAI chat.completions SSE のどちらも
// `data: <json>\n\n` 形式なので共通処理で扱える。
func joinSSE(raw []byte) []byte {
	if len(raw) == 0 {
		return raw
	}
	var out bytes.Buffer
	for _, line := range bytes.Split(raw, []byte("\n")) {
		line = bytes.TrimSpace(line)
		if !bytes.HasPrefix(line, []byte("data:")) {
			continue
		}
		payload := bytes.TrimSpace(line[len("data:"):])
		if len(payload) == 0 || bytes.Equal(payload, []byte("[DONE]")) {
			continue
		}
		out.Write(payload)
		out.WriteByte('\n')
	}
	return out.Bytes()
}

