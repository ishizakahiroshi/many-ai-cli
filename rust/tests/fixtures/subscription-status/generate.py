from pathlib import Path
import json
root=Path.cwd();base=root/'internal/subscription';out=root/'rust/tests/fixtures/subscription-status'
def fn(file,start):
 s=(base/file).read_text();a=s.index(start);return s[a:s.index('\n}\n',a)+3]
s=(base/'claude.go').read_text();a=s.index('type claudeAuthStatus struct');b=s.index('\n}\n',a)+3
pieces=[s[a:b],fn('claude.go','func (a claudeAdapter) Status('),fn('codex.go','func parseCodexLoginStatus('),fn('grok.go','func parseGrokModelsStatus('),fn('opencode.go','func parseOpenCodeProvidersList(')]
cases=[]
def add(p,o,code=0):cases.append({'provider':p,'output':o,'code':code})
for o in ['{}','null','{"loggedIn":false,"email":"synthetic@example.com"}','{"loggedIn":true,"subscriptionType":"max","authMethod":"oauth","email":"synthetic@example.com","orgId":"synthetic-only"}','{"loggedIn":true,"loggedIn":null,"subscriptionType":null}','{"LOGGEDIN":true,"AUTHMETHOD":"api-key"}','{"loggedIn":true,"loggedIn":"wrong"}','invalid','{} {}']:add('claude',o)
for o,c in [('Not logged in',1),('Logged in using ChatGPT',0),('Logged in using API key: synthetic-secret-value',0),('Logged in using api-key',1),('',0),('',1),('NOT LOGGED IN using ChatGPT',0)]:add('codex',o,c)
for o in ['You are not authenticated.','not logged in','Logged in with grok.com synthetic@example.com','authenticated with x.ai','logged in with synthetic-unlisted','unknown']:add('grok',o)
for o in ['0 credentials','1 credential','2 credentials Opencode Go','Credential listing','unknown','999999999999999999999999999 credentials','credentials then 0 credentials','1\u00a0credential']:add('opencode',o)
(out/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2))
program='''package main
import("context";"encoding/json";"fmt";"strings";"regexp";"strconv";"os")
type Status struct{LoggedIn bool `json:"logged_in"`;Plan string `json:"plan,omitempty"`;Method string `json:"method,omitempty"`}
type claudeAdapter struct{}
func(claudeAdapter)LaunchEnv(string)[]string{return nil}
var supplied string
func runVendorCLI(context.Context,string,[]string,[]string)(string,int,error){return supplied,0,nil}
var grokLoginMethods=[]string{"grok.com","x.ai"}
var openCodeCredentialCountRe=regexp.MustCompile(`(\\d+)\\s+credentials?`)
'''+ '\n'.join(pieces)+'''
type Case struct{Provider string `json:"provider"`;Output string `json:"output"`;Code int `json:"code"`}
func main(){var cases []Case;data,e:=os.ReadFile(os.Args[1]);if e!=nil{panic(e)};if e=json.Unmarshal(data,&cases);e!=nil{panic(e)};items:=[]any{};for _,c:=range cases{var status Status;var err error;switch c.Provider{case"claude":supplied=c.Output;status,err=(claudeAdapter{}).Status(context.Background(),"synthetic");case"codex":status=parseCodexLoginStatus(c.Output,c.Code);case"grok":status=parseGrokModelsStatus(c.Output);case"opencode":status=parseOpenCodeProvidersList(c.Output)};errorText:="";if err!=nil{errorText=err.Error()};items=append(items,map[string]any{"status":status,"error":errorText})};encoded,e:=json.MarshalIndent(items,"","  ");if e!=nil{panic(e)};if e=os.WriteFile(os.Args[2],encoded,0600);e!=nil{panic(e)}}
'''
(out/'oracle.go').write_text('//go:build ignore\n\n'+program)
print(len(cases),'synthetic status cases generated')
