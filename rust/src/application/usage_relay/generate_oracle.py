"""Pinned Go synthetic oracle: postUsage is replaced by capture, never HTTP."""
import json, pathlib, subprocess
root = pathlib.Path(__file__).resolve().parents[4]
here = pathlib.Path(__file__).resolve().parent
source = (root / 'internal/usagerelay/usagerelay.go').read_text(encoding='utf-8')
source = source.replace('package usagerelay', 'package main', 1)
source = source[:source.index('func postUsage(')] + '''
var captured *hubUsagePayload
func postUsage(hubURL, token string, payload hubUsagePayload, logger *slog.Logger) error {
 captured = &payload; return nil
}
type Case struct { Name string; Provider string; Input string; Rollout string }
type Result struct { Name string; Stdout string; Payload *hubUsagePayload }
func main() {
 _ = http.MethodPost // original import retained; no network operation
 var cases []Case
 if err := json.NewDecoder(os.Stdin).Decode(&cases); err != nil { panic(err) }
 results := []Result{}
 logger := slog.New(slog.NewTextHandler(io.Discard,nil))
 for _, c := range cases {
  captured=nil; var stdout bytes.Buffer
  if c.Provider=="claude" {
   runClaude("http://127.0.0.1:1","synthetic",7,strings.NewReader(c.Input),&stdout,logger)
  } else {
   dir,err:=os.MkdirTemp("","relay-synthetic-"); if err!=nil {panic(err)}
   path:=dir+"/rollout.jsonl"; if err=os.WriteFile(path,[]byte(c.Rollout),0600);err!=nil {panic(err)}
   var input map[string]interface{}; json.Unmarshal([]byte(c.Input),&input)
   if input==nil {input=map[string]interface{}{}}
   input["transcript_path"]=path; raw,_:=json.Marshal(input)
   runCodex("http://127.0.0.1:1","synthetic",7,bytes.NewReader(raw),logger)
   os.RemoveAll(dir)
  }
  if captured!=nil {
   captured.StartedAt=""
   if c.Provider=="claude" {captured.UsageObservedAt=""} else {captured.TranscriptPath="TRIAL_TRANSCRIPT"}
  }
  results=append(results,Result{c.Name,stdout.String(),captured})
 }
 json.NewEncoder(os.Stdout).Encode(results)
}
'''
(here/'oracle.go').write_text('//go:build ignore\n\n'+source,encoding='utf-8')
cases=[]
def claude(name,v): cases.append(dict(Name=name,Provider='claude',Input=v if isinstance(v,str) else json.dumps(v),Rollout=''))
def codex(name,rows): cases.append(dict(Name=name,Provider='codex',Input='{"model":"gpt-test"}',Rollout='\n'.join(x if isinstance(x,str) else json.dumps(x) for x in rows)))
claude('empty',{})
claude('null','null')
claude('malformed','{')
claude('full',dict(model={'display_name':'Model'},cost={'total_cost_usd':1.2345,'total_duration_ms':125.9,'total_api_duration_ms':90.9,'total_lines_added':3,'total_lines_removed':2},context_window={'total_input_tokens':1200,'total_output_tokens':2300,'context_window_size':200000,'used_percentage':25.5,'remaining_percentage':74.5,'current_usage':{'cache_creation_input_tokens':500,'cache_read_input_tokens':700}},effort={'level':'high'},thinking={'enabled':True},exceeds_200k_tokens=True,version='1.2',agent={'name':'agent'},workspace={'repo':{'host':'host','owner':'owner','name':'name'}},rate_limits={'five_hour':{'used_percentage':0,'resets_at':42},'seven_day':None}))
for v in [None,{},[],False,1,'bad',{'five_hour':None},{'five_hour':{}},{'seven_day':{'used_percentage':22.5,'resets_at':99}},{'five_hour':{'used_percentage':'wrong'}},{'FIVE_HOUR':{}},{'five_hour':{},'seven_day':{}}]: claude('limits-'+str(len(cases)),dict(rate_limits=v))
for raw in ['{"rate_limits":{"five_hour":{}},"rate_limits":null}','{"RATE_LIMITS":{"five_hour":{}}}','{"rate_limits":false,"RATE_LIMITS":{"five_hour":{}}}','{"cost":{"total_cost_usd":"bad"}}','{"context_window":{"total_input_tokens":1.5}}','{"context_window":{"total_input_tokens":-1234,"total_output_tokens":1000000}}','{"cost":{"total_duration_ms":1e30}}','{"unknown":1e999}','{"model":null,"cost":null,"thinking":null}','{"model":{"display_name":"a"},"MODEL":{"display_name":"b"}}']:
 claude('edge-'+str(len(cases)),raw)
def event(tokens=None,limits='absent',timestamp='2026-10-05T01:02:03.123Z'):
 p={'type':'token_count','info':{'total_token_usage':tokens or {'input_tokens':30,'output_tokens':4,'cached_input_tokens':2,'reasoning_output_tokens':1},'model_context_window':200000}}
 if limits!='absent':p['rate_limits']=limits
 return {'timestamp':timestamp,'payload':p}
codex('empty',[])
codex('full',[event(limits={'primary':{'used_percent':0,'window_minutes':300,'resets_at':42},'secondary':{'used_percent':30},'credits':{'has_credits':False,'unlimited':True,'balance':'12'},'plan_type':'plus'})])
for v in [None,{},[],False,1,'bad',{'primary':None},{'primary':{}},{'primary':{'used_percent':'wrong'}},{'PRIMARY':{}},{'credits':{}},{'credits':{'balance':1}}]:codex('rate-'+str(len(cases)),[event(limits=v)])
codex('zero-latest-clears-limits',[event(limits={'primary':{}}),event({'input_tokens':0,'output_tokens':0},timestamp='2026-10-05T02:00:00Z')])
codex('flat-keeps-limits',[event(limits={'primary':{}}),{'type':'token_count','input_tokens':50,'output_tokens':6}])
codex('priority',[event({'input_tokens':4,'inputTokens':8,'outputTokens':5,'totalTokens':12})])
codex('negative-primary-alias',[event({'input_tokens':-2,'inputTokens':9,'output_tokens':5})])
codex('last-usage',[{'payload':{'type':'token_count','info':{'total_token_usage':{},'last_token_usage':{'input_tokens':8,'output_tokens':3}}}}])
codex('oversize-then-valid',['x'*(256*1024+1),event()])
codex('false-positive',[{'body':'token_count','input_tokens':8,'output_tokens':2}])
codex('no-marker',[{'input_tokens':8,'output_tokens':2}])
codex('malformed-then-valid',['{',event()])
codex('invalid-observed',[event(timestamp='invalid')])
codex('zero-go-time',[event(timestamp='0001-01-01T00:00:00Z')])
codex('nested-duplicate-raw',['{"payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":4}},"rate_limits":{"primary":{}}},"PAYLOAD":{"rate_limits":null}}'])
(here/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2),encoding='utf-8')
exe=root.parent/'.toolchains/go1268/go/bin/go.exe'
result=subprocess.run([str(exe),'run',str(here/'oracle.go')],input=json.dumps(cases).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,check=True)
(here/'golden.json').write_bytes(result.stdout)
print('Generated',len(cases),'synthetic relay cases')
