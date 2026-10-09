"""Run fixed-Go Grok history parser only against synthetic temporary files."""
import json,pathlib,subprocess
here=pathlib.Path(__file__).resolve().parent;root=here.parents[3]
source=(root/'internal/hub/grok_history_handler.go').read_text(encoding='utf-8')
def decl(start):
    at=source.index(start);end=source.index('\n}',at)+2;return source[at:end]
oracle='''package main
import("bufio";"encoding/json";"os";"strings";"many-ai-cli/internal/sessionlog")
const grokHistoryLineMax=8*1024*1024
'''+ '\n'.join(decl(start) for start in ['type grokChatMessage struct','type grokChatLine struct','type grokContentPart struct','func readGrokChatHistory(','func grokContentText(','func extractGrokUserQuery('])+'''
type Case struct{Name,Body string}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};out:=[]map[string]any{}
for _,c:=range cases{f,err:=os.CreateTemp("","many-ai-grok-history-oracle-");if err!=nil{panic(err)};path:=f.Name();if _,err:=f.WriteString(c.Body);err!=nil{panic(err)};f.Close();messages,err:=readGrokChatHistory(path);os.Remove(path);out=append(out,map[string]any{"name":c.Name,"messages":messages,"error":err!=nil})}
if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}}
'''
(here/'oracle.go').write_text(oracle,encoding='utf-8')
cases=[]
def case(name,record=None,body=None):cases.append(dict(name=name,body=body if body is not None else json.dumps(record,ensure_ascii=False)+'\n'))
for name,record in [
('assistant',dict(type='assistant',content=' hello ')),('user-query',dict(type='user',content='<user_info>HIDDEN</user_info><user_query> question </user_query>ignored')),('user-missing-close',dict(type='user',content='<user_query> rest ')),('user-empty',dict(type='user',content='<user_query> </user_query>')),('environment-only',dict(type='user',content='<user_info>HIDDEN</user_info>')),('synthetic-user',dict(type='user',synthetic_reason='test',content='<user_query>HIDDEN</user_query>')),('reasoning',dict(type='reasoning',content='HIDDEN',encrypted_content='HIDDEN')),('tool-result',dict(type='tool_result',content='HIDDEN')),('system',dict(type='system',content='HIDDEN')),('parts',dict(type='assistant',content=[dict(type='text',text='one'),dict(type='image',text='HIDDEN'),dict(type='text',text='two')])),('parts-empty',dict(type='assistant',content=[dict(type='text',text=''),dict(type='text',text='one')])),('parts-null',dict(type='assistant',content=[None,dict(type='text',text='safe')])),('parts-wrongtype',dict(type='assistant',content=[dict(type='text',text='safe'),dict(type='image',text=9)])),('content-number',dict(type='assistant',content=9)),('content-null',dict(type='assistant',content=None)),('type-number',dict(type=9,content='HIDDEN')),('reason-number',dict(type='assistant',synthetic_reason=9,content='HIDDEN')),('reason-null',dict(type='assistant',synthetic_reason=None,content='safe')),('go-space',dict(type='assistant',content='\u0085\u3000safe\u2005')),('bom-not-space',dict(type='assistant',content='\ufeffsafe\ufeff')),('secret-mask',dict(type='assistant',content='ANTHROPIC_API_KEY=synthetic_secret')),('all-query-tail',dict(type='user',content='prefix<user_query>first</user_query>second<user_query>last</user_query>'))]:case(name,record)
case('duplicate-content',body='{"type":"assistant","content":"first","content":"last"}\n')
case('folded-fields',body='{"TYPE":"assistant","CONTENT":"safe"}\n')
case('wrong-known-then-valid',body='{"type":9,"type":"assistant","content":"safe"}\n')
case('ignored-huge',body='{"type":"assistant","unknown":1e999,"content":"safe"}\n')
case('empty-lines',body='\n\n{}\n')
case('last-no-newline',body='{"type":"assistant","content":"final"}')
case('crlf',body='{"type":"assistant","content":"safe"}\r\n')
case('trailing-value',body='{"type":"assistant","content":"HIDDEN"} {}\n')
case('malformed-between',body='{"type":"assistant","content":"first"}\ninvalid\n{"type":"assistant","content":"last"}\n')
case('scanner-cap',body='x'*(8*1024*1024)+'\n')
case('scanner-exact-newline',body='x'*(8*1024*1024-1)+'\n')
(here/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2),encoding='utf-8')
result=subprocess.run([str(root.parent/'.toolchains/go1268/go/bin/go.exe'),'run',str(here/'oracle.go')],cwd=root,input=json.dumps(cases).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
if result.returncode:raise RuntimeError(result.stderr.decode())
(here/'golden.json').write_bytes(result.stdout)
print('Generated',len(cases),'synthetic Grok history cases')
