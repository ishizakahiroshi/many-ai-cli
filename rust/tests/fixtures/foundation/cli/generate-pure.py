#!/usr/bin/env python3
"""Execute only extracted side-effect-free Go source functions with synthetic input.

OS env reads are replaced with a test map; no real environment is inspected.
The extracted source and its source hash are included in the resulting fixture.
"""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[4]
SOURCES = {
    "internal/orchestrate/orchestrate.go": ["hubEnv", "spawnClientTimeout", "parseRelayProviderModel"],
    "internal/wrapper/delegation.go": ["DelegationPromptEnabled"],
    "cmd/many-ai-cli/issue.go": ["confirmIssue", "validIssueProvider"],
    "internal/shell/init.go": ["InitScriptForProviders"],
    "cmd/many-ai-cli/provider_command.go": ["runProviderBackupCommand", "runProviderResetCommand", "runProviderRecoverCommand"],
    "internal/provider/store.go": ["IsBuiltinID"],
    "internal/provider/schema.go": [],
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--go", default="go")
    args = parser.parse_args()
    functions = []
    manifest = []
    declarations = []
    for source, names in SOURCES.items():
        raw = (ROOT / source).read_text()
        manifest.append({"path": source, "sha256": hashlib.sha256(raw.encode()).hexdigest(), "functions": names})
        for name in names:
            match = re.search(r"^func " + name + r"\(.*?^}", raw, re.M | re.S)
            if not match:
                raise ValueError(f"missing source function: {source}:{name}")
            body = match[0]
            functions.append({"source": source, "line": raw[:match.start()].count("\n") + 1, "function": name,
                              "body": body, "transformation": "os.Getenv reads replaced by a synthetic map; provider history calls replaced by an error-only boundary spy"})
            declarations.append(body.replace("os.Getenv(", "testGetenv(").replace("*provider.HistoryStore", "*boundaryHistory").replace("provider.IsBuiltinID", "IsBuiltinID"))
        if source.endswith("orchestrate.go"):
            declarations += re.findall(r"^const \(\n.*?^\)", raw, re.M | re.S)[:1]
            declarations += re.findall(r"^const defaultSpawnClientTimeout = .*", raw, re.M)
        if source.endswith("delegation.go"):
            declarations += re.findall(r"^const DelegationEnvName = .*", raw, re.M)
        if source.endswith("schema.go"):
            declarations += re.findall(r"^var BuiltinProviderIDs = \[\]string\{.*?^}", raw, re.M | re.S)
        if source.endswith("init.go"):
            declarations += re.findall(r"^var shellFunctionIDPattern = .*", raw, re.M)
    program = '''package main
import ("bufio"; "bytes"; "encoding/json"; "errors"; "fmt"; "io"; "os"; "regexp"; "strconv"; "strings"; "time")
// The spy ALWAYS errors at the first storage call. Successful runtime behavior
// is deliberately impossible: no generated fixture claims persistence happened.
type boundaryHistory struct { Call string; Args []string }
type boundaryRecord struct { Revision, CreatedAt, Reason, ContentDigest string }
var boundaryError = errors.New("oracle: storage boundary reached (not executed)")
func (h *boundaryHistory) hit(name string,args ...string) error {h.Call=name;h.Args=args;return boundaryError}
func (h *boundaryHistory) ListBackups(id string)([]boundaryRecord,error){return nil,h.hit("ListBackups",id)}
func (h *boundaryHistory) VerifyBackup(id,rev string)(boundaryRecord,error){return boundaryRecord{},h.hit("VerifyBackup",id,rev)}
func (h *boundaryHistory) RestoreBackup(id,rev,expected string)(boundaryRecord,error){return boundaryRecord{},h.hit("RestoreBackup",id,rev,expected)}
func (h *boundaryHistory) Reset(id,expected string)(boundaryRecord,error){return boundaryRecord{},h.hit("Reset",id,expected)}
func (h *boundaryHistory) LastVerifiedRevision(id string)(boundaryRecord,bool,error){return boundaryRecord{},false,h.hit("LastVerifiedRevision",id)}
func (h *boundaryHistory) RecoverHead(id,revision string)(boundaryRecord,error){return boundaryRecord{},h.hit("RecoverHead",id,revision)}
var environment map[string]string
func testGetenv(name string) string { return environment[name] }
''' + "\n\n".join(declarations) + r'''
func main() {
 out := map[string]any{}
 hubs := []any{}
 envs := []map[string]string{
  {}, {sessionIDEnv:"0"}, {sessionIDEnv:"-1"}, {sessionIDEnv:" 1"}, {sessionIDEnv:"1.5"},
  {sessionIDEnv:"1"}, {sessionIDEnv:"1",hubPortEnv:"49123"},
  {sessionIDEnv:"001",hubPortEnv:"49123",hubTokenEnv:"synthetic"},
  {sessionIDEnv:"+1",hubPortEnv:"not-a-number",hubTokenEnv:"synthetic"},
  {sessionIDEnv:"1",hubPortEnv:" ",hubTokenEnv:" "},
 }
 for _, env := range envs { environment=env; url,tok,id,err:=hubEnv("spawn");message:="";if err!=nil{message=err.Error()};hubs=append(hubs,map[string]any{"env":env,"url":url,"token":tok,"session_id":id,"error":message}) }
 out["hub_env"] = hubs
 providerArgs:=map[string][][]string{
 "backup":{{},{"list"},{"list","claude"},{"list","claude","ignored"},{"unknown","claude"},{"verify","claude"},{"verify","claude","r1","ignored"},{"restore","claude"},{"restore","claude","r1"},{"restore","claude","r1","--expected-revision"},{"restore","claude","r1","--expected-revision",""},{"restore","claude","r1","--expected-revision=r0"},{"restore","claude","r1","ignored","--expected-revision","r0"},{"restore","claude","r1","--expected-revision","first","--expected-revision","last"}},
 "reset":{{},{"claude"},{"--distributed"},{"--distributed","claude"},{"--distributed","claude","--expected-revision",""},{"--distributed","claude","--expected-revision","r0"},{"--distributed","custom-provider","--expected-revision","r0"},{"--distributed","claude","--expected-revision=r0"},{"--distributed","claude","--expected-revision","first","--expected-revision","last"}},
 "recover":{{},{"claude"},{"claude","--list"},{"claude","--list","ignored"},{"claude","r1"},{"claude","--unknown"},{"claude","r1","--list"}},
 }
 for _,action:=range []string{"backup","reset","recover"}{rows:=[]any{};for _,args:=range providerArgs[action]{h:=&boundaryHistory{};var err error;var output bytes.Buffer;switch action{case "backup":err=runProviderBackupCommand(h,args);case "reset":err=runProviderResetCommand(h,args);case "recover":err=runProviderRecoverCommand(h,args,&output)};message:="";if err!=nil{message=err.Error()};if err==nil{panic("provider oracle escaped its error-only boundary")};rows=append(rows,map[string]any{"args":args,"error":message,"boundary_call":h.Call,"boundary_args":h.Args,"stdout":output.String()})};out["provider_"+action]=rows}
 durations:=[]any{}
 for _,raw:=range []string{"","bad","0s","-1s","30s"," 1m30s ","100ms","9223372036854775808ns"}{environment=map[string]string{spawnTimeoutEnv:raw};durations=append(durations,map[string]any{"raw":raw,"nanoseconds":int64(spawnClientTimeout())})};out["spawn_timeout"]=durations
 delegation:=[]any{}
 for _,raw:=range []string{"","1","0","true","false"," 1"}{for _,def:=range []bool{false,true}{environment=map[string]string{DelegationEnvName:raw};delegation=append(delegation,map[string]any{"raw":raw,"config_default":def,"enabled":DelegationPromptEnabled(def)})}};out["delegation"]=delegation
 relays:=[]any{}
 for _,raw:=range []string{"claude"," codex/gpt-synthetic@high ","opencode/vendor/model@variant","claude/","claude@","/model","","claude/has space","claude/a\tb","codex/model@x@y","claude @ high","claude\topus"}{p,m,e,err:=parseRelayProviderModel(raw);message:="";if err!=nil{message=err.Error()};relays=append(relays,map[string]any{"raw":raw,"provider":p,"model":m,"effort":e,"error":message})};out["relay_provider_model"]=relays
 confirmations:=[]any{}
 for _,input:=range []string{"","\n","y\n","Y\n"," yes\n"," y \n","no\n","y","y\nnext\n"}{var output bytes.Buffer;ok,err:=confirmIssue(bufio.NewReader(strings.NewReader(input)),&output);message:="";if err!=nil{message=err.Error()};confirmations=append(confirmations,map[string]any{"stdin":input,"confirmed":ok,"stderr":output.String(),"error":message})};out["issue_confirmation"]=confirmations
 providers:=[]any{}
 for _,p:=range []string{"","claude","codex","copilot","cursor-agent","opencode","grok","command-code","gemini","shell","Codex"," codex ","custom-provider"}{providers=append(providers,map[string]any{"provider":p,"valid":validIssueProvider(p)})};out["issue_provider"]=providers
 shells:=[]any{}
 for _,ids:=range [][]string{{},{"claude","codex"},{"safe-id","safe_id","UPPER","bad id","bad;id","9bad","a/b","a.b","$bad","日本語"}}{shells=append(shells,map[string]any{"ids":ids,"stdout":InitScriptForProviders(ids)})};out["shell_init"]=shells
 enc:=json.NewEncoder(os.Stdout);enc.SetIndent("","  ");if err:=enc.Encode(out);err!=nil{panic(err)}
}
'''
    # Temporary source lives only inside the owned fixture directory and is removed.
    with tempfile.TemporaryDirectory(prefix=".pure-oracle-", dir=HERE) as tmp:
        go_file = pathlib.Path(tmp) / "main.go"
        go_file.write_text(program)
        result = subprocess.run([args.go, "run", str(go_file)], check=True, capture_output=True, cwd=ROOT)
    data = {"method": "Extracted baseline pure functions executed by Go; only os.Getenv changed to a synthetic map.",
            "sources": manifest, "extracted_functions": functions, "cases": json.loads(result.stdout)}
    (HERE / "pure-oracle.json").write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")
    print(sum(len(c) for c in data["cases"].values()), "pure source-function cases")


if __name__ == "__main__":
    main()
