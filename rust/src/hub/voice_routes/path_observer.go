// Synthetic stdlib-only oracle; no provider, network, config or home access.
package main
import("encoding/json";"net/url";"os";"strings")
func main(){out:=[]string{};for _,item:=range[][2]string{{"http://127.0.0.1:7777/base/?old=yes#f","inference"},{"http://127.0.0.1:7777/a%2Fb/","//inference"},{"http://127.0.0.1:7777/a%25b","/what?x#y"},{"http://127.0.0.1:7777/%FF","/inference"}}{u,err:=url.Parse(strings.TrimSpace(item[0]));if err!=nil{panic(err)};suffix:=item[1];if !strings.HasPrefix(suffix,"/"){suffix="/"+suffix};u.Path=strings.TrimRight(u.Path,"/")+suffix;u.RawQuery="";u.Fragment="";out=append(out,u.String())};json.NewEncoder(os.Stdout).Encode(out)}
