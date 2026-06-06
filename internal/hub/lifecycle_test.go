package hub

import "testing"

func TestLocalHubURLEscapesToken(t *testing.T) {
	got := localHubURL(47777, "/", "a+b&x=1")
	want := "http://127.0.0.1:47777/?token=a%2Bb%26x%3D1"
	if got != want {
		t.Errorf("localHubURL = %q, want %q", got, want)
	}
}
