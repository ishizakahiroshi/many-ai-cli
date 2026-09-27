package hub

import "testing"

// 出荷している provider が表から漏れていないこと。漏れると
// providerFeatureSourceFor の既定（none / derived）へ静かに落ち、チャット面だけが
// 消えて誰も気づかない。
func TestProviderFeatureSourceCoversShippedProviders(t *testing.T) {
	for _, provider := range KnownApprovalProviders() {
		if provider == "common" {
			continue // 承認パターンの共有欄で、provider ではない
		}
		found := false
		for _, row := range providerFeatureSources {
			if row.Provider == provider {
				found = true
				break
			}
		}
		if !found {
			t.Errorf("provider %q の行が providerFeatureSources に無い", provider)
		}
	}
}

// 表に無い provider（config.yaml の custom_providers 等）の既定。
// 「知らない CLI にはトランスクリプトが在ることにしない」を固定する。
func TestProviderFeatureSourceUnknownProviderDefaults(t *testing.T) {
	row := providerFeatureSourceFor("some-unknown-cli")
	if row.StructuredTranscript != featureSourceNone {
		t.Errorf("StructuredTranscript = %q, want %q", row.StructuredTranscript, featureSourceNone)
	}
	if row.ApprovalMarker != featureSourceDerived {
		t.Errorf("ApprovalMarker = %q, want %q", row.ApprovalMarker, featureSourceDerived)
	}
	if providerHasStructuredTranscript("some-unknown-cli") {
		t.Error("未知の provider がトランスクリプトを読める扱いになっている")
	}
	if providerApprovalMarkerFromTranscript("some-unknown-cli") {
		t.Error("未知の provider の承認供給元がトランスクリプトになっている")
	}
}

// ApprovalMarker が native の行は、読む先のトランスクリプトが無ければ成立しない。
// 表の編集でこの組み合わせを作ると、承認が「読めないファイルから来る」ことになり、
// approvalMarkerTranscriptMissLimit の退避に頼って無音で遅れるだけになる。
func TestApprovalMarkerNativeRequiresStructuredTranscript(t *testing.T) {
	for _, row := range providerFeatureSources {
		if row.ApprovalMarker == featureSourceNative && row.StructuredTranscript != featureSourceNative {
			t.Errorf("provider %q: 承認が native なのに StructuredTranscript が %q", row.Provider, row.StructuredTranscript)
		}
		if row.ApprovalMarker == featureSourceNone {
			t.Errorf("provider %q: ApprovalMarker に none は無い（端末に確認画面は出る）", row.Provider)
		}
	}
}

// トランスクリプトを読める provider の現時点の全量。増やすときは表とこのテストを
// 同時に触る（＝意図した変更としてレビューに出る）ことを強制する。
func TestStructuredTranscriptProvidersAreExactlyTheMeasuredOnes(t *testing.T) {
	want := map[string]bool{"claude": true, "codex": true, "command-code": true}
	for _, row := range providerFeatureSources {
		got := row.StructuredTranscript == featureSourceNative
		if got != want[row.Provider] {
			t.Errorf("provider %q: StructuredTranscript native = %v, want %v", row.Provider, got, want[row.Provider])
		}
	}
}

// 表（読もうとするか）とパーサ（読み方）は別の場所にある。片方だけ足すと、
// 「読める扱いなのにパーサが無い」（チャットが無言で空になる）か「パーサはあるのに
// 読まない」（作ったのに使われない）になる。両方向で突き合わせる。
func TestAgentChatParsersMatchFeatureSourceTable(t *testing.T) {
	for _, row := range providerFeatureSources {
		parse := agentChatRecordParserFor(row.Provider)
		native := row.StructuredTranscript == featureSourceNative
		if native && parse == nil {
			t.Errorf("provider %q: 表は native なのに record parser が無い", row.Provider)
		}
		if !native && parse != nil {
			t.Errorf("provider %q: record parser があるのに表が %q", row.Provider, row.StructuredTranscript)
		}
	}
	if agentChatRecordParserFor("some-unknown-cli") != nil {
		t.Error("未知の provider に parser が返った")
	}
}

// C1 の本題。**チャット面がトランスクリプトを読める provider を増やしても、
// その provider の承認供給元は VT ミラーのまま**であること。
//
// 以前は 1 つの provider 一覧（isAgentChatProvider）が両方を答えていたので、
// チャットの対象を増やす変更が承認の同一性（candidateKey + sourceEpoch の 1 本）
// を巻き込んだ。表の 2 列が独立していることを、実際に「チャットだけ native」の
// 行を差し込んで確かめる。
func TestApprovalMarkerSourceDoesNotFollowTranscriptReadability(t *testing.T) {
	const provider = "test-chat-only"
	original := providerFeatureSources
	// 既存 provider の引き当ては先頭一致なので、行を足しても他のテストの結果は変わらない。
	providerFeatureSources = append(append([]providerFeatureSource(nil), original...), providerFeatureSource{
		Provider:             provider,
		StructuredTranscript: featureSourceNative,
		ApprovalMarker:       featureSourceDerived,
	})
	t.Cleanup(func() { providerFeatureSources = original })

	if !providerHasStructuredTranscript(provider) {
		t.Fatal("前提が違う: 差し込んだ行のチャット読み取りが native になっていない")
	}
	if providerApprovalMarkerFromTranscript(provider) {
		t.Error("チャットを読めるだけで承認供給元がトランスクリプトへ移った")
	}
	ses := &session{Provider: provider, agentChatPath: "transcript.jsonl"}
	if approvalMarkerSourceIsTranscriptLocked(ses) {
		t.Error("トランスクリプトを読めているだけで承認の供給元が切り替わった（VT ミラーのままであるべき）")
	}
}
