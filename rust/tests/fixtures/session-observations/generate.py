"""Copy fixed Go pure parsers verbatim into an independent synthetic oracle."""
from pathlib import Path
import json
root=Path(__file__).resolve().parents[4]
out=Path(__file__).resolve().parent
workflow=(root/'internal/hub/workflow_scan.go').read_text(encoding='utf-8')
cross=(root/'internal/hub/cross_session_message.go').read_text(encoding='utf-8')
journal=(root/'internal/hub/workflow_journal.go').read_text(encoding='utf-8')
source=workflow[workflow.index('var ('):workflow.index('// workflowCounts is deliberately')]
source+=workflow[workflow.index('func stripWorkflowANSI'):workflow.index('func workflowCountsFromProgress')]
source+=cross[cross.index('var crossSessionMessageHeaderRE'):cross.index('func copyCrossSessionMessages')]
source+='\nconst workflowJournalFieldMax = 256\n'
source+=journal[journal.index('type workflowJournalEvent'):journal.index('type workflowJournalFileState')]
source+=journal[journal.index('type workflowJournalParserMode'):journal.index('// tailWorkflowJournal streams')]
main='''package main
import("encoding/json";"os";"regexp";"strconv";"strings";"unicode/utf8";"many-ai-cli/internal/proto")
'''+source+'''
func main(){ var cases []struct{ Name string `json:"name"`; Lines []string `json:"lines"`; Journal string `json:"journal"` };raw,err:=os.ReadFile(os.Args[1]);if err!=nil{panic(err)};if err=json.Unmarshal(raw,&cases);err!=nil{panic(err)};out:=[]map[string]any{};for _,row:=range cases{parser:=workflowJournalRecordParser{};for _,b:=range []byte(row.Journal){parser.feed(b)};event,ok:=parser.event();var journal any;if ok{journal=event};out=append(out,map[string]any{"name":row.Name,"workflow":parseWorkflowVT(row.Lines),"cross":detectCrossSessionMessage(row.Lines),"journal":journal})};if err=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(err)}}
'''
(out/'oracle.go').write_text('//go:build ignore\n\n'+main,encoding='utf-8')
cases=[]
for path in sorted((root/'internal/hub/testdata/workflow_vt').glob('*.txt')):
    cases.append({'name':path.stem,'lines':path.read_text(encoding='utf-8').splitlines()})
extra=[[],['ordinary output','✓ completed a normal edit'],['Waiting for 2 dynamic workflows to finish'],['Workflow','●x'],['Workflow','● x'],['Workflow','✓ done','❯ user input','● footer'],['Workflow','10000/10000 agents'],['Workflow','9/2 agents'],['Workflow','0/0 agents'],['Workflow','1/2 agents · 1h 2m 3s · ↓ 4k'],['Workflow','✓ '+('あ'*67)],['Workflow','✓ '+('あ'*66)],['⏺ Message from researcher'],['Received message from reviewer','Message from conductor'],['message received from a tool'],['Message from'],['received message to researcher'],['the assistant said Message from researcher'],['• MESSAGE FROM agent\r'],['\u00a0Message from agent\u00a0'],['⚙ Workflow running','● one  1s','✗ two','○ three','50%']]
cases += [{'name':f'boundary-{i}','lines':lines} for i,lines in enumerate(extra)]
records=['{"type":"started","agentId":"a"}', '{"agentId":"a","type":"result","result":"'+('synthetic-body'*1000)+'"}', '{"type":"started","agentId":"old","agentId":"new"}', '{"type":"started","nested":{"agentId":"wrong"},"agentId":"right"}', '{"type":null,"agentId":"a"}', '{"type":"started","agentId":42}', '{"type":"started","agentId":"a"}trailing', '{"type":"started","agentId":"'+('a'*260)+'"}', '{"type":"started","agentId":"\\u3042"}', '{"type":"started","result":{"a":[{"x":"}\\\""}]},"agentId":"a"}', '{"type":"started","agentId":"a",}', '{"type":"started","agentId":"a"']
cases += [{'name':f'journal-{i}','lines':[],'journal':record} for i,record in enumerate(records)]
(out/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(len(cases),'synthetic cases generated')
