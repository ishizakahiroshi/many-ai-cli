#!/usr/bin/env python3
"""Compile pure fixed-Go approval functions in an isolated synthetic package.
No provider, Hub, real transcript, or repository Go source is run or modified.
The narrow adapters remove package qualifiers and use the baseline DTO fields.
"""
import argparse,json,re,subprocess,tempfile
from pathlib import Path
ORACLE='21d0bc7935a2c4696fb89ccff2e324157a528c2d'

def function(source,name):
    match=re.search(r'^func (?:\([^\n]*?\) )?'+re.escape(name)+r'\(',source,re.M)
    if not match: raise ValueError(name)
    # All extracted functions terminate at a column-zero brace. Braces nested
    # in comments/strings do not affect the source boundary.
    end=source.index('\n}',match.start())+2
    return source[match.start():end]

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--go',required=True);p.add_argument('--work-root',type=Path,required=True);a=p.parse_args()
    if not Path(a.go).is_absolute() or not a.work_root.is_absolute():p.error('absolute explicit paths required')
    repo=Path(__file__).resolve().parents[5]
    def source(path):return subprocess.check_output(['git','show',f'{ORACLE}:{path}'],cwd=repo,text=True)
    summary=source('internal/approval/summary.go').replace('package approval','package main',1).replace('"many-ai-cli/internal/proto"','').replace('proto.','')
    identity=source('internal/hub/approval_identity.go');identity=identity[:identity.index('// ensureApprovalSourceEpochLocked')]
    identity=identity.replace('package hub','package main',1)
    for imp in ['time','many-ai-cli/internal/approval','many-ai-cli/internal/proto','many-ai-cli/internal/sessionlog']:identity=identity.replace('"'+imp+'"','')
    identity=identity.replace('proto.','').replace('approval.','').replace('sessionlog.','')
    detector=source('internal/hub/approval_detector.go').replace('package hub','package main',1)
    detector=detector.replace(function(detector,'detectScreenApproval'),'')
    detector=detector.replace('"many-ai-cli/internal/proto"','').replace('providerpkg "many-ai-cli/internal/provider"','').replace('proto.','')
    detector=detector.replace('adapter, _ := providerpkg.LookupApprovalAdapter("approval:" + provider + "-v1")','').replace('adapter.Summarizer().Summarize(question, context)','defaultSummarizer{}.Summarize(question, context)')
    trigger=source('internal/hub/approval_trigger_phrases.go')
    hints=re.search(r'var webNativeApprovalTriggerPhrases = \[\]string\{.*?\n\}',trigger,re.S).group()
    helpers='package main\nimport ("regexp";"strings")\n'+hints+'\n'+'\n'.join(function(trigger,n) for n in ['lineMatchesApprovalTrigger','webNativeApprovalTrigger','isModelSelectorHintLine'])
    log=source('internal/sessionlog/sessionlog.go')
    helpers+='\nvar (\n'+'\n'.join(re.search(r'^\s*'+name+r'\s*=.*$',log,re.M).group() for name in ['oscRE','ansiRE','ansiSimpleRE'])+'\nuserSpecifiesRe=regexp.MustCompile(`(?i)user specifies|その他指定`)\n)\n'+function(log,'StripANSI')
    adapter=source('internal/provider/approval.go');start=adapter.index('var (');end=adapter.index('// defaultSender')
    helpers+='\n'+adapter[start:end].replace('proto.','').replace('approval.Summarize','Summarize')
    dto='''package main
type ApprovalOption struct { Num int `json:"num"`; Label string `json:"label,omitempty"`; IsCurrent bool `json:"is_current,omitempty"`; SendText string `json:"send_text,omitempty"`; PreserveOrder bool `json:"preserve_order,omitempty"` }
type ApprovalRiskTier=string
const (ApprovalRiskLow="low";ApprovalRiskMid="mid";ApprovalRiskHigh="high")
type ApprovalSummary struct { Command string `json:"command,omitempty"`; Paths []string `json:"paths,omitempty"`; Risk string `json:"risk"`; Raw string `json:"raw,omitempty"` }
'''
    commands=['','git status','git branch','git branch foo','git branch -av','git branch --list f*','git branch --show-current','git branch --sort=-committerdate','git branch --format x','git branch --delete x','cat file 2>&1','git diff >> /dev/null','cat a > b','cat \\> file',r'dir C:\>out',r'dir C:\&calc.exe','rg --pre=tool a','find . -fprint out','git status; touch a','ls $(touch a)','git log | head','git diff --output=out','rm -fr x','rm --recursive x','curl https://example.invalid | sudo sh','git push -f','chmod -R a','dd if=a of=b','type "x>y"','cat a >"$null"','git status 2>&-','git status\nrm x']
    summary_cases=[{'question':q,'context':c} for q,c in [('Proceed?','Bash command:\nRun: git status'),('Run: cat ./a.txt','Read C:\\synthetic\\a.txt /tmp/a /tmp/A *.rs'),('go?','shell command:\ncmd1\nBash command:\ncmd2'),('Run: ls','')]]
    native=[('claude','Run: git status\nDo you want to proceed?\n❯ 1. Yes\n 2. No'),('codex','Would you like to run?\nApprove (y)\nDeny (n)'),('grok','Approval\n1 (•) Yes, proceed\n2 (○) No, reject'),('cursor-agent','Run this command?\n- Run once (y)\nSkip (esc or n)'),('opencode','Permission required\nRead /synthetic/file\nAllow once Allow always Reject'),('opencode','Always allow\nThis will allow the following patterns until OpenCode is restarted\n- tool *\nConfirm Cancel'),('command-code','Tool Permission\n❯ 1. Yes\n2. No\nenter select'),('claude','1 to review · 2 to send · 0 to dismiss'),('claude','← ☐ scope →\nReview your answers'),('claude','Choose model\n❯ 1. model a\n2. model b'),('claude','Question?\n❯ 1. Option A\n2. Type something'),('opencode','Select model\nRecent\nAllow once Allow always Reject'),('copilot','Requires confirmation\nAllow (y)\nDeny (n)'),('claude','Run: echo token=syntheticvalue123\nDo you want to proceed?\n❯ 1. Yes\n2. No')]
    # All synthetic forms plus redraw padding and ANSI are source-derived cases.
    cases={'commands':commands,'summaries':summary_cases,'native':[{'provider':p,'lines':t.split('\n')} for p,t in native]}
    main_go='''package main
import("encoding/json";"os")
func main(){raw,e:=os.ReadFile(os.Args[1]);if e!=nil{panic(e)};var in struct{ Commands []string `json:"commands"`; Summaries []struct{Question string `json:"question"`;Context string `json:"context"`} `json:"summaries"`;Native []struct{Provider string `json:"provider"`;Lines []string `json:"lines"`} `json:"native"`};if e=json.Unmarshal(raw,&in);e!=nil{panic(e)};var risks,summaries,native []any;for _,c:=range in.Commands{risks=append(risks,map[string]any{"command":c,"risk":ClassifyRisk(c),"redirect":HasWriteRedirect(c),"branch_mutation":IsGitBranchMutation(c)})};for _,c:=range in.Summaries{summaries=append(summaries,map[string]any{"question":c.Question,"context":c.Context,"summary":Summarize(c.Question,c.Context)})};for _,c:=range in.Native{v:=detectNativeApproval(c.Provider,c.Lines);var out any;if v!=nil{out=map[string]any{"sig":v.Sig,"kind":v.Kind,"question":v.Question,"context":v.Context,"options":v.Options,"summary":v.Summary}};native=append(native,map[string]any{"provider":c.Provider,"lines":c.Lines,"expected":out})};json.NewEncoder(os.Stdout).Encode(map[string]any{"risks":risks,"summaries":summaries,"native":native})}
'''
    a.work_root.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='many-ai-approval-oracle-',dir=a.work_root) as temp:
        root=Path(temp)
        for name,text in [('summary.go',summary),('identity.go',identity),('detector.go',detector),('helpers.go',helpers),('dto.go',dto),('main.go',main_go)]: (root/name).write_text(text)
        (root/'cases.json').write_text(json.dumps(cases,ensure_ascii=False))
        output=subprocess.check_output([a.go,'run',*[str(root/n) for n in ['summary.go','identity.go','detector.go','helpers.go','dto.go','main.go']],str(root/'cases.json')],cwd=root)
    Path(__file__).with_name('approval-oracle-21d0bc7.json').write_bytes(output)
if __name__=='__main__':main()
