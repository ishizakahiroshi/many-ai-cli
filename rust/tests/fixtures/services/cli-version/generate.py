#!/usr/bin/env python3
"""Observe exact pinned CLI-version Go caller with explicit process/clock seams."""
import hashlib,json,os,pathlib,subprocess,sys,tempfile
HERE=pathlib.Path(__file__).resolve().parent
ROOT=HERE.parents[4]
SHA='21d0bc7935a2c4696fb89ccff2e324157a528c2d'
SOURCES={}
def source(path):
 if path not in SOURCES:
  raw=subprocess.check_output(['git','show',f'{SHA}:{path}'],cwd=ROOT)
  if (ROOT/path).read_bytes().replace(b'\r\n',b'\n')!=raw.replace(b'\r\n',b'\n'):raise RuntimeError('frozen source differs: '+path)
  SOURCES[path]=raw.decode()
 return SOURCES[path]
def decl(path,start):
 text=source(path);i=text.index(start);return text[i:text.index('\n}',i)+2]+'\n'
def function(path,name):return decl(path,'func '+name)
def structure(path,name):return decl(path,'type '+name+' struct {')
def write(root,path,text):
 p=root/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(text)
with tempfile.TemporaryDirectory(prefix='cli-version-oracle-') as tmp:
 root=pathlib.Path(tmp)
 write(root,'go.mod','module many-ai-cli\n\ngo 1.26\n')
 provider='package provider\nimport "sort"\n'
 provider+='type LaunchDefinition struct{Executable string `json:"executable"`;ExecutableCandidates []string `json:"executable_candidates"`}\n'
 provider+=structure('internal/provider/schema.go','UpdateDefinition')
 provider+='type Definition struct{ID string `json:"id"`;Enabled *bool `json:"enabled"`;Launch *LaunchDefinition `json:"launch"`;Update *UpdateDefinition `json:"update"`}\ntype EffectiveDefinition struct{Definition}\ntype Summary struct{ID string;Enabled bool}\n'
 provider+='type Registry struct{definitions map[string]EffectiveDefinition; summaries []Summary}\nfunc cloneEffectiveDefinition(d EffectiveDefinition)EffectiveDefinition{return d}\n'
 for n in ['(r *Registry) Lookup','(r *Registry) List']:provider+=function('internal/provider/registry.go',n)
 provider+=function('internal/provider/schema.go','sortIDs')
 # sortIDs references the frozen builtin ordering function.
 provider+=decl('internal/provider/schema.go','var BuiltinProviderIDs =')+function('internal/provider/schema.go','builtinOrder')
 provider+='const defaultVersionArg="--version"\n'+function('internal/provider/update.go','ResolveVersionArgs')
 provider+='func New(ds []Definition)*Registry{r:=&Registry{definitions:map[string]EffectiveDefinition{}};ids:=[]string{};for _,d:=range ds{ids=append(ids,d.ID);r.definitions[d.ID]=EffectiveDefinition{d}};sortIDs(ids);for _,id:=range ids{d:=r.definitions[id];r.summaries=append(r.summaries,Summary{id,d.Enabled==nil||*d.Enabled})};return r}\n'
 write(root,'internal/provider/source.go',provider)
 write(root,'internal/execpath/source.go','package execpath\n'+function('internal/execpath/execpath_other.go','Resolve'))
 hub='package main\nimport("bytes";"context";"encoding/json";"fmt";"net/http";"strings";"sync";"time";"many-ai-cli/internal/provider";"many-ai-cli/internal/execpath")\n'
 hub+='const cliVersionOutputCap=8192;const cliVersionMaxConcurrency=8;const jsonBodyMaxBytes=1024*1024;var cliVersionCheckTimeout=10*time.Second\n'
 for name in ['cliVersionResult','cliVersionResponse','cliVersionState','cliVersionInflight','cliVersionRequest','cappedWriter']:hub+=structure('internal/hub/cli_version.go',name)
 for name in ['newCLIVersionState','(s *cliVersionState) run','(s *cliVersionState) lastResult','checkCLIVersions','allCLIVersionProviderIDs','checkOneCLIVersion','selectCLIVersionExecutablePath','(w *cappedWriter) Write','(w *cappedWriter) String','firstNonEmptyLine','normalizeCLIVersionProviderIDs','(s *Server) handleCLIVersions(','(s *Server) handleCLIVersionsGet','(s *Server) handleCLIVersionsPost']:
  text=function('internal/hub/cli_version.go',name)
  text=text.replace('time.Now().UTC().Format(time.RFC3339)','oracleTime.UTC().Format(time.RFC3339)').replace('os.Stat(exe)','oracleStat(exe)')
  hub+=text
 for name in ['requireMethodOneOf','writeJSON(','writeJSONStatus','writeJSONError']:hub+=function('internal/hub/http_helpers.go',name)
 hub+=structure('internal/hub/http_helpers.go','httpErrorResp')
 write(root,'source.go',hub)
 write(root,'main.go',(HERE/'oracle.go').read_text())
 for directory in ['home','temporary']:(root/directory).mkdir()
 env=dict(os.environ,HOME=str(root/'home'),TMPDIR=str(root/'temporary'),GOTOOLCHAIN='local',GOPROXY='off',GOSUMDB='off',GO111MODULE='on',TZ='UTC')
 executable=os.environ.get('CLI_VERSION_GO','/workspace/shared/many-ai-rust-tools/go/bin/go')
 run=subprocess.run([executable,'run','.',str(HERE/'cases.json')],cwd=root,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 sys.stderr.buffer.write(run.stderr)
 if run.returncode:raise SystemExit(run.returncode)
 out={'baseline':SHA,'source_sha256':{p:hashlib.sha256(s.encode()).hexdigest() for p,s in SOURCES.items()},'cases':json.loads(run.stdout)}
 pathlib.Path(sys.argv[1]).write_text(json.dumps(out,ensure_ascii=True,indent=2)+'\n')
