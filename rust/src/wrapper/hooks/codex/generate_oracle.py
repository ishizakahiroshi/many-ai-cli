"""Synthetic pure Codex hook oracle, extracted from fixed Go source."""
import pathlib,json,subprocess
here=pathlib.Path(__file__).resolve().parent
root=here.parents[4]
source=(root/'internal/wrapper/usage_hooks.go').read_text(encoding='utf-8')
def function(name):
 start=source.index('func '+name+'(');end=source.index('\n}',start)+2
 return source[start:end]
constants=source[source.index('const ('):source.index('\n)',source.index('const ('))+2]
kind=source[source.index('type UsageHookParams struct'):source.index('\n}',source.index('type UsageHookParams struct'))+2]
inject=source[source.index('\t\t// 旧名 any-ai-cli マーカー',source.index('func InjectCodexStopHook')):source.index('\n\t})',source.index('func InjectCodexStopHook'))]
inject=inject.replace('return writeCodexConfig(path, content)','return content')
remove=source[source.index('\t\tcontent := string(data)',source.index('func RemoveCodexStopHook')):source.index('\n\t})',source.index('func RemoveCodexStopHook'))]
remove=remove.replace('\t\tcontent := string(data)','').replace('return nil','return content').replace('return writeCodexConfig(path, newContent)','return newContent')
oracle='''package main
import("encoding/json";"fmt";"os";"regexp";"strings")
const hubTokenEnvName="MANY_AI_CLI_HUB_TOKEN"
'''+constants+'\n'+kind+'\n'+'\n'.join(function(x) for x in ['toShellPath','usageHookQuotePOSIX','codexStopHookBlock'])+'''
func inject(content string,p UsageHookParams) string {
'''+inject+'''
}
func remove(content string) string {
'''+remove+'''
}
type Case struct{Name,Content,Exe string}
type Result struct{Name,Block,Injected,Removed string}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};results:=[]Result{}
for _,c:=range cases{p:=UsageHookParams{HubURL:"http://127.0.0.1:49686",Token:"synthetic_test_token",SessionID:7,ExePath:c.Exe};results=append(results,Result{c.Name,codexStopHookBlock(p),inject(c.Content,p),remove(c.Content)})};json.NewEncoder(os.Stdout).Encode(results)}
'''
(here/'oracle.go').write_text(oracle,encoding='utf-8')
start='# many-ai-cli:usage-hook-start';end='# many-ai-cli:usage-hook-end'
contents=['','model="example"','model="example"\n','before\n'+start+'\nold\n'+end+'\nafter',start+'\nmalformed',end,'# any-ai-cli:usage-hook-start\nlegacy\n# any-ai-cli:usage-hook-end\n',start+'\r\nold\r\n'+end+'\r\n',start+'\nx\n'+end+'\n'+start+'\ny\n'+end+'\n']
paths=[r'C:\trial\$portable\ai\many-ai-cli.exe',"C:/trial/${portable}/ai's app/many-ai-cli.exe",'C:/trial/\ue000\u200d/ai.exe']
cases=[dict(Name=f'content-{i}-path-{j}',Content=c,Exe=p) for i,c in enumerate(contents) for j,p in enumerate(paths)]
(here/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2),encoding='utf-8')
result=subprocess.run([str(root.parent/'.toolchains/go1268/go/bin/go.exe'),'run',str(here/'oracle.go')],input=json.dumps(cases).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=True)
(here/'golden.json').write_bytes(result.stdout)
print('Generated',len(cases),'pure hook cases')
