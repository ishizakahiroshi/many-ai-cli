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

	// RequestBody は廃止（常に nil）。以前は転送用ボディにも maxCaptureBytes の cap を
	// 適用してしまい、1 MiB 超のリクエストを切断して上流 400 (unexpected end of data) を
	// 誘発していたため、request は捕捉せず素通しに変更した（捕捉は response のみ）。
	// ResponseBody は raw JSON（SSE は結合後の JSON 配列風テキスト）。上限 maxCaptureBytes 超で truncate。
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

		// request body は読まず ReverseProxy に素通しさせる（上流へ無加工で全文転送）。
		// 以前は観測用 cap を転送ボディにも適用し、1 MiB 超のリクエストを切断して
		// 上流 400 (unexpected end of data) を起こしていた。捕捉は response のみ。
		//
		// [診断] 転送 body は cap せず、実際に上流へ流れたバイト数だけ数える（非破壊）。
		// Content-Length と食い違えば、上流到達前（CLI 側）で body が短く切れている疑い。
		reqCL := r.ContentLength
		bodyCount := instrumentRequestBody(r)

		// response 捕捉用 ResponseWriter ラッパー（client へは素通し、コピーだけ cap）
		cap := &captureWriter{
			ResponseWriter: w,
			buf:            &bytes.Buffer{},
			maxBytes:       maxCaptureBytes,
		}

		rp.ServeHTTP(cap, r)

		s.logRequestForward(provider, endpoint, reqCL, bodyCount, cap.statusCode)

		// response body 確定 → Sink へ送信（非同期、ノンブロッキング）
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
			ResponseBody: respBody,
			Truncated:    respTruncated,
			IsStream:     isStream,
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
	// request body は読まず素通し（上流へ無加工で全文転送）。捕捉は response のみ。
	// [診断] 転送バイト数だけ数える（非破壊・cap しない）。
	reqCL := r.ContentLength
	bodyCount := instrumentRequestBody(r)
	cap := &captureWriter{ResponseWriter: w, buf: &bytes.Buffer{}, maxBytes: maxCaptureBytes}
	rp.ServeHTTP(cap, r)
	s.logRequestForward(provider, endpoint, reqCL, bodyCount, cap.statusCode)
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
		ResponseBody: respBody,
		Truncated:    respTruncated,
		IsStream:     isStream,
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

// countingReadCloser は ReadCloser をラップして、読まれた総バイト数を数えるだけのもの。
// ReverseProxy が r.Body から上流へコピーした実バイト数を計測する用途（非破壊・cap なし）。
type countingReadCloser struct {
	rc io.ReadCloser
	n  int64
}

func (c *countingReadCloser) Read(p []byte) (int, error) {
	n, err := c.rc.Read(p)
	c.n += int64(n)
	return n, err
}

func (c *countingReadCloser) Close() error { return c.rc.Close() }

// instrumentRequestBody は r.Body を countingReadCloser に差し替え、上流へ流れた
// 実バイト数を後から取得できるようにする。body の中身は一切加工・cap しない。
// r.Body が nil（GET 等）の場合は何もせず nil を返す。
func instrumentRequestBody(r *http.Request) *countingReadCloser {
	if r.Body == nil {
		return nil
	}
	c := &countingReadCloser{rc: r.Body}
	r.Body = c
	return c
}

// logRequestForward は 1 リクエストの「宣言 Content-Length」と「実際に上流へ転送した
// バイト数」を比較してログする（診断用）。
//   - cl >= 0 かつ転送数と食い違う → WARN（上流到達前で body が短く切れている疑い＝
//     CLI 側送出か localhost 接続断。proxy は cap していないので proxy 由来ではない）。
//   - 一致していて 256 KiB 以上の大きめリクエスト → INFO（大リクエストが無事素通しできた記録）。
//   - それ以外（小さく一致）→ 何も出さない（ノイズ抑制）。
func (s *Server) logRequestForward(provider Provider, endpoint string, cl int64, c *countingReadCloser, status int) {
	if c == nil {
		return
	}
	if cl >= 0 && c.n != cl {
		s.logger.Warn("proxy request body length mismatch (possible pre-proxy truncation)",
			"provider", provider, "endpoint", endpoint,
			"content_length", cl, "forwarded", c.n, "status", status)
		return
	}
	if c.n >= 256*1024 {
		s.logger.Info("proxy request forwarded",
			"provider", provider, "endpoint", endpoint,
			"content_length", cl, "forwarded", c.n, "status", status)
	}
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

