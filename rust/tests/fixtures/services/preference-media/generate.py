#!/usr/bin/env python3
"""Observe pinned media handlers; no server, account, network or real home."""
import hashlib, json, os
from pathlib import Path
import subprocess, sys, tempfile
HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[4]
SHA = '21d0bc7935a2c4696fb89ccff2e324157a528c2d'
GO = os.environ['GO']  # Caller supplies the pinned Go executable; no machine paths.
if 'go1.26.8 ' not in subprocess.check_output([GO, 'version'], text=True):
    raise SystemExit('oracle requires pinned Go 1.26.8')
sources = {}
def source(path):
    if path not in sources:
        data = subprocess.check_output(['git', 'show', f'{SHA}:{path}'], cwd=ROOT)
        if (ROOT/path).read_bytes().replace(b'\r\n',b'\n') != data.replace(b'\r\n',b'\n'):
            raise SystemExit('baseline source changed: '+path)
        sources[path] = data.decode()
    return sources[path]
def decl(path, marker):
    text=source(path); start=text.index(marker)
    return text[start:text.index('\n}',start)+2]+'\n'
for directory in ['internal/config','internal/securefile','internal/wslutil']:
    for path in sorted((ROOT/directory).glob('*.go')):
        if not path.name.endswith('_test.go'): source(path.relative_to(ROOT).as_posix())
parts=[]
for name in ['notifySoundCustomPath','avatarUploadPath','(s *Server) handleUserPrefsAvatarUpload','(s *Server) handleUserPrefsNotifySoundCustom(', 'notifySoundAllowed','avatarImageAllowed','(s *Server) handleUserPrefsNotifySoundCustomGet','(s *Server) handleUserPrefsNotifySoundCustomPut']:
    parts.append(decl('internal/hub/user_prefs_handlers.go','func '+name))
for name in ['(s *Server) handleAvatar','pathExistsCandidateKey']:
    parts.append(decl('internal/hub/misc_handlers.go','func '+name))
parts.append(decl('internal/hub/server.go','func (s *Server) persistConfig'))
for name in ['requireMethodOneOf','writeJSON(', 'writeJSONStatus','writeJSONError','errorDetail']:
    parts.append(decl('internal/hub/http_helpers.go','func '+name))
parts.append(decl('internal/hub/http_helpers.go','type httpErrorResp struct'))
# Verify constants used by the observed handlers instead of guessing their sizes.
helpers=source('internal/hub/http_helpers.go')
for line in ['avatarMaxBytes = 5 * 1024 * 1024','notifySoundMaxBytes = 2 * 1024 * 1024']:
    assert line in helpers
header='''//go:build ignore

// Generated commit-pinned source observer, never application code.
package main
import("crypto/sha256";"encoding/base64";"encoding/hex";"encoding/json";"fmt";"io";"log/slog";"net/http";"net/http/httptest";"os";"path/filepath";"runtime";"strings";"sync";"many-ai-cli/internal/config";"gopkg.in/yaml.v3")
const avatarMaxBytes=5*1024*1024
const notifySoundMaxBytes=2*1024*1024
type Server struct{cfg *config.Config;cfgMu sync.Mutex;logger *slog.Logger}
func(s *Server)guard(w http.ResponseWriter,r *http.Request,methods ...string)bool{return requireMethodOneOf(w,r,methods...)}
func must(e error){if e!=nil{panic(e)}}
'''
(HERE/'oracle.go').write_text(header+'\n'.join(parts)+(HERE/'harness.go.txt').read_text())
with tempfile.TemporaryDirectory(prefix='preference-media-observer-') as temporary:
    env=dict(os.environ,GOMODCACHE=os.environ['GOMODCACHE'], GOCACHE=os.environ['GOCACHE'], HOME=temporary,USERPROFILE=temporary,GOTOOLCHAIN='local',GOPROXY='off',GOSUMDB='off',GOTELEMETRY='off',GOENV='off',TZ='UTC')
    run=subprocess.run([GO,'run',str(HERE/'oracle.go'),str(HERE/'cases.json')],cwd=ROOT,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    sys.stderr.buffer.write(run.stderr)
    if run.returncode: raise SystemExit(run.returncode)
    platform=subprocess.check_output([GO,'env','GOOS','GOARCH'],cwd=ROOT,env=env,text=True).splitlines()
    goroot=subprocess.check_output([GO,'env','GOROOT'],cwd=ROOT,env=env,text=True).strip()
    sniff=Path(goroot,'src/net/http/sniff.go').read_bytes()
    result={'baseline':SHA,'go_version':subprocess.check_output([GO,'version'],text=True).strip(),'observer_platform':dict(zip(['goos','goarch'],platform)), 'go_sniff_sha256':hashlib.sha256(sniff).hexdigest(),'source_sha256':{p:hashlib.sha256(s.encode()).hexdigest() for p,s in sorted(sources.items())},'cases':json.loads(run.stdout)}
    Path(sys.argv[1]).write_text(json.dumps(result,ensure_ascii=True,indent=2)+'\n')
