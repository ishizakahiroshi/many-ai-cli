//go:build ignore

// Generated commit-pinned source observer, never application code.
package main
import("crypto/sha256";"encoding/base64";"encoding/hex";"encoding/json";"fmt";"io";"log/slog";"net/http";"net/http/httptest";"os";"path/filepath";"runtime";"strings";"sync";"many-ai-cli/internal/config";"gopkg.in/yaml.v3")
const avatarMaxBytes=5*1024*1024
const notifySoundMaxBytes=2*1024*1024
type Server struct{cfg *config.Config;cfgMu sync.Mutex;logger *slog.Logger}
func(s *Server)guard(w http.ResponseWriter,r *http.Request,methods ...string)bool{return requireMethodOneOf(w,r,methods...)}
func must(e error){if e!=nil{panic(e)}}
func notifySoundCustomPath() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "notify_sound_custom.bin"), nil
}

func avatarUploadPath() (string, error) {
	dir, err := config.Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "user_avatar.bin"), nil
}

func (s *Server) handleUserPrefsAvatarUpload(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodPut, http.MethodDelete) {
		return
	}
	switch r.Method {
	case http.MethodPut:
		path, err := avatarUploadPath()
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "home_dir_error", errorDetail("home dir error", err))
			return
		}
		r.Body = http.MaxBytesReader(w, r.Body, avatarMaxBytes)
		data, err := io.ReadAll(r.Body)
		if err != nil {
			writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("read body error", err))
			return
		}
		// MIME は client header を信用せず sniff する。image/*（SVG 以外）のみ許可。
		if !avatarImageAllowed(http.DetectContentType(data)) {
			writeJSONError(w, http.StatusUnsupportedMediaType, "bad_request", "image/png, image/jpeg, image/gif, or image/webp required")
			return
		}
		if err := os.WriteFile(path, data, 0o600); err != nil {
			writeJSONError(w, http.StatusInternalServerError, "write_error", errorDetail("write error", err))
			return
		}
		s.cfgMu.Lock()
		s.cfg.UserPrefs.Avatar = path
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			s.logger.Warn("save config failed", "err", err)
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	case http.MethodDelete:
		s.cfgMu.Lock()
		s.cfg.UserPrefs.Avatar = ""
		s.cfgMu.Unlock()
		if err := s.persistConfig(); err != nil {
			s.logger.Warn("save config failed", "err", err)
			writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
			return
		}
		writeJSON(w, map[string]bool{"ok": true})
	}
}

func (s *Server) handleUserPrefsNotifySoundCustom(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet, http.MethodPut) {
		return
	}
	switch r.Method {
	case http.MethodGet:
		s.handleUserPrefsNotifySoundCustomGet(w, r)
	case http.MethodPut:
		s.handleUserPrefsNotifySoundCustomPut(w, r)
	}
}

func notifySoundAllowed(mime string) bool {
	mime = strings.TrimSpace(strings.ToLower(mime))
	if mime == "" {
		return false
	}
	if i := strings.Index(mime, ";"); i >= 0 {
		mime = strings.TrimSpace(mime[:i])
	}
	return strings.HasPrefix(mime, "audio/")
}

func avatarImageAllowed(mime string) bool {
	mime = strings.TrimSpace(strings.ToLower(mime))
	if mime == "" {
		return false
	}
	if i := strings.Index(mime, ";"); i >= 0 {
		mime = strings.TrimSpace(mime[:i])
	}
	switch mime {
	case "image/png", "image/jpeg", "image/gif", "image/webp":
		return true
	default:
		return false
	}
}

func (s *Server) handleUserPrefsNotifySoundCustomGet(w http.ResponseWriter, _ *http.Request) {
	path, err := notifySoundCustomPath()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "home_dir_error", errorDetail("home dir error", err))
		return
	}
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		writeJSONError(w, http.StatusNotFound, "not_found", "not found")
		return
	}
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "read_error", errorDetail("read error", err))
		return
	}
	s.cfgMu.Lock()
	mime := s.cfg.UserPrefs.NotifySound.CustomMime
	s.cfgMu.Unlock()
	// 過去に attacker-controlled な MIME（text/html 等）が保存されている可能性が
	// あるため、audio/* 以外は application/octet-stream に正規化して配信する
	// （HTML として inline render される経路を閉じる）。
	if !notifySoundAllowed(mime) {
		mime = "application/octet-stream"
	}
	w.Header().Set("Content-Type", mime)
	w.Header().Set("X-Content-Type-Options", "nosniff")
	// audio/* のみ inline 再生を許可（同 URL を別文脈で開かれても document として
	// レンダリングされないよう Content-Disposition でも明示する）。
	w.Header().Set("Content-Disposition", `inline; filename="notify-sound"`)
	_, _ = w.Write(data)
}

func (s *Server) handleUserPrefsNotifySoundCustomPut(w http.ResponseWriter, r *http.Request) {
	path, err := notifySoundCustomPath()
	if err != nil {
		writeJSONError(w, http.StatusInternalServerError, "home_dir_error", errorDetail("home dir error", err))
		return
	}
	r.Body = http.MaxBytesReader(w, r.Body, notifySoundMaxBytes)
	data, err := io.ReadAll(r.Body)
	if err != nil {
		writeJSONError(w, http.StatusBadRequest, "bad_request", errorDetail("read body error", err))
		return
	}
	// MIME は client header を信用せず、バイト列から sniff する（http.DetectContentType
	// は OGG/WAV/MP3 の magic byte を audio/* に判定する）。検出結果が audio/* で
	// ない場合は Content-Type ヘッダが audio/* でも拒否する（fail-open しない）。
	mime := http.DetectContentType(data)
	if !notifySoundAllowed(mime) {
		writeJSONError(w, http.StatusUnsupportedMediaType, "bad_request", "audio/* MIME type required")
		return
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		writeJSONError(w, http.StatusInternalServerError, "write_error", errorDetail("write error", err))
		return
	}
	s.cfgMu.Lock()
	s.cfg.UserPrefs.NotifySound.CustomFile = path
	s.cfg.UserPrefs.NotifySound.CustomMime = mime
	s.cfgMu.Unlock()
	if err := s.persistConfig(); err != nil {
		s.logger.Warn("save config failed", "err", err)
		writeJSONError(w, http.StatusInternalServerError, "save_failed", errorDetail("save failed", err))
		return
	}
	writeJSON(w, map[string]bool{"ok": true})
}

func (s *Server) handleAvatar(w http.ResponseWriter, r *http.Request) {
	if !s.guard(w, r, http.MethodGet) {
		return
	}
	s.cfgMu.Lock()
	path := s.cfg.UserPrefs.Avatar
	s.cfgMu.Unlock()
	if path == "" || strings.HasPrefix(path, "http://") || strings.HasPrefix(path, "https://") {
		http.NotFound(w, r)
		return
	}
	// avatar はアップロード専用パス（~/.many-ai-cli/user_avatar.bin）の配信のみ許可する。
	// 任意ローカルパスを許すと config.yaml（Token / RemotePINHash / AuthCookieSecret 等を
	// 含む）など ~/.many-ai-cli 配下の機密ファイルを read できてしまうため、固定パスとの
	// 完全一致のみ通す（任意ファイル read プリミティブ化を防ぐ。AuthCookieSecret 漏洩は
	// PIN cookie 偽造による PIN 境界の恒久バイパスにつながる）。
	uploadPath, err := avatarUploadPath()
	if err != nil {
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}
	if pathExistsCandidateKey(path) != pathExistsCandidateKey(uploadPath) {
		http.NotFound(w, r)
		return
	}
	data, err := os.ReadFile(uploadPath)
	if err != nil {
		http.NotFound(w, r)
		return
	}
	ct := http.DetectContentType(data)
	if !avatarImageAllowed(ct) {
		ct = "application/octet-stream"
	}
	w.Header().Set("Content-Type", ct)
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.Header().Set("Cache-Control", "max-age=3600")
	_, _ = w.Write(data)
}

func pathExistsCandidateKey(path string) string {
	key := filepath.Clean(path)
	if runtime.GOOS == "windows" {
		key = strings.ToLower(key)
	}
	return key
}

func (s *Server) persistConfig() error {
	s.cfgMu.Lock()
	defer s.cfgMu.Unlock()
	return config.Save(s.cfg.Clone())
}

func requireMethodOneOf(w http.ResponseWriter, r *http.Request, methods ...string) bool {
	for _, method := range methods {
		if r.Method == method {
			return true
		}
	}
	writeJSONError(w, http.StatusMethodNotAllowed, "method_not_allowed", "method not allowed")
	return false
}

func writeJSON(w http.ResponseWriter, v any) {
	writeJSONStatus(w, http.StatusOK, v)
}

func writeJSONStatus(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	if status != http.StatusOK {
		w.WriteHeader(status)
	}
	_ = json.NewEncoder(w).Encode(v)
}

func writeJSONError(w http.ResponseWriter, status int, code, detail string) {
	if code == "" {
		code = "error"
	}
	if detail == "" {
		detail = http.StatusText(status)
	}
	writeJSONStatus(w, status, httpErrorResp{
		OK:     false,
		Error:  code,
		Detail: detail,
	})
}

func errorDetail(prefix string, err error) string {
	if err == nil {
		return prefix
	}
	if prefix == "" {
		return err.Error()
	}
	return fmt.Sprintf("%s: %v", prefix, err)
}

type httpErrorResp struct {
	OK     bool   `json:"ok"`
	Error  string `json:"error"`
	Detail string `json:"detail,omitempty"`
}
type payload struct { Hex string `json:"hex"`; PadTo int `json:"pad_to"`; Directory bool `json:"directory"`; WriteOnly bool `json:"write_only"` }
func(p payload) bytes()[]byte{b,e:=hex.DecodeString(p.Hex);must(e);if p.PadTo>len(b){b=append(b,make([]byte,p.PadTo-len(b))...)};return b}
type input struct{
 Name string `json:"name"`; Path string `json:"path"`; Method string `json:"method"`; Body payload `json:"body"`; ContentType string `json:"content_type"`
 Initial config.UserPrefs `json:"initial"`; Media map[string]payload `json:"media"`; FailSave bool `json:"fail_save"`; MissingRoot bool `json:"missing_root"`
}
func digest(b []byte)map[string]any{return map[string]any{"len":len(b),"sha256":fmt.Sprintf("%x",sha256.Sum256(b))}}
func main(){
 raw,e:=os.ReadFile(os.Args[1]);must(e);var cases []input;must(json.Unmarshal(raw,&cases));rows:=[]any{}
 for _,c:=range cases{
  home,e:=os.MkdirTemp("","preference-media-owned-");must(e);must(os.Setenv("HOME",home));must(os.Setenv("USERPROFILE",home))
  root:=filepath.Join(home,".many-ai-cli");if !c.MissingRoot{must(os.MkdirAll(root,0700))}
  encodedRoot,e:=json.Marshal(root);must(e);rootJSON:=string(encodedRoot[1:len(encodedRoot)-1])
  expand:=func(s string)string{return strings.ReplaceAll(strings.ReplaceAll(s,`\u003croot\u003e`,rootJSON),"<root>",rootJSON)}
  b,e:=json.Marshal(c.Initial);must(e);must(json.Unmarshal([]byte(expand(string(b))),&c.Initial))
  cfg:=&config.Config{UserPrefs:c.Initial};s:=&Server{cfg:cfg,logger:slog.New(slog.NewTextHandler(io.Discard,nil))}
  names:=map[string]string{"avatar":"user_avatar.bin","sound":"notify_sound_custom.bin"}
  for key,p:=range c.Media{path:=filepath.Join(root,names[key]);if p.Directory{must(os.Mkdir(path,0700))}else{must(os.WriteFile(path,p.bytes(),0600));if p.WriteOnly&&runtime.GOOS!="windows"{must(os.Chmod(path,0200))}}}
  if c.FailSave{must(os.Mkdir(filepath.Join(root,"config.yaml"),0700))}
  r:=httptest.NewRequest(c.Method,"http://127.0.0.1:49962"+c.Path,strings.NewReader(string(c.Body.bytes())));r.Header.Set("Content-Type",c.ContentType)
  w:=httptest.NewRecorder();switch c.Path{case "/api/avatar":s.handleAvatar(w,r);case "/api/user-prefs/avatar":s.handleUserPrefsAvatarUpload(w,r);case "/api/user-prefs/notify-sound-custom":s.handleUserPrefsNotifySoundCustom(w,r);default:panic("unknown fixture route")}
  normalize:=func(value any)any{bytes,e:=json.Marshal(value);must(e);var out any;must(json.Unmarshal([]byte(strings.ReplaceAll(string(bytes),rootJSON,"<root>")),&out));return out}
  project:=func(p config.UserPrefs)any{return normalize(map[string]any{"avatar":p.Avatar,"notify_sound":p.NotifySound})}
  var body any;var detail string
  if w.Header().Get("Content-Type")=="application/json"{must(json.Unmarshal(w.Body.Bytes(),&body));if w.Code==500{m:=body.(map[string]any);detail=m["detail"].(string);prefix:=strings.SplitN(detail,":",2)[0];m["detail"]=prefix+": <owned filesystem error>"}}else if w.Body.Len()<1024{body=map[string]any{"base64":base64.StdEncoding.EncodeToString(w.Body.Bytes())}}else{body=digest(w.Body.Bytes())}
  for key,p:=range c.Media{if p.WriteOnly&&runtime.GOOS!="windows"{must(os.Chmod(filepath.Join(root,names[key]),0600))}}
  files:=map[string]any{};for key,name:=range names{path:=filepath.Join(root,name);if b,e:=os.ReadFile(path);e==nil{files[key]=digest(b)}else if st,e:=os.Stat(path);e==nil&&st.IsDir(){files[key]="directory"}else{files[key]=nil}}
  var saved any;if b,e:=os.ReadFile(filepath.Join(root,"config.yaml"));e==nil{var loaded config.Config;must(yaml.Unmarshal(b,&loaded));saved=project(loaded.UserPrefs)}
  headers:=map[string]string{};for _,name:=range []string{"Content-Type","Cache-Control","Content-Disposition"}{headers[name]=w.Header().Get(name)}
  rows=append(rows,map[string]any{"name":c.Name,"status":w.Code,"body":body,"headers":headers,"runtime":project(cfg.UserPrefs),"saved":saved,"files":files,"source_detail_prefix":strings.SplitN(detail,":",2)[0]})
  must(os.RemoveAll(home))
 }
 enc:=json.NewEncoder(os.Stdout);enc.SetIndent("","  ");must(enc.Encode(rows))
}
