"""Pinned-Go pure mobile URL/typed status oracle; no daemon, network or home I/O."""
import json,pathlib,subprocess
here=pathlib.Path(__file__).resolve().parent
root=here.parents[3]
probe=(root/'internal/hub/tailscale_probe.go').read_text(encoding='utf-8')
handlers=(root/'internal/hub/tailscale_handlers.go').read_text(encoding='utf-8')
def decl(source,start):
    at=source.index(start);end=source.index('\n}',at)+2
    return source[at:end]
oracle=r'''package main
import("encoding/json";"fmt";"os";"strings";"regexp";neturl "net/url")
'''+decl(probe,'type tailscaleStatusJSON struct')+'\n'+decl(probe,'func trimDNSName(')+'\n'+decl(handlers,'func tailscaleHTTPSURL(')+r'''
var adminURLRe=regexp.MustCompile(`https://login\.tailscale\.com/\S+`)
type Case struct{Name,DNS,Token,Stderr,Status string;Hosts []string}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};out:=[]map[string]any{}
for _,c:=range cases{var status tailscaleStatusJSON;err:=json.Unmarshal([]byte(c.Status),&status);hosts:=[]string{};seen:=map[string]bool{}
for _,raw:=range c.Hosts{host:=strings.TrimSuffix(strings.TrimSpace(raw),".");key:=strings.ToLower(host);if host!=""&&!seen[key]{hosts=append(hosts,host);seen[key]=true}}
out=append(out,map[string]any{"name":c.Name,"normalized":trimDNSName(c.DNS),"url":tailscaleHTTPSURL(c.DNS,c.Token),"admin":adminURLRe.FindString(c.Stderr),"hosts":hosts,"decoded":err==nil,"dns":status.Self.DNSName,"online":status.Self.Online,"backend":status.BackendState})}
if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(fmt.Sprint(err))}}
'''
(here/'oracle.go').write_text(oracle,encoding='utf-8')
cases=[]
def case(name,dns='node.example',token=' +&/~!()*\'日本',stderr='',hosts=None,status='{"BackendState":"Running","Self":{"DNSName":"node.","Online":true}}'):
    cases.append(dict(name=name,dns=dns,token=token,stderr=stderr,hosts=hosts or ['Node.',' node ','192.0.2.1','HOST.'],status=status))
for name,dns in [('empty',''),('single-dot','.'),('double-dot','Node..'),('space',' \r\nNode.\u0085'),('go-space','\u2005Node.\u3000'),('bom','\ufeffNode.')]:case(name,dns=dns)
for name,stderr in [('empty-url','https://login.tailscale.com/ https://login.tailscale.com/admin/dns'),('first-url','prefix https://login.tailscale.com/admin/dns?x=1. second https://login.tailscale.com/other'),('unicode-space','https://login.tailscale.com/admin\u00a0keep'),('vertical-tab','https://login.tailscale.com/admin\u000bkeep'),('newline','https://login.tailscale.com/admin\nend'),('form-feed','https://login.tailscale.com/admin\fend'),('wrong-host','https://evil.invalid/login.tailscale.com/admin')]:case(name,stderr=stderr)
for name,status in [('null','null'),('empty-object','{}'),('array','[]'),('wrong-backend','{"BackendState":12}'),('wrong-online','{"Self":{"Online":"true"}}'),('null-fields','{"BackendState":null,"Self":{"DNSName":null,"Online":null}}'),('duplicate-merge','{"Self":{"DNSName":"node."},"Self":{"Online":true}}'),('duplicate-null','{"Self":{"DNSName":"node."},"Self":null}'),('folded','{"backendstate":"Running","sELF":{"dnsname":"folded.","online":true}}'),('wrong-dns','{"Self":{"DNSName":[]}}'),('ignored-large','{"unknown":1e999,"Self":{"DNSName":"safe"}}'),('trailing','{} {}'),('wrong-self','{"Self":true}'),('duplicate-leaf','{"BackendState":"Running","BackendState":"NeedsLogin"}')]:case(name,status=status)
case('go-simple-lower',hosts=['İ.','i','K.','k','Σ.','σ','ς'])
case('all-token-bytes',token=''.join(chr(i) for i in range(128)))
case('query-unicode',token='🙂 e\u0301 　')
(here/'cases.json').write_text(json.dumps(cases,ensure_ascii=False,indent=2),encoding='utf-8')
result=subprocess.run([str(root.parent/'.toolchains/go1268/go/bin/go.exe'),'run',str(here/'oracle.go')],cwd=root,input=json.dumps(cases).encode(),stdout=subprocess.PIPE,stderr=subprocess.PIPE)
if result.returncode:raise RuntimeError(result.stderr.decode())
(here/'golden.json').write_bytes(result.stdout)
print('Generated',len(cases),'synthetic pure mobile cases')
