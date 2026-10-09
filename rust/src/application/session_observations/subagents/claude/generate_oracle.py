"""Fixed-Go reader oracle, exclusively creates synthetic temporary transcripts."""
import pathlib,json,subprocess
here=pathlib.Path(__file__).resolve().parent;root=here.parents[5]
source=(root/'internal/hub/subagent_source_claude.go').read_text(encoding='utf-8').replace('package hub','package main',1)
source=source.replace('"encoding/json"','"encoding/json"\n"fmt"',1)
source=source.replace('now := time.Now()','now := oracleNow').replace('UpdatedAt: time.Now().UnixMilli()','UpdatedAt: oracleNow.UnixMilli()')
chat=(root/'internal/hub/agent_chat_parse.go').read_text(encoding='utf-8')
stall=(root/'internal/hub/transcript_stall.go').read_text(encoding='utf-8')
def decl(s,kind,name):
 start=s.index(kind+' '+name);end=s.index('\n}',start)+2;return s[start:end]
source+='\n'+ '\n'.join(decl(chat,k,n) for k,n in [('type','agentChatReadBudget'),('type','agentChatReadStats'),('type','agentChatTailRecord'),('type','claudeTranscriptLine'),('type','anthropicContentBlock'),('func','agentChatBudgetNow'),('func','agentChatBudgetExpired'),('func','readAgentChatTailPageWithBudget')])
source+='\n'+decl(stall,'func','subagentDirForTranscript')+'''
const agentChatPageRecordsMax=512
const agentChatReadBuffer=64*1024
const agentChatPageBytesMax=2*1024*1024
const agentChatReadTimeBudget=100*time.Millisecond
type subagentReadBudget struct {HeadBytes,TailBytes int64;MaxNodes int}
func defaultSubagentReadBudget() subagentReadBudget {return subagentReadBudget{64*1024,128*1024,50}}
var oracleNow time.Time
type Child struct {ID,Meta,JSONL string;Age int64}
type Stage struct {Parent *string;Now,Since int64;Max int}
type Case struct {Name string;Children []Child;Stages []Stage}
type Result struct{Name string;Trees []*proto.SubagentTree}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};results:=[]Result{}
for _,c:=range cases{
 dir,err:=os.MkdirTemp("","claude-reader-synthetic-");if err!=nil{panic(err)}
 parent:=filepath.Join(dir,"parent.jsonl");sub:=subagentDirForTranscript(parent);os.MkdirAll(sub,0700)
 base:=time.Unix(1791158400,0)
 for _,child:=range c.Children{meta:=filepath.Join(sub,"agent-"+child.ID+".meta.json");path:=filepath.Join(sub,"agent-"+child.ID+".jsonl");os.WriteFile(meta,[]byte(child.Meta),0600);os.WriteFile(path,[]byte(child.JSONL),0600);mtime:=base.Add(-time.Duration(child.Age)*time.Second);os.Chtimes(path,mtime,mtime)}
 result:=Result{Name:c.Name};var state any
 for _,stage:=range c.Stages{oracleNow=base.Add(time.Duration(stage.Now)*time.Second);if stage.Parent!=nil{os.WriteFile(parent,[]byte(*stage.Parent),0600);os.Chtimes(parent,oracleNow,oracleNow)}
 budget:=defaultSubagentReadBudget();budget.MaxNodes=stage.Max
 tree,next,err:=readClaudeSubagentTree(parent,base.Add(time.Duration(stage.Since)*time.Second),state,budget);if err!=nil{panic(err)};state=next;result.Trees=append(result.Trees,tree)}
 results=append(results,result);os.RemoveAll(dir)
};json.NewEncoder(os.Stdout).Encode(results)}
'''
(here/'oracle.go').write_text(source,encoding='utf-8')
def line(v):return json.dumps(v,ensure_ascii=False)+'\n'
def assistant(name='Bash',inp=None,**extra):
 return line(dict(type='assistant',timestamp='2026-10-05T00:00:00Z',message={'role':'assistant','content':[dict(type='tool_use',id='tool-child',name=name,input=inp or {'command':'git status'},**extra)]}))
def child(id='a',meta=None,body=None,age=60):return dict(ID=id,Meta=meta or json.dumps(dict(agentType='general',description='short label',toolUseId='tool-'+id,spawnDepth=1)),JSONL=body or assistant(),Age=age)
def launch(id='a'):return line(dict(type='assistant',timestamp='2026-10-05T00:00:01Z',message={'content':[dict(type='tool_use',id='tool-'+id)]}))
def result(status,id='a',async_=False):return line(dict(type='user',timestamp='2026-10-05T00:00:10Z',toolUseResult=dict(status=status,isAsync=async_),message={'content':[dict(type='tool_result',tool_use_id='tool-'+id)]}))
def notification(status,id='a'):return line(dict(type='queue-operation',operation='enqueue',timestamp='2026-10-05T00:00:12Z',content='<task-notification><tool-use-id>tool-'+id+'</tool-use-id><status>'+status+'</status><result>PRIVATE_BODY</result></task-notification>'))
cases=[]
def case(name,children,parent='',since=-100,max_=50,stages=None):cases.append(dict(Name=name,Children=children,Stages=stages or [dict(Parent=parent,Now=0,Since=since,Max=max_)]))
case('empty',[])
for status in ['completed','FAILED','failure','killed','stopped','cancelled','canceled','aborted','interrupted','error','timeout','timed_out','async_launched','unknown',' completed ']:
 case('status-'+status,[child()],launch()+result(status))
for status in ['completed','failed','killed','unknown']:case('notice-'+status,[child()],launch()+notification(status))
case('background-ack',[child(age=1000)],result('completed',async_=True))
case('sticky-running',[child(age=1000)],stages=[dict(Parent=launch(),Now=0,Since=-100,Max=50),dict(Parent='',Now=2000,Since=-100,Max=50)])
case('sticky-resolved',[child()],stages=[dict(Parent=result('failed'),Now=0,Since=-100,Max=50),dict(Parent=launch(),Now=2000,Since=-100,Max=50)])
case('latest-completion',[child()],result('completed')+result('failed'))
case('stale-unknown-pruned',[child(age=1000)],since=1)
case('stale-running-kept',[child(age=1000)],launch(),since=1)
case('bad-meta',[child(meta='{"description":12,"toolUseId":"tool-a"}')])
case('meta-null',[child(meta='null')])
case('meta-duplicate-invalid',[child(meta='{"description":12,"description":"label","toolUseId":"tool-a"}')])
case('meta-uppercase',[child(meta='{"AGENTTYPE":"general","DESCRIPTION":"label","TOOLUSEID":"tool-a","SPAWNDEPTH":null}')])
case('nested-duplicate-message',[child(body='{"type":"assistant","timestamp":"2026-10-05T00:00:00Z","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"one"}}]},"MESSAGE":{"role":"assistant"}}\n')])
case('array-null',[child(body=line(dict(type='assistant',timestamp='2026-10-05T00:00:00Z',message={'content':[None,dict(type='tool_use',name='Read',input={'file_path':'file'})]})))])
for inp in [{'prompt':'PRIVATE_BODY'},{'pattern':'a\n b'},{'path':'path','file_path':'fallback'},{'command':'','pattern':'next'},{'command':'x'*120},{'command':12},{'command':'x','pattern':12},{'COMMAND':'caps'},None]:case('summary-'+str(len(cases)),[child(body=assistant(inp=inp))])
case('workflow-filter',[child(meta='{"agentType":"workflow-subagent","description":"ignored","toolUseId":"tool-a"}')])
case('orphans',[child('a'),child('b',meta='{"description":"child","toolUseId":"tool-b","spawnDepth":2,"parentAgentId":"missing"}'),child('c',meta='{"description":"grand","toolUseId":"tool-c","spawnDepth":3,"parentAgentId":"b"}')])
case('cap-cascade',[child('a'),child('b',meta='{"description":"child","toolUseId":"tool-b","spawnDepth":2,"parentAgentId":"a"}'),child('c')],result('completed','a')+result('failed','b')+launch('c'),max_=2)
case('cap-ties',[child('a'),child('b'),child('c')],result('completed','a')+result('completed','b'),max_=2)
case('no-newline-tail',[child(body=assistant().rstrip('\n'))])
case('bad-transcript-knownfield',[child(body=line(dict(type='assistant',timestamp='2026-10-05T00:00:00Z',isSidechain='bad',message={'content':[dict(type='tool_use',name='Bad')]})))])
case('ignored-huge-raw',[child(body='{"type":"assistant","timestamp":"2026-10-05T00:00:00Z","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"safe","unknown":1e999}}]}}\n')])
case('unknown-block-typed-error',[child(body=assistant(text=12))])
case('zero-cap',[child()],max_=0)
old_child=child(age=1200,body=assistant().replace('2026-10-05T00:00:00Z','2026-10-04T23:40:00Z'))
bounded_parent=launch()+'{}\n'*500+launch().rstrip('\n')
case('parent-500-incomplete-final-old-child',[old_child],bounded_parent,since=-1)
case('parent-500-complete-final-old-child',[old_child],bounded_parent+'\n',since=0)
(here/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2),encoding='utf-8')
result_=subprocess.run([str(root.parent/'.toolchains/go1268/go/bin/go.exe'),'run',str(here/'oracle.go')],cwd=root,input=json.dumps(cases).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
if result_.returncode:raise RuntimeError(result_.stderr.decode())
(here/'golden.json').write_bytes(result_.stdout)
print('Generated',len(cases),'synthetic Claude reader cases')
