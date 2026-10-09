#!/usr/bin/env python3
"""Execute unchanged fixed-Go one-shot selection functions on synthetic options."""
import argparse, json, re, subprocess, tempfile
from pathlib import Path
ORACLE = '21d0bc7935a2c4696fb89ccff2e324157a528c2d'
def function(source, name):
    match = re.search(r'^func ' + re.escape(name) + r'\(', source, re.M)
    if not match: raise ValueError(name)
    return source[match.start():source.index('\n}', match.start()) + 2]
def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--go', required=True); parser.add_argument('--work-root', type=Path, required=True)
    args=parser.parse_args()
    if not Path(args.go).is_absolute() or not args.work_root.is_absolute(): parser.error('absolute paths required')
    repo=Path(__file__).resolve().parents[5]
    def source(path): return subprocess.check_output(['git','show',f'{ORACLE}:{path}'],cwd=repo,text=True)
    auto=source('internal/hub/auto_approval.go'); action=source('internal/hub/approval_action.go')
    code='package main\nimport("strings";"fmt";"encoding/json";"os")\n'
    code+='type ApprovalOption struct { Num int `json:"num"`; Label string `json:"label"`; IsCurrent bool `json:"is_current"`; SendText string `json:"send_text"` }\n'
    code+='\n'.join(function(auto,n).replace('proto.','') for n in ['normalizeApprovalOptionLabel','isExplicitApprovalNegativeLabel','isExplicitApprovalPositiveLabel','approvalOptionInput','autoApprovalInput'])
    code+='\n'+function(action,'oneTapRejectInput').replace('proto.','')+'\n'
    labels=['','Yes',' Yes (y) ','Yes (y) (recommended)','Allow once','Run (once)','Run once (y)','Yes, allow once','Yes, proceed','Yes proceed','許可','実行','続行','Deny once','Cancel this session','Skip (esc)','Do not allow','Don’t run','No, stop','Yes, always','Allow this session','Yes, don’t ask','Yes, all similar','Confirm','Send','Review','YES\u2003(recommended)']
    cases=[{'options':[{'num':1,'label':'Yes','is_current':focus=='yes'},{'num':2,'label':'No','is_current':focus=='no'}]} for focus in ['yes','no','none']]
    cases += [
        {'options':[{'num':1,'label':'Approve (y)','send_text':'y'},{'num':3,'label':'Deny (n)','send_text':'n','is_current':True}]},
        {'options':[{'num':1,'label':'Yes, always','is_current':True},{'num':2,'label':'Allow once'},{'num':3,'label':'Cancel this session'}]},
        {'options':[]},
        {'options':[{'num':0,'label':'Yes','is_current':True},{'num':0,'label':'Skip (esc)','send_text':'\u001b'}]},
        {'options':[{'num':-1,'label':'Yes'},{'num':0,'label':'Deny'}]},
    ]
    code+='''func main(){var in struct{Labels []string `json:"labels"`; Cases []struct{Options []ApprovalOption `json:"options"`} `json:"cases"`};if e:=json.NewDecoder(os.Stdin).Decode(&in);e!=nil{panic(e)};var labels,cases []any;for _,label:=range in.Labels{labels=append(labels,map[string]any{"label":label,"normalized":normalizeApprovalOptionLabel(label),"positive":isExplicitApprovalPositiveLabel(label),"negative":isExplicitApprovalNegativeLabel(label)})};for _,c:=range in.Cases{cases=append(cases,map[string]any{"options":c.Options,"approve":autoApprovalInput(c.Options),"reject":oneTapRejectInput(c.Options)})};if e:=json.NewEncoder(os.Stdout).Encode(map[string]any{"labels":labels,"cases":cases});e!=nil{panic(e)}}'''
    args.work_root.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='many-ai-selection-',dir=args.work_root) as root:
        path=Path(root)/'oracle.go';path.write_text(code)
        output=subprocess.check_output([args.go,'run',str(path)],cwd=root,input=json.dumps({'labels':labels,'cases':cases}).encode())
    Path(__file__).with_name('selection-oracle-21d0bc7.json').write_bytes(output)
if __name__=='__main__':main()
