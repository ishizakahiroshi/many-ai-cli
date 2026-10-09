#!/usr/bin/env python3
"""Synthetic owned version outputs. Never runs a provider or an updater."""
import base64,copy,json,pathlib
HERE=pathlib.Path(__file__).resolve().parent
cases=[]
def definition(id='alpha',**changes):
 d={'schema_version':1,'id':id,'display_name':'Synthetic '+id,'launch':{'executable':id}}
 d.update(changes);return d
def add(name,**changes):
 c={'name':name,'kind':'http','method':'POST','body':'','definitions':[definition()], 'lookup':{'alpha':'/synthetic/bin/alpha'},'modified':{'/synthetic/bin/alpha':'2026-10-03T12:34:56.987654321+09:00'},'processes':{'/synthetic/bin/alpha':{'output':base64.b64encode(b'synthetic v1.2.3\n').decode()}}}
 c.update(changes);cases.append(c)
for method in ['GET','PUT','PATCH','DELETE','HEAD','OPTIONS','post']:
 add('method-'+method,method=method,body='{bad')
for name,body in [('empty',''),('object','{}'),('null','null'),('space',' \t\n'),('missing-close','{'),('string','"alpha"'),('array','[]'),('number','3'),('false','false'),('unknown','{"ignored":true}'),('null-providers','{"providers":null}'),('empty-providers','{"providers":[]}'),('blank-providers','{"providers":[" ","\t"]}'),('trim-dedupe','{"providers":[" alpha ","alpha","missing","missing"]}'),('case-sensitive','{"providers":["ALPHA","alpha"]}'),('folded-key','{"PrOvIdErS":["alpha"]}'),('long-s-key','{"providerſ":["missing"]}'),('null-element','{"providers":[null,"alpha"]}'),('wrong-element','{"providers":[3]}'),('wrong-providers','{"providers":"alpha"}'),('duplicate-last','{"providers":["missing"],"providers":["alpha"]}'),('duplicate-null','{"providers":["missing"],"providers":null}'),('duplicate-reused-null','{"providers":["alpha"],"providers":[null]}'),('trailing-object','{"providers":["alpha"]}{"providers":["missing"]}'),('trailing-garbage','{} nope'),('body-limit','@limit'),('early-value-large-trailing','@trailing')]:
 add('body-'+name,body='' if body.startswith('@') else body,body_mode=body[1:] if body.startswith('@') else '')
add('framing-empty-chunked',chunked=True)
add('framing-empty-chunked-before-registry',chunked=True,registry_missing=True)
add('framing-chunked-nonempty',chunked=True,body='{}')
add('framing-explicit-zero',content_length=0)
add('framing-explicit-positive-eof',content_length=7)
add('framing-artificial-zero-ignores-body',content_length=0,body='{bad')
for body in ['','{bad','{}','null']:
 add('no-registry-'+repr(body),registry_missing=True,body=body)
add('empty-registry',definitions=[])
add('default-order-and-disabled',definitions=[definition('zulu'),definition('codex'),definition('alpha',enabled=False),definition('claude')],lookup={})
add('explicit-disabled-runs',definitions=[definition(enabled=False)],body='{"providers":["alpha"]}')
add('explicit-request-order',definitions=[definition('zulu'),definition('alpha')],body='{"providers":["zulu","alpha","missing"]}',lookup={})
add('unknown-preserves-old-shape',body='{"providers":["missing"]}')
outputs=[('normal',b'synthetic v1.2.3\n',0,False),('multiline',b'\n\r\n  synthetic v1 \r\nbuild\n',0,False),('stderr-only',b'warning v2\n',0,False),('empty',b'',0,False),('whitespace',b' \r\n\t',0,False),('exit-one',b'synthetic boom\n',1,False),('signal',b'partial',-1,False),('timeout',b'partial\n',-1,True),('timeout-exit-zero',b'complete',0,True),('cap',b'x'*8300,0,False),('cap-empty-after',b' '*8192+b'version',0,False),('cap-utf8',b'x'*8191+'あ'.encode(),0,False),('invalid-utf8',b'\xf0\x9f\xff v2\n',0,False),('html','<v1>&\u2028\n'.encode(),0,False),('unicode-space','\u0085\u00a0\u2003\n\u3000 日本語 v3 \u2009\n'.encode(),0,False)]
for name,output,exit_code,timed_out in outputs:
 add('one-'+name,kind='one',definition=definition(),processes={'/synthetic/bin/alpha':{'output':base64.b64encode(output).decode(),'exit_code':exit_code,'timed_out':timed_out}})
add('one-start-failure',kind='one',definition=definition(),processes={'/synthetic/bin/alpha':{'start_failed':True}})
add('one-no-mtime',kind='one',definition=definition(),modified={})
add('one-no-launch',kind='one',definition=definition(launch=None))
add('one-no-hit',kind='one',definition=definition(),lookup={})
add('one-secondary-candidate',kind='one',definition=definition(launch={'executable':'absent','executable_candidates':['','alpha','other']}))
add('one-first-hit-wins',kind='one',definition=definition(launch={'executable':'alpha','executable_candidates':['other']}),lookup={'alpha':'/synthetic/bin/alpha','other':'/synthetic/bin/other'})
add('one-empty-executable',kind='one',definition=definition(launch={'executable_candidates':['alpha']}))
add('one-version-args',kind='one',definition=definition(update={'version_args':['version','--plain'],'args':['update'],'executable':'updater','enabled':False,'login_may_be_required':True,'timeout_seconds':99}))
add('one-empty-version-args',kind='one',definition=definition(update={'version_args':[]}))
add('one-blank-version-arg',kind='one',definition=definition(update={'version_args':['']}))
for i,text in enumerate(['','\r','\r\n','\n a\r\nb','\n a\rb','\u0085\u00a0\u2003\n日本語\u3000','\u001ccontrol','\ufeffversion','\u200bversion']):
 add('line-'+str(i),kind='line',text=text)
for i,ids in enumerate([[],[''],[' ','\t'],[' alpha ','alpha','Alpha','beta','beta'],['\u0085alpha\u3000','\ufeffalpha','\u001calpha']]):
 add('normalize-'+str(i),kind='normalize',ids=ids)
(HERE/'cases.json').write_text(json.dumps(cases,ensure_ascii=True,indent=2)+'\n')
print(len(cases),'cases')
