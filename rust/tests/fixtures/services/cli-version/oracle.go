package main

import (
 "context"
 "encoding/json"
 "errors"
 "io/fs"
 "io"
 "net/http"
 "net/http/httptest"
 "os"
 "sort"
 "strings"
 "sync"
 "time"
 "many-ai-cli/internal/provider"
)

type process struct { Output []byte `json:"output"`; ExitCode int `json:"exit_code"`; TimedOut bool `json:"timed_out"`; StartFailed bool `json:"start_failed"` }
type input struct {
 Name string `json:"name"`; Kind string `json:"kind"`; Method string `json:"method"`; Body string `json:"body"`
 ContentLength *int64 `json:"content_length"`; Chunked bool `json:"chunked"`; BodyMode string `json:"body_mode"`; RegistryMissing bool `json:"registry_missing"`; Definitions []provider.Definition `json:"definitions"`
 Lookup map[string]string `json:"lookup"`; Modified map[string]string `json:"modified"`; Processes map[string]process `json:"processes"`
 Definition provider.Definition `json:"definition"`; IDs []string `json:"ids"`; Text string `json:"text"`
}
var current input
var oracleTime=time.Date(2026,10,4,2,3,4,567890000,time.FixedZone("synthetic",9*3600))
var calls []map[string]any
var callsMu sync.Mutex
func providerCommandLookPath(name string)(string,error){if path,ok:=current.Lookup[name];ok{return path,nil};return "",errors.New("synthetic not found")}
type metadata struct{ t time.Time }
func(m metadata)Name()string{return "synthetic"};func(m metadata)Size()int64{return 0};func(m metadata)Mode()fs.FileMode{return 0700};func(m metadata)ModTime()time.Time{return m.t};func(m metadata)IsDir()bool{return false};func(m metadata)Sys()any{return nil}
func oracleStat(path string)(fs.FileInfo,error){if raw,ok:=current.Modified[path];ok{t,e:=time.Parse(time.RFC3339Nano,raw);return metadata{t},e};return nil,errors.New("synthetic stat failure")}
func runCLIVersionCommand(_ context.Context,exe string,args []string,timeout time.Duration)(string,int,bool,error){
 callsMu.Lock();calls=append(calls,map[string]any{"executable":exe,"args":args,"timeout_ms":timeout.Milliseconds(),"output_cap":cliVersionOutputCap});callsMu.Unlock()
 p:=current.Processes[exe];if p.StartFailed{return "",0,false,errors.New("synthetic start failure")}
 w:=&cappedWriter{limit:cliVersionOutputCap};_,_=w.Write(p.Output)
 return w.String(),p.ExitCode,p.TimedOut,nil
}
type Server struct{cliVersions *cliVersionState;registry *provider.Registry}
func(s *Server)providerRegistrySnapshot()*provider.Registry{return s.registry}
func(s *Server)guard(w http.ResponseWriter,r *http.Request,methods ...string)bool{return requireMethodOneOf(w,r,methods...)}
func must(e error){if e!=nil{panic(e)}}
func main(){
 raw,e:=os.ReadFile(os.Args[1]);must(e);var cases []input;must(json.Unmarshal(raw,&cases));out:=[]any{}
 for _,c:=range cases{
 if c.BodyMode=="limit"{c.Body=strings.Repeat(" ",1048576)+"{}"};if c.BodyMode=="trailing"{c.Body="{}"+strings.Repeat(" ",1048577)}
 current=c;calls=[]map[string]any{};row:=map[string]any{"name":c.Name}
 switch c.Kind{
 case "one":row["result"]=checkOneCLIVersion(context.Background(),c.Definition.ID,provider.EffectiveDefinition{Definition:c.Definition})
 case "line":row["line"]=firstNonEmptyLine(c.Text)
 case "normalize":row["ids"]=normalizeCLIVersionProviderIDs(c.IDs)
 default:
  r:=provider.New(c.Definitions);if c.RegistryMissing{r=nil};s:=&Server{cliVersions:newCLIVersionState(),registry:r}
  req:=httptest.NewRequest(c.Method,"/api/cli-versions",strings.NewReader(c.Body));if c.ContentLength!=nil {req.ContentLength=*c.ContentLength};if c.Chunked {req.TransferEncoding=[]string{"chunked"};req.ContentLength=-1;req.Body=io.NopCloser(strings.NewReader(c.Body))};resp:=httptest.NewRecorder();s.handleCLIVersions(resp,req)
  var body any;must(json.Unmarshal(resp.Body.Bytes(),&body));row["status"]=resp.Code;row["body"]=body;row["cache_control"]=resp.Header().Get("Cache-Control")
  cached,ok:=s.cliVersions.lastResult();if ok{row["cached"]=cached}else{row["cached"]=nil}
 }
 sort.Slice(calls,func(i,j int)bool{return calls[i]["executable"].(string)<calls[j]["executable"].(string)})
 row["calls"]=calls;out=append(out,row)
 }
 must(json.NewEncoder(os.Stdout).Encode(out))
}
