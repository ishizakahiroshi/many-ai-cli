package main
import("encoding/json";"fmt";"os";"strings";"regexp";neturl "net/url")
type tailscaleStatusJSON struct {
	BackendState string `json:"BackendState"`
	Self         struct {
		DNSName string `json:"DNSName"`
		Online  bool   `json:"Online"`
	} `json:"Self"`
}
func trimDNSName(name string) string {
	return strings.TrimSuffix(strings.TrimSpace(name), ".")
}
func tailscaleHTTPSURL(dnsName, token string) string {
	if dnsName == "" {
		return ""
	}
	return fmt.Sprintf("https://%s/?token=%s", dnsName, neturl.QueryEscape(token))
}
var adminURLRe=regexp.MustCompile(`https://login\.tailscale\.com/\S+`)
type Case struct{Name,DNS,Token,Stderr,Status string;Hosts []string}
func main(){var cases []Case;if err:=json.NewDecoder(os.Stdin).Decode(&cases);err!=nil{panic(err)};out:=[]map[string]any{}
for _,c:=range cases{var status tailscaleStatusJSON;err:=json.Unmarshal([]byte(c.Status),&status);hosts:=[]string{};seen:=map[string]bool{}
for _,raw:=range c.Hosts{host:=strings.TrimSuffix(strings.TrimSpace(raw),".");key:=strings.ToLower(host);if host!=""&&!seen[key]{hosts=append(hosts,host);seen[key]=true}}
out=append(out,map[string]any{"name":c.Name,"normalized":trimDNSName(c.DNS),"url":tailscaleHTTPSURL(c.DNS,c.Token),"admin":adminURLRe.FindString(c.Stderr),"hosts":hosts,"decoded":err==nil,"dns":status.Self.DNSName,"online":status.Self.Online,"backend":status.BackendState})}
if err:=json.NewEncoder(os.Stdout).Encode(out);err!=nil{panic(fmt.Sprint(err))}}
