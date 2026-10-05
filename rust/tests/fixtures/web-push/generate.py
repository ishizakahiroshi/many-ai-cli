from pathlib import Path
root=Path.cwd();out=root/'rust/tests/fixtures/web-push'
s=Path(r'D:/.gomodcache/github.com/!sher!clock!holmes/webpush-go@v1.4.0/webpush.go').read_text()
def fn(start):
 a=s.index(start);return s[a:s.index('\n}\n',a)+3]
a=s.index('import (');imports=s[a:s.index('\n)',a)+2]+'\nimport("os";"encoding/json")'
pieces=[]
for name in ['type HTTPClient interface','type Options struct','type Keys struct','type Subscription struct']:
 a=s.index(name);pieces.append(s[a:s.index('\n}',a)+2])
body=fn('func SendNotificationWithContext(').replace('localPrivateKey, x, y, err := elliptic.GenerateKey(curve, rand.Reader)','localPrivateKey := make([]byte,32); localPrivateKey[31]=2; x,y := curve.ScalarBaseMult(localPrivateKey)')
program='package main\n'+imports+'''
type Urgency string
func isValidUrgency(Urgency)bool{return false}
const MaxRecordSize uint32=4096
var ErrMaxPadExceeded=errors.New("payload has exceeded the maximum length")
var saltFunc=func()([]byte,error){return make([]byte,16),nil}
func getVAPIDAuthorizationHeader(string,string,string,string,time.Time)(string,error){return "synthetic-test-only",nil}
var _=rand.Reader
'''+ '\n'.join(pieces+[body,fn('func decodeSubscriptionKey('),fn('func getHKDFKey('),fn('func pad(')])+'''
type Capture struct{Body string}
func(c *Capture)Do(r *http.Request)(*http.Response,error){data,e:=io.ReadAll(r.Body);if e!=nil{return nil,e};c.Body=base64.RawURLEncoding.EncodeToString(data);return &http.Response{StatusCode:201,Body:http.NoBody},nil}
func main(){private:=make([]byte,32);private[31]=1;x,y:=elliptic.P256().ScalarBaseMult(private);keys:=Keys{P256dh:base64.RawURLEncoding.EncodeToString(elliptic.Marshal(elliptic.P256(),x,y)),Auth:base64.RawURLEncoding.EncodeToString(make([]byte,16))};rows:=[]any{};for _,payload:=range []string{"synthetic push payload", "合成 Web Push だけです"}{client:=&Capture{};_,e:=SendNotificationWithContext(context.Background(),[]byte(payload),&Subscription{Endpoint:"https://example.invalid/push",Keys:keys},&Options{HTTPClient:client,TTL:300});if e!=nil{panic(e)};rows=append(rows,map[string]string{"payload":payload,"body":client.Body,"p256dh":keys.P256dh,"auth":keys.Auth})};data,e:=json.MarshalIndent(rows,"","  ");if e!=nil{panic(e)};if e=os.WriteFile(os.Args[1],data,0600);e!=nil{panic(e)}}
'''
(out/'oracle.go').write_text(program)
print('two synthetic pinned webpush-go encryption vectors generated')
