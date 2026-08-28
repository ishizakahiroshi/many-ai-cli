many-ai-cli v0.7.0 を、はじめて入れる人へ

AI にコードを書いてもらうようになってから、ターミナルが増えました。

Claude Code、Codex CLI、Copilot CLI などを同時に動かしていると、どの画面が止まっているのか分からなくなります。承認待ちのまま静かに止まっている画面を探すために、ターミナルを順番に見て回ることになります。

many-ai-cli は、AI コーディング CLI の代わりになるものではありません。複数の CLI をそのまま動かしながら、承認待ちやタスク完了などの状態を、ブラウザの 1 画面に集めるローカル Web ダッシュボードです。

自作のツールなので、使い方を短くまとめておきます。

GitHub リポジトリ
https://github.com/ishizakahiroshi/many-ai-cli

機能紹介ページ
https://ishizakahiroshi.com/work.html?id=many-ai-cli

Windows なら、PowerShell に次の 1 行を貼り付けます。

winget install ishizakahiroshi.many-ai-cli; & "$env:LOCALAPPDATA\Microsoft\WinGet\Links\many-ai-cli.exe" setup

インストールと初回セットアップが終わると、デスクトップに「MANY-AI-CLI」というショートカットができます。

次回からは、そのショートカットをダブルクリックします。タスクトレイに many-ai-cli のアイコンが出るので、アイコンをクリックして「Hub を開く」を選んでください。ブラウザに Hub の画面が開きます。

Hub の画面で「+ 新しいセッション」を押し、使いたい AI コーディング CLI を選びます。Claude Code や Codex CLI などの本体は、many-ai-cli とは別にインストールしておきます。

止めるときは、タスクトレイの「Hub を停止」を使います。Hub の画面にある停止ボタンや、別のターミナルから many-ai-cli stop を実行しても止められます。

macOS では、Homebrew を使って次のように入れます。

brew install --cask ishizakahiroshi/tap/many-ai-cli && many-ai-cli setup

Linux では、リリースページから自分の環境に合うパッケージを取得し、インストール後に many-ai-cli setup を実行します。Debian や Ubuntu では deb パッケージ、RHEL 系では rpm パッケージを使います。

macOS と Linux では、セットアップ後に「Many AI Hub Start」と「Many AI Hub Stop」のショートカットが作られます。「Many AI Hub Start」を起動するとブラウザとコンソールが開きます。このコンソールが Hub の実体なので、使っている間は閉じずに最小化してください。Linux の GNOME では、最初だけショートカットを右クリックして「起動を許可」を選びます。

Windows、macOS、Linux の詳しい手順と、手動ダウンロードを含む別の導入方法は README にまとめています。

https://github.com/ishizakahiroshi/many-ai-cli/blob/main/README.ja.md#はじめるインストール直後にやること

なお、many-ai-cli の Hub は通常、自分の PC の中だけで待ち受けます。セッションログには自分の入力と AI の出力が残るので、仕事の情報や認証情報を入力する場合は、ログも含めて自分のデータとして扱ってください。

いきなり全部の機能を使う必要はありません。まずは AI コーディング CLI を 1 本だけ起動して、ブラウザから状態を見られるところまで試すのがおすすめです。そこから 2 本、3 本と増やすと、どの場面で便利なのかが分かりやすいと思います。

使ってみて「ここが不便」というところがあれば、GitHub の Issue で教えてください。

https://github.com/ishizakahiroshi/many-ai-cli/issues
