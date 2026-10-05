package main
import("context";"encoding/json";"fmt";"strings";"regexp";"strconv";"os")
type Status struct{LoggedIn bool `json:"logged_in"`;Plan string `json:"plan,omitempty"`;Method string `json:"method,omitempty"`}
type claudeAdapter struct{}
func(claudeAdapter)LaunchEnv(string)[]string{return nil}
var supplied string
func runVendorCLI(context.Context,string,[]string,[]string)(string,int,error){return supplied,0,nil}
var grokLoginMethods=[]string{"grok.com","x.ai"}
var openCodeCredentialCountRe=regexp.MustCompile(`(\d+)\s+credentials?`)
type claudeAuthStatus struct {
	LoggedIn         bool   `json:"loggedIn"`
	AuthMethod       string `json:"authMethod"`
	SubscriptionType string `json:"subscriptionType"`
}

func (a claudeAdapter) Status(ctx context.Context, profileDir string) (Status, error) {
	out, _, err := runVendorCLI(ctx, "claude", []string{"auth", "status"}, a.LaunchEnv(profileDir))
	if err != nil {
		return Status{}, err
	}
	// 未ログイン時は exit code 1 だが stdout には JSON が出るので、終了コードでは
	// なく本文で判定する。
	var parsed claudeAuthStatus
	if jsonErr := json.Unmarshal([]byte(strings.TrimSpace(out)), &parsed); jsonErr != nil {
		// 出力そのものは載せない（アカウント情報を含みうる）。
		return Status{}, fmt.Errorf("could not read `claude auth status` output")
	}
	if !parsed.LoggedIn {
		return Status{LoggedIn: false}, nil
	}
	return Status{
		LoggedIn: true,
		Plan:     parsed.SubscriptionType,
		Method:   parsed.AuthMethod,
	}, nil
}

func parseCodexLoginStatus(out string, exitCode int) Status {
	lower := strings.ToLower(out)
	if strings.Contains(lower, "not logged in") {
		return Status{LoggedIn: false}
	}
	if exitCode != 0 && !strings.Contains(lower, "logged in") {
		return Status{LoggedIn: false}
	}
	status := Status{LoggedIn: true}
	switch {
	case strings.Contains(lower, "chatgpt"):
		status.Method = "chatgpt"
	case strings.Contains(lower, "api key"), strings.Contains(lower, "api-key"):
		status.Method = "api-key"
	}
	return status
}

func parseGrokModelsStatus(out string) Status {
	lower := strings.ToLower(out)
	if strings.Contains(lower, "not authenticated") || strings.Contains(lower, "not logged in") {
		return Status{LoggedIn: false}
	}
	if !strings.Contains(lower, "logged in") && !strings.Contains(lower, "authenticated") {
		return Status{LoggedIn: false}
	}
	status := Status{LoggedIn: true}
	for _, method := range grokLoginMethods {
		if strings.Contains(lower, method) {
			status.Method = method
			break
		}
	}
	return status
}

func parseOpenCodeProvidersList(out string) Status {
	lower := strings.ToLower(out)
	count := -1
	if m := openCodeCredentialCountRe.FindStringSubmatch(lower); len(m) == 2 {
		if n, err := strconv.Atoi(m[1]); err == nil {
			count = n
		}
	}
	if count == 0 {
		return Status{LoggedIn: false}
	}
	if count < 0 && !strings.Contains(lower, "credential") {
		// 想定外の出力形。ログイン済みと決めつけない。
		return Status{LoggedIn: false}
	}
	status := Status{LoggedIn: true}
	// 今回の対象は Go 月額契約に紐づく credential。他 provider の API キーだけが
	// 入っている profile を「サブスクリプション」と見せないよう区別する。
	if strings.Contains(lower, "opencode go") {
		status.Plan = "go"
	}
	return status
}

type Case struct{Provider string `json:"provider"`;Output string `json:"output"`;Code int `json:"code"`}
func main(){var cases []Case;data,e:=os.ReadFile(os.Args[1]);if e!=nil{panic(e)};if e=json.Unmarshal(data,&cases);e!=nil{panic(e)};items:=[]any{};for _,c:=range cases{var status Status;var err error;switch c.Provider{case"claude":supplied=c.Output;status,err=(claudeAdapter{}).Status(context.Background(),"synthetic");case"codex":status=parseCodexLoginStatus(c.Output,c.Code);case"grok":status=parseGrokModelsStatus(c.Output);case"opencode":status=parseOpenCodeProvidersList(c.Output)};errorText:="";if err!=nil{errorText=err.Error()};items=append(items,map[string]any{"status":status,"error":errorText})};encoded,e:=json.MarshalIndent(items,"","  ");if e!=nil{panic(e)};if e=os.WriteFile(os.Args[2],encoded,0600);e!=nil{panic(e)}}
