package hub

import (
	"archive/tar"
	"archive/zip"
	"compress/gzip"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"

	"many-ai-cli/internal/config"
	"many-ai-cli/internal/whisperruntime"
)

const (
	whisperReleaseTag        = "v1.8.6"
	whisperDefaultModelID    = "small"
	whisperInstallHTTPUA     = "many-ai-cli whisper installer"
	whisperServerReadyWait   = 20 * time.Second
	whisperDownloadExtraRoom = 256 * 1024 * 1024

	// whisperDownloadHeaderTimeout は TCP 接続後に応答ヘッダが届くまでの上限。
	// 本文(最大 488MB)の正常な低速転送は切らないため、全体 Timeout ではなく
	// ヘッダ/ハンドシェイク単位のタイムアウトを使う。
	whisperDownloadHeaderTimeout = 60 * time.Second
	whisperDownloadTLSTimeout    = 30 * time.Second
	// whisperDownloadStallTimeout は本文読込中に進捗が無いまま許容する最大時間。
	// これを超えると Body を閉じて Read のブロックを解除し、ダウンロードを失敗させる。
	whisperDownloadStallTimeout = 90 * time.Second
)

// whisperDownloadClient は managed Whisper のアーカイブ/モデル取得専用 HTTP クライアント。
// http.DefaultClient（Timeout:0=無制限）と異なり、ヘッダ受信・TLS ハンドシェイクに
// 上限を設けて stall による恒久ハングを防ぐ。本文全体への一律 Timeout は付けない
// （488MB の正常な低速転送を中断しないため。idle/stall はウォッチドッグで検出する）。
var whisperDownloadClient = func() *http.Client {
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.ResponseHeaderTimeout = whisperDownloadHeaderTimeout
	transport.TLSHandshakeTimeout = whisperDownloadTLSTimeout
	return &http.Client{Transport: transport}
}()

// whisperBinaryEntry は OS/arch ごとの managed Whisper バイナリ入手定義。
// ServerNames は実行ファイル候補名、KeepFromArchive が非空ならアーカイブから
// その basename だけを bin/ へ平坦展開する。Runtime は internal/whisperruntime
// の同梱物キー（os-arch）で、空なら同梱ランタイム無し。Archive が空のときは
// URL 拡張子から zip / tar.gz を推定する。
type whisperBinaryEntry struct {
	Version         string
	URL             string
	SHA256          string
	SizeBytes       int64
	Archive         string
	ServerNames     []string
	KeepFromArchive []string
	Runtime         string
}

type whisperProcessJob uintptr

// whisperBinaries は OS/arch → 入手定義。ここに在る OS/arch だけが
// 「ダウンロード方式の managed install」をサポートする（whisperManagedSupported）。
// Docker/リモートサーバー など実行ファイルを焼き込む構成では MANY_AI_CLI_WHISPER_SERVER で
// 既設バイナリを指す（bakedWhisperServerPath）ため、ここへの登録は不要。
//
// TODO(C3/C4): Linux/macOS は公式 release に server バイナリが無いため、
// .github/workflows/whisper-binaries.yml が自前ビルドした tar.gz を
// 自リポジトリ release 資産として公開したら、実 URL/SHA256 で下記を有効化する。
//
//	"linux/amd64": {
//	    Version: "v1.8.6",
//	    URL:     "https://github.com/ishizakahiroshi/many-ai-cli/releases/download/whisper-v1.8.6/whisper-server-linux-amd64.tar.gz",
//	    SHA256:  "<fill-from-ci>",
//	    Archive: "tar.gz",
//	    ServerNames:     []string{"whisper-server"},
//	    KeepFromArchive: []string{"whisper-server"},
//	},
//	"darwin/arm64": { ...whisper-server-darwin-arm64.tar.gz... },
//	"darwin/amd64": { ...whisper-server-darwin-amd64.tar.gz... },
var whisperBinaries = map[string]whisperBinaryEntry{
	"windows/amd64": {
		Version:         whisperReleaseTag,
		URL:             "https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.6/whisper-bin-x64.zip",
		SHA256:          "b07ea0b1b4115a38e1a7b07debf581f0b77d999925f8acb8f39d322b0ba0a822",
		SizeBytes:       4093849,
		Archive:         "zip",
		ServerNames:     []string{"whisper-server.exe", "server.exe"},
		KeepFromArchive: []string{"whisper-server.exe", "whisper.dll", "ggml.dll", "ggml-base.dll", "ggml-cpu.dll"},
		Runtime:         "windows-amd64",
	},
}

// whisperServerEnvVar は焼き込み済み whisper-server のフルパスを指す環境変数。
// Docker(リモートサーバー) イメージが /usr/local/bin/whisper-server を焼き込み、この変数で
// Hub に知らせる。設定されていればダウンロード無しで managed 扱いになる（C3/D5）。
const whisperServerEnvVar = "MANY_AI_CLI_WHISPER_SERVER"

func bakedWhisperServerPath() string {
	p := strings.TrimSpace(os.Getenv(whisperServerEnvVar))
	if p != "" && whisperFileExists(p) {
		return p
	}
	return ""
}

func whisperBinaryForHost() (whisperBinaryEntry, bool) {
	entry, ok := whisperBinaries[runtime.GOOS+"/"+runtime.GOARCH]
	return entry, ok
}

type whisperModelOption struct {
	ID          string `json:"id"`
	Label       string `json:"label"`
	FileName    string `json:"file_name"`
	URL         string `json:"-"`
	SizeBytes   int64  `json:"size_bytes"`
	Quality     string `json:"quality"`
	SHA256      string `json:"sha256,omitempty"`
	Default     bool   `json:"default,omitempty"`
	HashChecked bool   `json:"hash_checked"`
}

// whisperModelOptions はモデル選択候補。SHA256 は HuggingFace LFS ポインタ
// （ggerganov/whisper.cpp の /raw/main/<file>.bin）から取得した値で、
// resolve/main/ 経由でダウンロードされる実ファイルバイトの SHA256 と同一。
// SHA256 が空のモデルは downloadFile で整合性検証をスキップする（インストールは可能）。
// 誤値を入れると正規インストールが失敗するため、確証のない値は絶対に設定しないこと。
var whisperModelOptions = []whisperModelOption{
	{
		ID:        "small",
		Label:     "Small (recommended)",
		FileName:  "ggml-small.bin",
		URL:       "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
		SizeBytes: 488 * 1024 * 1024,
		Quality:   "fast on ordinary CPUs (2-3s per utterance), may misspell technical terms",
		Default:   true,
		// SHA256: HF LFS oid (ggerganov/whisper.cpp /raw/main/ggml-small.bin)
		SHA256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
	},
	{
		ID:        "large-v3-turbo-q5_0",
		Label:     "Large v3 Turbo Q5_0",
		FileName:  "ggml-large-v3-turbo-q5_0.bin",
		URL:       "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
		SizeBytes: 574 * 1024 * 1024,
		Quality:   "best accuracy for Japanese/English; needs a fast multi-core CPU or GPU server",
		// SHA256: HF LFS oid (ggerganov/whisper.cpp /raw/main/ggml-large-v3-turbo-q5_0.bin)
		SHA256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
	},
	{
		ID:        "tiny-q5_1",
		Label:     "Tiny Q5_1",
		FileName:  "ggml-tiny-q5_1.bin",
		URL:       "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny-q5_1.bin",
		SizeBytes: 15 * 1024 * 1024,
		Quality:   "smoke test / very low resource",
		// SHA256: HF LFS oid (ggerganov/whisper.cpp /raw/main/ggml-tiny-q5_1.bin)
		SHA256: "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7",
	},
}

type whisperInstallState struct {
	Installing bool    `json:"installing"`
	Phase      string  `json:"phase,omitempty"`
	Current    string  `json:"current,omitempty"`
	Progress   float64 `json:"progress,omitempty"`
	BytesDone  int64   `json:"bytes_done,omitempty"`
	BytesTotal int64   `json:"bytes_total,omitempty"`
	Error      string  `json:"error,omitempty"`
	UpdatedAt  string  `json:"updated_at,omitempty"`
}

type whisperStatusResponse struct {
	OK                bool                 `json:"ok"`
	Supported         bool                 `json:"supported"`
	Platform          string               `json:"platform"`
	Arch              string               `json:"arch"`
	Managed           bool                 `json:"managed"`
	Installed         bool                 `json:"installed"`
	Running           bool                 `json:"running"`
	ServerURL         string               `json:"server_url,omitempty"`
	Model             string               `json:"model"`
	InstallDir        string               `json:"install_dir,omitempty"`
	BinaryPath        string               `json:"binary_path,omitempty"`
	ModelPath         string               `json:"model_path,omitempty"`
	BinaryVersion     string               `json:"binary_version,omitempty"`
	Install           whisperInstallState  `json:"install"`
	Models            []whisperModelOption `json:"models"`
	ManualOnlyMessage string               `json:"manual_only_message,omitempty"`
}

type whisperInstallRequest struct {
	Model string `json:"model"`
}

func handleJSONDecode(r *http.Request, dst any) error {
	if r.Body == nil {
		return nil
	}
	defer r.Body.Close()
	if r.ContentLength == 0 {
		return nil
	}
	dec := json.NewDecoder(io.LimitReader(r.Body, 1<<20))
	return dec.Decode(dst)
}

func (s *Server) handleWhisperStatus(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	writeJSON(w, s.whisperStatus())
}

func (s *Server) handleWhisperInstall(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	if !whisperManagedSupported() {
		writeJSONError(w, http.StatusBadRequest, "unsupported_platform", whisperUnsupportedDetail())
		return
	}
	var req whisperInstallRequest
	if err := handleJSONDecode(r, &req); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid json")
		return
	}
	model, ok := whisperModelByID(req.Model)
	if !ok {
		model, _ = whisperModelByID(whisperDefaultModelID)
	}
	s.whisperMu.Lock()
	if s.whisperInstall.Installing {
		state := s.whisperInstall
		s.whisperMu.Unlock()
		writeJSON(w, map[string]any{"ok": true, "install": state})
		return
	}
	s.whisperInstall = whisperInstallState{
		Installing: true,
		Phase:      "queued",
		Current:    model.ID,
		UpdatedAt:  time.Now().Format(time.RFC3339),
	}
	s.whisperMu.Unlock()

	s.safeGo("whisper_install", func() {
		if err := s.installManagedWhisper(context.Background(), model); err != nil {
			s.setWhisperInstallError(err)
			return
		}
		s.setWhisperInstallDone(model.ID)
	})
	writeJSON(w, s.whisperStatus())
}

func (s *Server) handleWhisperUninstall(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.whisperMu.Lock()
	installing := s.whisperInstall.Installing
	s.whisperMu.Unlock()
	if installing {
		writeJSONError(w, http.StatusConflict, "install_in_progress", "cannot uninstall while install is running")
		return
	}
	s.stopManagedWhisper()
	baseDir, err := whisperBaseDir()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "internal_error", err.Error())
		return
	}
	if err := os.RemoveAll(baseDir); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "remove_failed", err.Error())
		return
	}
	s.cfgMu.Lock()
	s.cfg.Voice.Whisper.Managed = false
	s.cfg.Voice.Whisper.ServerURL = ""
	s.cfg.Voice.Whisper.ServerPort = 0
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "save_failed", err.Error())
		return
	}
	writeJSON(w, s.whisperStatus())
}

func (s *Server) handleWhisperStart(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	var req whisperInstallRequest
	if err := handleJSONDecode(r, &req); err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", "invalid json")
		return
	}
	if reqModel := strings.TrimSpace(req.Model); reqModel != "" {
		model, ok := whisperModelByID(reqModel)
		if !ok {
			writeJSONError(w, http.StatusBadRequest, "whisper_bad_model", "unknown Whisper model "+reqModel)
			return
		}
		s.cfgMu.Lock()
		changed := s.cfg.Voice.Whisper.Model != model.ID
		s.cfg.Voice.Whisper.Model = model.ID
		s.cfgMu.Unlock()
		if changed {
			if err := s.persistConfig(); err != nil {
				writeJSONError(w, http.StatusInternalServerError, "save_failed", err.Error())
				return
			}
		}
	}
	if _, err := s.ensureManagedWhisper(r.Context()); err != nil {
		var proxyErr whisperProxyError
		if errors.As(err, &proxyErr) {
			writeJSONError(w, proxyErr.status, proxyErr.code, proxyErr.detail)
			return
		}
		writeJSONError(w, http.StatusBadGateway, "whisper_start_failed", err.Error())
		return
	}
	writeJSON(w, s.whisperStatus())
}

func (s *Server) handleWhisperStop(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPost) {
		return
	}
	s.stopManagedWhisper()
	s.cfgMu.Lock()
	if s.cfg.Voice.Whisper.Managed {
		s.cfg.Voice.Whisper.ServerURL = ""
	}
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "save_failed", err.Error())
		return
	}
	writeJSON(w, s.whisperStatus())
}

func (s *Server) whisperStatus() whisperStatusResponse {
	cfg := s.snapshotCfg().Voice.Whisper
	modelID := strings.TrimSpace(cfg.Model)
	if modelID == "" {
		modelID = whisperDefaultModelID
	}
	model, ok := whisperModelByID(modelID)
	if !ok {
		model, _ = whisperModelByID(whisperDefaultModelID)
		modelID = model.ID
	}
	baseDir, _ := whisperBaseDir()
	binaryPath, _ := findWhisperServerExe(filepath.Join(baseDir, "bin"))
	modelPath := filepath.Join(baseDir, "models", model.FileName)
	installed := binaryPath != "" && whisperFileExists(modelPath)

	s.whisperMu.Lock()
	install := s.whisperInstall
	running := s.whisperCmd != nil
	serverURL := s.whisperServerURL
	s.whisperMu.Unlock()
	if serverURL == "" {
		serverURL = strings.TrimSpace(cfg.ServerURL)
	}
	manualOnly := ""
	if !whisperManagedSupported() {
		manualOnly = whisperManualOnlyMessage()
	}
	binaryVersion := whisperReleaseTag
	if entry, ok := whisperBinaryForHost(); ok {
		binaryVersion = entry.Version
	}
	return whisperStatusResponse{
		OK:                true,
		Supported:         whisperManagedSupported(),
		Platform:          runtime.GOOS,
		Arch:              runtime.GOARCH,
		Managed:           cfg.Managed,
		Installed:         installed,
		Running:           running,
		ServerURL:         serverURL,
		Model:             modelID,
		InstallDir:        baseDir,
		BinaryPath:        binaryPath,
		ModelPath:         modelPath,
		BinaryVersion:     binaryVersion,
		Install:           install,
		Models:            publicWhisperModelOptions(),
		ManualOnlyMessage: manualOnly,
	}
}

func (s *Server) installManagedWhisper(ctx context.Context, model whisperModelOption) error {
	baseDir, err := whisperBaseDir()
	if err != nil {
		return err
	}
	binDir := filepath.Join(baseDir, "bin")
	modelDir := filepath.Join(baseDir, "models")
	tmpDir := filepath.Join(baseDir, "tmp")
	for _, dir := range []string{baseDir, binDir, modelDir, tmpDir} {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return err
		}
	}
	if _, err := findWhisperServerExe(binDir); err != nil {
		entry, ok := whisperBinaryForHost()
		if !ok {
			return fmt.Errorf("no managed Whisper binary is available for %s/%s", runtime.GOOS, runtime.GOARCH)
		}
		archivePath := filepath.Join(tmpDir, whisperArchiveName(entry))
		// 抽出成功/失敗いずれの return でも tmp アーカイブを必ず消す。
		// downloadFile の defer は dest+".download" しか消さず、rename 後の
		// archivePath 本体は対象外のため（成功時の明示 Remove と二重になるが無害）。
		defer func() { _ = os.Remove(archivePath) }()
		label := "whisper.cpp " + entry.Version
		s.setWhisperInstallProgress("binary", label, 0, entry.SizeBytes)
		if err := downloadFile(ctx, entry.URL, archivePath, entry.SHA256, func(done, total int64) {
			if total <= 0 {
				total = entry.SizeBytes
			}
			s.setWhisperInstallProgress("binary", label, done, total)
		}); err != nil {
			return err
		}
		if err := extractWhisperArchive(archivePath, binDir, entry); err != nil {
			return err
		}
		_ = os.Remove(archivePath)
		if _, err := findWhisperServerExe(binDir); err != nil {
			return fmt.Errorf("whisper-server not found in release archive")
		}
	}
	// 同梱ランタイム（Windows の VC++ DLL 等）を exe と同一ディレクトリへ配置する。
	// 焼き込み構成や同梱物の無い OS では no-op。
	if err := ensureWhisperRuntime(binDir); err != nil {
		return err
	}
	modelPath := filepath.Join(modelDir, model.FileName)
	if !whisperFileExists(modelPath) {
		if err := ensureDownloadRoom(modelDir, model.SizeBytes+whisperDownloadExtraRoom); err != nil {
			return err
		}
		s.setWhisperInstallProgress("model", model.ID, 0, model.SizeBytes)
		if err := downloadFile(ctx, model.URL, modelPath, model.SHA256, func(done, total int64) {
			if total <= 0 {
				total = model.SizeBytes
			}
			s.setWhisperInstallProgress("model", model.ID, done, total)
		}); err != nil {
			return err
		}
	}
	s.cfgMu.Lock()
	s.cfg.Voice.Whisper.Managed = true
	s.cfg.Voice.Whisper.Model = model.ID
	if s.cfg.Voice.Whisper.TimeoutSeconds <= 0 {
		s.cfg.Voice.Whisper.TimeoutSeconds = 60
	}
	if strings.TrimSpace(s.cfg.Voice.Whisper.Language) == "" {
		s.cfg.Voice.Whisper.Language = "ja"
	}
	s.cfgMu.Unlock()
	return s.persistConfig()
}

func (s *Server) ensureManagedWhisper(ctx context.Context) (config.VoiceWhisperConfig, error) {
	cfg := s.snapshotCfg().Voice.Whisper
	if !cfg.Managed {
		return cfg, nil
	}
	if !whisperManagedSupported() {
		return cfg, whisperProxyError{status: http.StatusBadRequest, code: "unsupported_platform", detail: whisperUnsupportedDetail()}
	}
	modelID := strings.TrimSpace(cfg.Model)
	if modelID == "" {
		modelID = whisperDefaultModelID
	}
	model, ok := whisperModelByID(modelID)
	if !ok {
		return cfg, whisperProxyError{status: http.StatusBadRequest, code: "whisper_bad_model", detail: "unknown Whisper model " + modelID}
	}
	baseDir, err := whisperBaseDir()
	if err != nil {
		return cfg, err
	}
	binDir := filepath.Join(baseDir, "bin")
	binaryPath, err := findWhisperServerExe(binDir)
	if err != nil {
		return cfg, whisperProxyError{status: http.StatusBadRequest, code: "whisper_not_installed", detail: "Whisper server is not installed"}
	}
	// 旧バージョンでインストール済みの環境にも、起動前に同梱ランタイムを補填する。
	if err := ensureWhisperRuntime(binDir); err != nil {
		return cfg, whisperProxyError{status: http.StatusBadGateway, code: "whisper_start_failed", detail: err.Error()}
	}
	modelPath := filepath.Join(baseDir, "models", model.FileName)
	if !fileExists(modelPath) {
		return cfg, whisperProxyError{status: http.StatusBadRequest, code: "whisper_not_installed", detail: "Whisper model is not installed"}
	}
	serverURL, err := s.startManagedWhisper(ctx, cfg, binaryPath, modelPath)
	if err != nil {
		return cfg, err
	}
	cfg.ServerURL = serverURL
	return cfg, nil
}

func (s *Server) startManagedWhisper(ctx context.Context, cfg config.VoiceWhisperConfig, binaryPath, modelPath string) (string, error) {
	s.whisperMu.Lock()
	if s.whisperCmd != nil && s.whisperServerURL != "" {
		url := s.whisperServerURL
		s.whisperMu.Unlock()
		return url, nil
	}
	s.whisperMu.Unlock()

	port := cfg.ServerPort
	if port == 0 || !tcpPortFree(port) {
		var err error
		port, err = pickFreeTCPPort()
		if err != nil {
			return "", err
		}
	}
	serverURL := fmt.Sprintf("http://127.0.0.1:%d", port)
	// ログは常に ~/.many-ai-cli/whisper/whisper-server.log（ドキュメント記載の
	// 書込可能パス）へ。binaryPath 相対だと焼き込みバイナリ
	// (/usr/local/bin/whisper-server) で /usr/local/ 配下になり、非 root の
	// Docker ユーザーでは書けず観測不能になる。
	logPath := filepath.Join(filepath.Dir(filepath.Dir(binaryPath)), "whisper-server.log")
	if baseDir, baseErr := whisperBaseDir(); baseErr == nil {
		_ = os.MkdirAll(baseDir, 0o700)
		logPath = filepath.Join(baseDir, "whisper-server.log")
	}
	logFile, _ := os.OpenFile(logPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0o600)
	// whisper-server の既定スレッド数は 4 固定で、コア数の多いホストでは
	// CPU を使い切れず認識が遅い（10 論理 CPU 環境で 13s → 7s の実測差）。
	cmd := exec.Command(binaryPath, "-m", modelPath, "--host", "127.0.0.1", "--port", strconv.Itoa(port), "-t", strconv.Itoa(runtime.NumCPU()))
	cmd.Dir = filepath.Dir(binaryPath)
	if logFile != nil {
		cmd.Stdout = logFile
		cmd.Stderr = logFile
	}
	// 非 Windows では子を独立プロセスグループにし（Setpgid）、停止時に
	// グループごと kill して孤児を残さない。Windows は JobObject 側で扱う。
	configureWhisperCmd(cmd)
	if err := cmd.Start(); err != nil {
		if logFile != nil {
			_ = logFile.Close()
		}
		return "", whisperProxyError{status: http.StatusBadGateway, code: "whisper_start_failed", detail: err.Error()}
	}
	job, err := attachWhisperProcessJob(cmd)
	if err != nil {
		_ = cmd.Process.Kill()
		// 起動済みの子は必ず Wait で reap する（成功パスの wait goroutine と対称）。
		// Wait 完了後に logFile を閉じ、kill 直前の出力取りこぼしを避ける。
		go func() {
			_ = cmd.Wait()
			if logFile != nil {
				_ = logFile.Close()
			}
		}()
		return "", whisperProxyError{status: http.StatusBadGateway, code: "whisper_start_failed", detail: err.Error()}
	}
	s.whisperMu.Lock()
	s.whisperCmd = cmd
	s.whisperJob = job
	s.whisperServerURL = serverURL
	s.whisperMu.Unlock()
	s.safeGo("whisper_wait", func() {
		err := cmd.Wait()
		if logFile != nil {
			_ = logFile.Close()
		}
		var job whisperProcessJob
		s.whisperMu.Lock()
		if s.whisperCmd == cmd {
			s.whisperCmd = nil
			job = s.whisperJob
			s.whisperJob = 0
			s.whisperServerURL = ""
			if err != nil {
				s.whisperInstall.Error = err.Error()
			}
		}
		s.whisperMu.Unlock()
		closeWhisperProcessJob(job)
	})
	readyCtx, cancel := context.WithTimeout(ctx, whisperServerReadyWait)
	defer cancel()
	if err := waitTCPReady(readyCtx, "127.0.0.1", port); err != nil {
		s.stopManagedWhisper()
		return "", whisperProxyError{status: http.StatusBadGateway, code: "whisper_start_failed", detail: err.Error()}
	}
	s.cfgMu.Lock()
	s.cfg.Voice.Whisper.Managed = true
	s.cfg.Voice.Whisper.ServerURL = serverURL
	s.cfg.Voice.Whisper.ServerPort = port
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("persist managed whisper config failed", "err", err)
	}
	return serverURL, nil
}

func (s *Server) stopManagedWhisper() {
	s.whisperMu.Lock()
	cmd := s.whisperCmd
	job := s.whisperJob
	s.whisperCmd = nil
	s.whisperJob = 0
	s.whisperServerURL = ""
	s.whisperMu.Unlock()
	killWhisperProcess(cmd)
	closeWhisperProcessJob(job)
}

func (s *Server) setWhisperInstallProgress(phase, current string, done, total int64) {
	progress := 0.0
	if total > 0 {
		progress = float64(done) / float64(total)
		if progress > 1 {
			progress = 1
		}
	}
	s.whisperMu.Lock()
	s.whisperInstall = whisperInstallState{
		Installing: true,
		Phase:      phase,
		Current:    current,
		Progress:   progress,
		BytesDone:  done,
		BytesTotal: total,
		UpdatedAt:  time.Now().Format(time.RFC3339),
	}
	s.whisperMu.Unlock()
}

func (s *Server) setWhisperInstallError(err error) {
	s.whisperMu.Lock()
	s.whisperInstall.Installing = false
	s.whisperInstall.Phase = "error"
	s.whisperInstall.Progress = 0
	s.whisperInstall.Error = err.Error()
	s.whisperInstall.UpdatedAt = time.Now().Format(time.RFC3339)
	s.whisperMu.Unlock()
}

func (s *Server) setWhisperInstallDone(modelID string) {
	s.whisperMu.Lock()
	s.whisperInstall = whisperInstallState{
		Installing: false,
		Phase:      "done",
		Current:    modelID,
		Progress:   1,
		UpdatedAt:  time.Now().Format(time.RFC3339),
	}
	s.whisperMu.Unlock()
}

func whisperManagedSupported() bool {
	if bakedWhisperServerPath() != "" {
		return true
	}
	_, ok := whisperBinaryForHost()
	return ok
}

func whisperUnsupportedDetail() string {
	return "managed Whisper is not available on this platform; set an external Whisper server URL instead"
}

func whisperManualOnlyMessage() string {
	return "Managed install is not available on this platform. Use an external Whisper server URL, or run on a supported platform (Windows x64, or a Docker image with a bundled whisper-server)."
}

func whisperModelByID(id string) (whisperModelOption, bool) {
	id = strings.TrimSpace(id)
	if id == "" {
		id = whisperDefaultModelID
	}
	for _, opt := range whisperModelOptions {
		if opt.ID == id {
			return opt, true
		}
	}
	return whisperModelOption{}, false
}

func publicWhisperModelOptions() []whisperModelOption {
	out := make([]whisperModelOption, 0, len(whisperModelOptions))
	for _, opt := range whisperModelOptions {
		opt.HashChecked = opt.SHA256 != ""
		out = append(out, opt)
	}
	return out
}

func whisperBaseDir() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "whisper"), nil
}

func findWhisperServerExe(dir string) (string, error) {
	if p := bakedWhisperServerPath(); p != "" {
		return p, nil
	}
	names := whisperServerNames()
	for _, name := range names {
		path := filepath.Join(dir, name)
		if whisperFileExists(path) {
			return path, nil
		}
	}
	lower := make(map[string]bool, len(names))
	for _, n := range names {
		lower[strings.ToLower(n)] = true
	}
	var found string
	_ = filepath.WalkDir(dir, func(path string, d os.DirEntry, err error) error {
		if err != nil || d.IsDir() || found != "" {
			return nil
		}
		if lower[strings.ToLower(d.Name())] {
			found = path
		}
		return nil
	})
	if found != "" {
		return found, nil
	}
	return "", os.ErrNotExist
}

// whisperServerNames は実行ファイル候補名。host の manifest エントリがあれば
// その ServerNames を、無ければ全 OS 共通の既定候補（焼き込み Linux/mac の
// whisper-server / server を含む）を返す。
func whisperServerNames() []string {
	if entry, ok := whisperBinaryForHost(); ok && len(entry.ServerNames) > 0 {
		return entry.ServerNames
	}
	return []string{"whisper-server.exe", "server.exe", "whisper-server", "server"}
}

func whisperFileExists(path string) bool {
	st, err := os.Stat(path) // #nosec G703 -- path は管理下の whisper バイナリ/モデルの存在確認で外部入力ではない
	return err == nil && !st.IsDir()
}

func downloadFile(ctx context.Context, url, dest, sha256Hex string, progress func(done, total int64)) (err error) {
	// managed Whisper の取得先は https の固定 HF / GitHub release のみ。
	// http へのダウングレードや非 https URL は拒否する（最小ハードニング）。
	if !strings.EqualFold(strings.SplitN(url, ":", 2)[0], "https") {
		return fmt.Errorf("refusing non-https download URL for %s", filepath.Base(dest))
	}
	tmp := dest + ".download"
	_ = os.Remove(tmp)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return err
	}
	req.Header.Set("User-Agent", whisperInstallHTTPUA)
	resp, err := whisperDownloadClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("download %s returned HTTP %d", url, resp.StatusCode)
	}
	if err := os.MkdirAll(filepath.Dir(dest), 0o700); err != nil {
		return err
	}
	f, err := os.OpenFile(tmp, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o600)
	if err != nil {
		return err
	}
	defer func() {
		_ = f.Close()
		if err != nil {
			_ = os.Remove(tmp)
		}
	}()
	// stall ウォッチドッグ: 進捗が一定時間途絶えたら resp.Body を閉じ、ブロック中の
	// Read を error 解除してダウンロードを失敗させる。これにより stall サーバ相手でも
	// goroutine が固着せず、installManagedWhisper が error を返して
	// setWhisperInstallError 経由で Installing=false に戻る（再 install/uninstall 可能）。
	watch := whisperNewDownloadWatchdog(whisperDownloadStallTimeout, func() { _ = resp.Body.Close() })
	defer watch.stop()
	hasher := sha256.New()
	writer := io.MultiWriter(f, hasher)
	buf := make([]byte, 256*1024)
	var done int64
	total := resp.ContentLength
	for {
		n, readErr := resp.Body.Read(buf)
		if n > 0 {
			watch.tick()
			if _, err = writer.Write(buf[:n]); err != nil {
				return err
			}
			done += int64(n)
			if progress != nil {
				progress(done, total)
			}
		}
		if readErr == io.EOF {
			break
		}
		if readErr != nil {
			if watch.stalled() {
				return fmt.Errorf("download %s stalled: no data for %s", filepath.Base(dest), whisperDownloadStallTimeout)
			}
			return readErr
		}
	}
	if err = f.Sync(); err != nil {
		return err
	}
	// SHA256 検証: sha256Hex が設定されている場合のみ照合する。
	// 不一致は即 error を返し、defer の os.Remove(tmp) が tmp ファイルを破棄する。
	// sha256Hex が空のエントリはここをスキップして検証なしでインストールを続行する
	// （whisperModelOptions の全モデルには SHA256 を設定済み。将来モデルを追加する
	// 場合は必ず HF LFS oid を確認して設定すること）。
	if sha256Hex != "" {
		got := hex.EncodeToString(hasher.Sum(nil))
		if !strings.EqualFold(got, sha256Hex) {
			return fmt.Errorf("sha256 mismatch for %s: got %s, want %s", filepath.Base(dest), got, strings.ToLower(sha256Hex))
		}
	}
	if err = f.Close(); err != nil {
		return err
	}
	return os.Rename(tmp, dest)
}

// whisperDownloadWatchdog は本文読込の idle/stall を監視する。timeout の間
// tick() が一度も来なければ onStall を一度だけ呼ぶ（典型的には resp.Body.Close）。
// tick() で計時をリセットし、stop() で監視を終了する。すべてのメソッドは
// 並行呼び出し安全。
type whisperDownloadWatchdog struct {
	timeout time.Duration
	onStall func()
	mu      sync.Mutex
	timer   *time.Timer
	fired   bool
	done    bool
}

func whisperNewDownloadWatchdog(timeout time.Duration, onStall func()) *whisperDownloadWatchdog {
	w := &whisperDownloadWatchdog{timeout: timeout, onStall: onStall}
	w.timer = time.AfterFunc(timeout, w.trip)
	return w
}

func (w *whisperDownloadWatchdog) trip() {
	w.mu.Lock()
	if w.done || w.fired {
		w.mu.Unlock()
		return
	}
	w.fired = true
	onStall := w.onStall
	w.mu.Unlock()
	if onStall != nil {
		onStall()
	}
}

// tick は進捗があったことを記録し、stall タイマーをリセットする。
func (w *whisperDownloadWatchdog) tick() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.done || w.fired || w.timer == nil {
		return
	}
	w.timer.Reset(w.timeout)
}

// stalled は stall を検出して onStall を発火済みかを返す。
func (w *whisperDownloadWatchdog) stalled() bool {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.fired
}

func (w *whisperDownloadWatchdog) stop() {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.done = true
	if w.timer != nil {
		w.timer.Stop()
	}
}

func extractZip(zipPath, destDir string) error {
	zr, err := zip.OpenReader(zipPath)
	if err != nil {
		return err
	}
	defer zr.Close()
	cleanDest, err := filepath.Abs(destDir)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(cleanDest, 0o700); err != nil {
		return err
	}
	for _, f := range zr.File {
		target := filepath.Join(cleanDest, f.Name)
		cleanTarget, err := filepath.Abs(target)
		if err != nil {
			return err
		}
		if cleanTarget != cleanDest && !strings.HasPrefix(cleanTarget, cleanDest+string(os.PathSeparator)) {
			return fmt.Errorf("zip entry outside destination: %s", f.Name)
		}
		if f.FileInfo().IsDir() {
			if err := os.MkdirAll(cleanTarget, 0o700); err != nil {
				return err
			}
			continue
		}
		if err := os.MkdirAll(filepath.Dir(cleanTarget), 0o700); err != nil {
			return err
		}
		rc, err := f.Open()
		if err != nil {
			return err
		}
		out, err := os.OpenFile(cleanTarget, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o700)
		if err != nil {
			_ = rc.Close()
			return err
		}
		_, copyErr := io.Copy(out, rc)
		closeErr := out.Close()
		_ = rc.Close()
		if copyErr != nil {
			return copyErr
		}
		if closeErr != nil {
			return closeErr
		}
	}
	return nil
}

func pickFreeTCPPort() (int, error) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return 0, err
	}
	defer ln.Close()
	return ln.Addr().(*net.TCPAddr).Port, nil
}

func tcpPortFree(port int) bool {
	ln, err := net.Listen("tcp", net.JoinHostPort("127.0.0.1", strconv.Itoa(port)))
	if err != nil {
		return false
	}
	_ = ln.Close()
	return true
}

func waitTCPReady(ctx context.Context, host string, port int) error {
	addr := net.JoinHostPort(host, strconv.Itoa(port))
	ticker := time.NewTicker(150 * time.Millisecond)
	defer ticker.Stop()
	for {
		conn, err := net.DialTimeout("tcp", addr, 300*time.Millisecond)
		if err == nil {
			_ = conn.Close()
			return nil
		}
		select {
		case <-ctx.Done():
			return fmt.Errorf("timed out waiting for Whisper server to become ready on %s: %w", addr, ctx.Err())
		case <-ticker.C:
		}
	}
}

// whisperArchiveName はダウンロード先のファイル名を URL から導く。
func whisperArchiveName(entry whisperBinaryEntry) string {
	url := strings.TrimRight(entry.URL, "/")
	if i := strings.LastIndex(url, "/"); i >= 0 && i+1 < len(url) {
		if name := url[i+1:]; name != "" {
			return name
		}
	}
	if whisperArchiveKind(entry) == "tar.gz" {
		return "whisper-download.tar.gz"
	}
	return "whisper-download.zip"
}

// whisperArchiveKind は Archive 指定が無ければ URL 拡張子から種別を推定する。
func whisperArchiveKind(entry whisperBinaryEntry) string {
	if entry.Archive != "" {
		return entry.Archive
	}
	u := strings.ToLower(entry.URL)
	if strings.HasSuffix(u, ".tar.gz") || strings.HasSuffix(u, ".tgz") {
		return "tar.gz"
	}
	return "zip"
}

// extractWhisperArchive はアーカイブを bin/ へ展開する。KeepFromArchive が
// 非空のときはその basename だけを平坦展開する（公式 Windows zip は Release/
// 配下に21ファイル入りのため必要なものだけ取り出す）。
func extractWhisperArchive(archivePath, destDir string, entry whisperBinaryEntry) error {
	var err error
	switch whisperArchiveKind(entry) {
	case "tar.gz":
		err = extractTarGzSelected(archivePath, destDir, entry.KeepFromArchive)
	default:
		if len(entry.KeepFromArchive) > 0 {
			err = extractZipSelected(archivePath, destDir, entry.KeepFromArchive)
		} else {
			err = extractZip(archivePath, destDir)
		}
	}
	if err != nil {
		return err
	}
	// KeepFromArchive 指定時は、想定ファイルが全て展開されたか確認する。
	// 将来 zip のレイアウト/命名が変わったら、start 時の不可解な DLL ロード失敗
	// ではなく install 時に明示エラーで落とす（選択抽出は未一致を黙って skip するため）。
	var missing []string
	for _, name := range entry.KeepFromArchive {
		base := filepath.Base(name)
		if !whisperFileExists(filepath.Join(destDir, base)) {
			missing = append(missing, base)
		}
	}
	if len(missing) > 0 {
		return fmt.Errorf("archive %s is missing expected files: %s", whisperArchiveName(entry), strings.Join(missing, ", "))
	}
	return nil
}

func extractZipSelected(zipPath, destDir string, keep []string) error {
	zr, err := zip.OpenReader(zipPath)
	if err != nil {
		return err
	}
	defer zr.Close()
	if err := os.MkdirAll(destDir, 0o700); err != nil {
		return err
	}
	keepSet := whisperKeepSet(keep)
	for _, f := range zr.File {
		if f.FileInfo().IsDir() {
			continue
		}
		base := filepath.Base(f.Name)
		if keepSet != nil && !keepSet[strings.ToLower(base)] {
			continue
		}
		rc, err := f.Open()
		if err != nil {
			return err
		}
		err = writeWhisperFlatFile(filepath.Join(destDir, base), rc)
		_ = rc.Close()
		if err != nil {
			return err
		}
	}
	return nil
}

func extractTarGzSelected(tarPath, destDir string, keep []string) error {
	f, err := os.Open(tarPath)
	if err != nil {
		return err
	}
	defer f.Close()
	gz, err := gzip.NewReader(f)
	if err != nil {
		return err
	}
	defer gz.Close()
	if err := os.MkdirAll(destDir, 0o700); err != nil {
		return err
	}
	keepSet := whisperKeepSet(keep)
	tr := tar.NewReader(gz)
	for {
		hdr, err := tr.Next()
		if err == io.EOF {
			break
		}
		if err != nil {
			return err
		}
		if hdr.FileInfo().IsDir() {
			continue
		}
		base := filepath.Base(hdr.Name)
		if keepSet != nil && !keepSet[strings.ToLower(base)] {
			continue
		}
		if err := writeWhisperFlatFile(filepath.Join(destDir, base), tr); err != nil {
			return err
		}
	}
	return nil
}

func whisperKeepSet(keep []string) map[string]bool {
	if len(keep) == 0 {
		return nil
	}
	set := make(map[string]bool, len(keep))
	for _, k := range keep {
		set[strings.ToLower(filepath.Base(k))] = true
	}
	return set
}

func writeWhisperFlatFile(dest string, src io.Reader) error {
	if err := os.MkdirAll(filepath.Dir(dest), 0o700); err != nil {
		return err
	}
	out, err := os.OpenFile(dest, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0o700)
	if err != nil {
		return err
	}
	_, copyErr := io.Copy(out, src)
	closeErr := out.Close()
	if copyErr != nil {
		return copyErr
	}
	return closeErr
}

// ensureWhisperRuntime は同梱ランタイム（Windows の VC++ DLL 等）を bin/ へ
// 配置する。冪等（既存はスキップ）で、同梱物の無い OS/arch では no-op。
func ensureWhisperRuntime(binDir string) error {
	return whisperruntime.Ensure(binDir, runtime.GOOS+"-"+runtime.GOARCH)
}
