from pathlib import Path
import subprocess,json,base64,os
here=Path(__file__).resolve().parent
root=here.parents[3]
start='<!-- many-ai-cli:approval-rules -->';end='<!-- /many-ai-cli:approval-rules -->'
bodies=[b'',b'custom no newline',b'custom\n',b'custom\r\n',b'custom\xff\xfe',b'\n@~/.many-ai-cli/approval-rules.md\n',b'custom\n\n @~/.many-ai-cli/approval-rules.md \r\nmore\n',b'custom\n@~/.many-ai-cli/approval-rules.md\n@~/.many-ai-cli/approval-rules.md\n',f'before\n{start}\n<!-- version: 1 -->\nstale\n{end}\nafter'.encode(),f'before\n{start}\n<!-- version: 24 -->\nkeep\n{end}\nafter'.encode(),b'<!-- any-ai-cli:approval-rules -->\nold\n<!-- /any-ai-cli:approval-rules -->\n',start.encode()+b'\nunterminated',b'\n<!-- many-ai-cli:delegation -->\nbody\n<!-- /many-ai-cli:delegation -->\n']
cases=[]
for provider in ['claude','codex','copilot','cursor-agent','opencode','grok','shell']:
 for i,body in enumerate(bodies):cases.append(dict(Name=f'{provider}-{i}',Provider=provider,Body=base64.b64encode(body).decode()))
(here/'cases.json').write_text(json.dumps(cases,indent=2),encoding='utf-8')
env=os.environ.copy();env.update(GOPROXY='off',GOSUMDB='off',GOTOOLCHAIN='local')
result=subprocess.run([str(root.parent/'.toolchains/go1268/go/bin/go.exe'),'run',str(here/'oracle.go')],cwd=root,input=json.dumps(cases).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=env)
if result.returncode:raise RuntimeError(result.stderr.decode())
(here/'golden.json').write_bytes(result.stdout)
print('Generated',len(cases),'fixed-Go synthetic injection/removal cases')
