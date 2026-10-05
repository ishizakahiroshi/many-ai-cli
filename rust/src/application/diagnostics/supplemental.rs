use super::*;
use crate::{
    config::{self, Resource},
    files::safe_fs::Dir,
    profile::subscriptions::diagnostics::{diagnostic_entries, sync_drift},
    proto::core::TaskCancellation,
};
use std::collections::BTreeSet;
fn read(owner: &Diagnostics, path: &Path) -> io::Result<Vec<u8>> {
    if !owner.deps.paths.is_trial() {
        return std::fs::read(path);
    }
    let relative = path.strip_prefix(owner.deps.paths.root()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "diagnostic path escapes trial",
        )
    })?;
    let parts = relative.components().collect::<Vec<_>>();
    let mut dir = Dir::open(owner.deps.paths.root())?;
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        let std::path::Component::Normal(name) = part else {
            return Err(io::Error::other("invalid diagnostic component"));
        };
        dir = dir.child_dir(
            name.to_str()
                .ok_or_else(|| io::Error::other("invalid diagnostic component"))?,
            false,
        )?;
    }
    let Some(std::path::Component::Normal(name)) = parts.last() else {
        return Err(io::Error::other("invalid diagnostic file"));
    };
    let bytes = dir.read(
        name.to_str()
            .ok_or_else(|| io::Error::other("invalid diagnostic file"))?,
        8 * 1024 * 1024 + 1,
    )?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(io::Error::other("diagnostic file exceeds bound"));
    }
    Ok(bytes)
}
pub(super) fn custom(owner: &Diagnostics, cfg: &Config) -> Vec<Check> {
    let effective = config::effective_custom_providers(&cfg.custom_providers);
    if effective.is_empty() {
        return vec![];
    }
    let ids = effective.iter().map(|p| p.id.as_str()).collect::<Vec<_>>();
    let mut out = vec![Check::new(
        "custom provider",
        "OK",
        format!(
            "玄人設定の custom_providers を {} 件使用中です（{}）。built-in provider と異なり、利用規約・実行結果の責任は利用者側です",
            ids.len(),
            ids.join(", ")
        ),
        "",
    )];
    let mut missing = vec![];
    for p in &effective {
        match p.argv() {
            Ok(args) if !args.is_empty() => {
                if owner.deps.io.look_path(&args[0]).is_err() {
                    missing.push(format!("{} ({})", p.id, args[0]))
                }
            }
            _ => missing.push(format!("{} (command は分解できません)", p.id)),
        }
    }
    out.push(if missing.is_empty() {
        Check::new(
            "custom provider path",
            "OK",
            "custom_providers の command は全て PATH 上に見つかりました",
            "",
        )
    } else {
        Check::new(
            "custom provider path",
            "WARN",
            format!(
                "custom_providers の一部が PATH 上に見つかりません: {}",
                missing.join(", ")
            ),
            "該当コマンドをインストールするか PATH を確認し、many-ai-cli を再起動してください",
        )
    });
    let sources = effective
        .iter()
        .filter(|p| !p.approval_pattern_source.trim().is_empty())
        .collect::<Vec<_>>();
    if !sources.is_empty() {
        let missing = sources
            .iter()
            .filter(|p| {
                Dir::open(&owner.deps.paths.resource(Resource::ApprovalPatterns))
                    .and_then(|d| d.metadata(&format!("{}.json", p.id)))
                    .is_err()
            })
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>();
        out.push(if missing.is_empty(){Check::new("custom provider approval pattern","OK",format!("approval_pattern_source を設定している custom_providers は全て同期済みです（{} 件）",sources.len()),"")}else{Check::new("custom provider approval pattern","WARN",format!("approval_pattern_source を設定しているのに未同期の custom_providers があります（{}）。Hub を起動（または再起動）すると同期を試みます。それでも消えない場合は source（絶対パスなら ~/.many-ai-cli 配下、URL なら https://raw.githubusercontent.com のみ許可）を確認し、hub.log を見てください",missing.join(", ")),"")});
    }
    out
}
fn keys(keys: &[String]) -> String {
    if keys.len() <= 4 {
        keys.join(", ")
    } else {
        format!("{} ほか {} 件", keys[..4].join(", "), keys.len() - 4)
    }
}
pub(super) async fn subscriptions(
    owner: &Diagnostics,
    cfg: &Config,
    cancel: &Cancellation,
) -> Vec<Check> {
    let mut out = vec![];
    let mut providers = crate::application::subscriptions::cli::PROVIDERS.to_vec();
    providers.sort();
    let extra = cfg
        .subscriptions
        .keys()
        .map(String::as_str)
        .filter(|p| !providers.contains(p))
        .collect::<Vec<_>>();
    providers.extend(extra);
    for provider in providers {
        let Some(profiles) = cfg.subscriptions.get(provider) else {
            continue;
        };
        let supported = crate::application::subscriptions::cli::env_key(provider).is_some();
        let mut seen = BTreeSet::new();
        for profile in profiles {
            let id = config::normalize_subscription_id(&profile.id);
            let name = format!(
                "{provider} / {}",
                if profile.name.is_empty() {
                    &profile.id
                } else {
                    &profile.name
                }
            );
            let issue = if let Err(e) = config::validate_subscription_provider(provider) {
                Some(e.to_string())
            } else if let Err(e) = config::validate_subscription_id(&id) {
                Some(e.to_string())
            } else if !seen.insert(id.clone()) {
                Some(format!("duplicate profile id {id:?}"))
            } else {
                None
            };
            if let Some(issue) = issue {
                out.push(Check::new(
                    "subscription",
                    "FAIL",
                    format!("{name}: 設定が壊れています（{issue}）"),
                    "~/.many-ai-cli/config.yaml の subscriptions セクションを修正してください",
                ));
                continue;
            }
            let path = match config::resolve_subscription_profile_dir(
                &owner.deps.paths,
                provider,
                profile,
                Some(&owner.deps.home),
            ) {
                Ok(path) => path,
                Err(error) => {
                    out.push(Check::new(
                        "subscription",
                        "FAIL",
                        format!("{name}: 設定が壊れています（{error}）"),
                        "~/.many-ai-cli/config.yaml の subscriptions セクションを修正してください",
                    ));
                    continue;
                }
            };
            if !supported {
                out.push(Check::new(
                    "subscription",
                    "WARN",
                    format!("{name}: この provider は profile 分離に未対応です"),
                    "セッション起動時には選べません。設定を残しても害はありません",
                ));
                continue;
            }
            if profile.enabled == Some(false) {
                out.push(Check::new(
                    "subscription",
                    "OK",
                    format!("{name}: 無効化されています"),
                    "",
                ));
                continue;
            }
            if crate::profile::subscriptions::check_path(&owner.deps.paths, &path).is_err()
                || !path.is_dir()
            {
                out.push(Check::new(
                    "subscription",
                    "WARN",
                    format!("{name}: ログインが必要です（profile ディレクトリがまだありません）"),
                    "Settings のサブスクリプション欄で「ログイン」を実行してください",
                ));
                continue;
            }
            let task = TaskCancellation::default();
            let status =
                owner
                    .deps
                    .subscription_cli
                    .status(provider, &path, Duration::from_secs(5), &task);
            tokio::pin!(status);
            let result = tokio::select! {v=&mut status=>v,_=cancel.cancelled()=>{task.cancel();status.await}};
            out.push(match result {
                Err(_) => Check::new(
                    "subscription",
                    "WARN",
                    format!("{name}: ログイン状態を確認できません"),
                    "対応する公式 CLI が PATH にあるか確認してください",
                ),
                Ok(s) if !s.logged_in => Check::new(
                    "subscription",
                    "WARN",
                    format!("{name}: ログインが必要です"),
                    "Settings のサブスクリプション欄で「ログイン」を実行してください",
                ),
                Ok(s) if !s.plan.is_empty() => Check::new(
                    "subscription",
                    "OK",
                    format!("{name}: ログイン済み（{}）", s.plan),
                    "",
                ),
                Ok(_) => Check::new("subscription", "OK", format!("{name}: ログイン済み"), ""),
            });
            let entries = match diagnostic_entries(
                &owner.deps.paths,
                &owner.deps.home,
                &owner.deps.cwd,
                &owner.deps.environment,
                provider,
                profile,
            ) {
                Ok(e) => e,
                Err(_) => {
                    out.push(Check::new(
                        "subscription",
                        "WARN",
                        format!("{name}: 既定の設定を確認できません"),
                        "既定側の設定ディレクトリの読み取り権限を確認してください",
                    ));
                    continue;
                }
            };
            let pending = entries
                .iter()
                .filter(|e| {
                    std::fs::symlink_metadata(&e.source).is_ok()
                        && std::fs::symlink_metadata(path.join(e.dest)).is_err()
                })
                .map(|e| e.label.clone())
                .collect::<Vec<_>>();
            if !pending.is_empty() {
                let shown = pending
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(" / ");
                let suffix = if pending.len() > 4 {
                    format!(" ほか {} 件", pending.len() - 4)
                } else {
                    String::new()
                };
                out.push(Check::new("subscription","WARN",format!("{name}: 既定の設定のうち {} 件がこの profile にありません（{shown}{suffix}）",pending.len()),"次回このプロファイルでセッションを起動すると自動で持ち込まれます。すぐ入れたい場合は既定側から手でコピーしてください（既にあるものは上書きされません）"));
            }
            if profile.settings_sync == Some(false) {
                out.push(Check::new("subscription","OK",format!("{name}: ユーザー設定の同期は off です（config.yaml の settings_sync: false。既定側の変更は届きません）"),""))
            } else {
                let mut parts = vec![];
                for (label, raw) in [
                    ("この profile が持つ鍵", &profile.profile_owned_keys),
                    ("既定に揃える鍵", &profile.default_wins_keys),
                ] {
                    let list = raw
                        .iter()
                        .map(|v| v.trim())
                        .filter(|v| !v.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>();
                    if !list.is_empty() {
                        parts.push(format!("{label}: {}", keys(&list)))
                    }
                }
                if !parts.is_empty() {
                    out.push(Check::new(
                        "subscription",
                        "OK",
                        format!("{name}: {}", parts.join(" / ")),
                        "",
                    ));
                }
            }
            for entry in &entries {
                let dest = path.join(entry.dest);
                if entry.sync_owned.is_none()
                    || std::fs::symlink_metadata(&entry.source).is_err()
                    || std::fs::symlink_metadata(&dest).is_err()
                {
                    continue;
                }
                match sync_drift(entry, &dest) {
                    Err(_) => {
                        out.push(Check::new("subscription","WARN",format!("{name}: {} を既定の設定と比較できません（どちらかが設定ファイルとして読めません）",entry.label),"既定側と profile 側の設定ファイルが壊れていないか確認してください。読めない間このファイルは同期されません"));
                        break;
                    }
                    Ok((added, changed)) => {
                        let mut parts = vec![];
                        if !added.is_empty() {
                            parts.push(format!(
                                "既定にしかない設定 {} 件（{}）",
                                added.len(),
                                keys(&added)
                            ))
                        }
                        if !changed.is_empty() {
                            parts.push(format!(
                                "値が違う設定 {} 件（{}）",
                                changed.len(),
                                keys(&changed)
                            ))
                        }
                        if !parts.is_empty() {
                            out.push(Check::new("subscription","WARN",format!("{name}: {} が既定の設定と食い違っています。{}",entry.label,parts.join(" / ")),"次回このプロファイルでセッションを起動すると既定の値で同期されます。profile 側の値を残したい場合は、同じ変更を既定側の設定にも入れてください"));
                            break;
                        }
                    }
                }
            }
            for entry in entries.iter().filter(|e| e.mirror) {
                let dest = path.join(entry.dest);
                let Ok(info) = std::fs::symlink_metadata(&dest) else {
                    continue;
                };
                if info.file_type().is_symlink() {
                    continue;
                }
                if std::fs::symlink_metadata(&entry.source)
                    .is_ok_and(|i| i.file_type().is_symlink())
                {
                    out.push(Check::new("subscription","WARN",format!("{name}: {} は既定側がリンクなのに profile はコピーです。既定側の変更が届きません",entry.label),&format!("profile 側の {} を削除して次回起動すると、リンクとして持ち込まれます。profile 専用の内容にしたい場合はこのままで構いません",entry.dest)));
                    break;
                }
                if let (Ok(a), Ok(b)) = (entry.read(&entry.source), entry.read(&dest))
                    && strip_blocks(&a) != strip_blocks(&b)
                {
                    out.push(Check::new("subscription","WARN",format!("{name}: {} は既定側と内容が違います",entry.label),&format!("意図した差分でなければ profile 側の {} を削除して次回起動で再取得。意図した差分ならこのままで構いません",entry.dest)));
                    break;
                }
            }
        }
    }
    out
}
fn strip_blocks(bytes: &[u8]) -> Vec<u8> {
    let pairs = [
        (
            "<!-- many-ai-cli:approval-rules -->",
            "<!-- /many-ai-cli:approval-rules -->",
        ),
        (
            "<!-- any-ai-cli:approval-rules -->",
            "<!-- /any-ai-cli:approval-rules -->",
        ),
        (
            "<!-- many-ai-cli:delegation -->",
            "<!-- /many-ai-cli:delegation -->",
        ),
    ];
    let alternatives = pairs
        .into_iter()
        .map(|(start, end)| format!("{}.*?{}", regex::escape(start), regex::escape(end)))
        .collect::<Vec<_>>()
        .join("|");
    regex::bytes::Regex::new(&format!("(?s-u)\\n?(?:{alternatives})\\n?"))
        .expect("fixed owned block pattern")
        .replace_all(bytes, b"".as_slice())
        .into_owned()
}
async fn git(
    owner: &Diagnostics,
    cwd: &Path,
    args: &[&str],
    cancel: &Cancellation,
) -> Option<Vec<u8>> {
    let exe = owner.deps.io.look_path("git").ok()?;
    let mut argv = vec!["-C".into(), cwd.to_string_lossy().into_owned()];
    argv.extend(args.iter().map(|s| s.to_string()));
    let out = owner
        .deps
        .io
        .command(&exe, argv, cwd, Duration::from_secs(3), cancel)
        .await
        .ok()?;
    out.success.then_some(out.bytes)
}
fn permission(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(bytes).ok()?["permission"]["*"]
        .as_str()
        .map(str::to_string)
}
fn generated(bytes: &[u8]) -> bool {
    [b"ai-cli:approval-rules".as_slice(), b"ai-cli:delegation"]
        .iter()
        .any(|needle| bytes.windows(needle.len()).any(|w| w == *needle))
}
pub(super) async fn residue(
    owner: &Diagnostics,
    cfg: &Config,
    cancel: &Cancellation,
) -> Vec<Check> {
    let Some(root) = git(
        owner,
        &owner.deps.cwd,
        &["rev-parse", "--show-toplevel"],
        cancel,
    )
    .await
    else {
        return vec![];
    };
    let root = PathBuf::from(String::from_utf8_lossy(&root).trim());
    if root.as_os_str().is_empty()
        || crate::profile::subscriptions::check_path(&owner.deps.paths, &root).is_err()
    {
        return vec![];
    }
    let mut checks = vec![];
    let lock = crate::wrapper::hooks::OPENCODE_LOCK;
    let tracked_lock = git(owner, &root, &["show", &format!(":{lock}")], cancel)
        .await
        .is_some();
    let lock_info = std::fs::metadata(root.join(lock));
    let stale = lock_info.as_ref().is_ok_and(|info| {
        let pid = read(owner, &root.join(lock))
            .ok()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v["pid"].as_u64())
            .and_then(|v| u32::try_from(v).ok());
        match pid {
            Some(pid) if pid > 0 => !owner.deps.io.pid_alive(pid),
            _ => info
                .modified()
                .ok()
                .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
                .is_some_and(|d| d > Duration::from_secs(1800)),
        }
    });
    let tracked_config = git(
        owner,
        &root,
        &[
            "show",
            &format!(":{}", crate::wrapper::hooks::OPENCODE_CONFIG),
        ],
        cancel,
    )
    .await
    .and_then(|b| permission(&b));
    let work_config = if stale && tracked_config.is_none() {
        read(owner, &root.join(crate::wrapper::hooks::OPENCODE_CONFIG))
            .ok()
            .and_then(|b| permission(&b))
    } else {
        None
    };
    if let Some(value) = tracked_config {
        checks.push(Check::new("residue",if value=="allow"{"FAIL"}else{"WARN"},if value=="allow"{format!("opencode.json が git に登録されており、承認を全許可にする設定（permission \"*\": {value:?}）が含まれています。clone した利用者の環境でも承認プロンプトが出なくなります")}else{format!("opencode.json が git に登録されており、many-ai-cli が書く設定（permission \"*\": {value:?}）が含まれています。後始末を通らずに終了したセッションの置き去りの可能性があります")},"git rm --cached opencode.json を実行し、.gitignore に opencode.json* を追加してから commit してください（.gitignore は追跡中のファイルには効かないため git rm が要ります）"));
    } else if let Some(value) = work_config {
        checks.push(Check::new("residue","WARN",format!("opencode.json が作業フォルダに置き去りになっています（permission \"*\": {value:?} / git には未登録）"),"次に opencode セッションを起動すると自動で回収されます。commit しないよう注意してください"))
    }
    if tracked_lock {
        checks.push(Check::new("residue","WARN",format!("many-ai-cli の排他ロックファイル {lock} が git に登録されています"),&format!("git rm --cached {lock} を実行し、.gitignore に opencode.json* を追加してから commit してください")))
    } else if stale {
        checks.push(Check::new("residue","WARN",format!("前回の opencode セッションが後始末を通らずに終了した痕跡が残っています（{lock} / git には未登録）"),"次に opencode セッションを起動すると自動で回収されます。今すぐ消す場合はこのファイルを削除してください"))
    }
    if git(owner, &root, &["show", ":AGENTS.md"], cancel)
        .await
        .is_some_and(|b| generated(&b))
    {
        checks.push(Check::new("residue","WARN","AGENTS.md に many-ai-cli の承認ルールブロックが含まれたまま git に登録されています","AGENTS.md から many-ai-cli:approval-rules ブロック（旧名 any-ai-cli 版を含む）を削除して commit してください"))
    } else if std::net::TcpListener::bind(format!("127.0.0.1:{}", cfg.hub.port)).is_ok()
        && read(owner, &root.join("AGENTS.md")).is_ok_and(|b| generated(&b))
    {
        checks.push(Check::new(
            "residue",
            "WARN",
            "AGENTS.md に many-ai-cli の承認ルールブロックが残っています（git には未登録）",
            "Hub を起動すると自動で回収されます。commit しないよう注意してください",
        ))
    }
    checks.extend(relays(owner, &root, cancel).await);
    checks
}
async fn relays(owner: &Diagnostics, root: &Path, cancel: &Cancellation) -> Vec<Check> {
    let mut rows = vec![];
    let mut seen = BTreeSet::new();
    if let Some(out) = git(owner, root, &["worktree", "list", "--porcelain"], cancel).await {
        for block in String::from_utf8_lossy(&out).split("\n\n") {
            let mut path = None;
            let mut branch = None;
            for line in block.lines() {
                if let Some(v) = line.strip_prefix("worktree ") {
                    path = Some(PathBuf::from(v.trim()))
                }
                if let Some(v) = line.strip_prefix("branch refs/heads/many-ai-cli/relay/") {
                    branch = Some(v.trim().to_owned())
                }
            }
            if let (Some(path), Some(id)) = (path, branch) {
                let live = read(
                    owner,
                    &owner
                        .deps
                        .paths
                        .resource(Resource::Orchestration)
                        .join(&id)
                        .join("relay.json"),
                )
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v["state"].as_str().map(str::to_owned))
                .is_some_and(|s| !s.is_empty() && s != "completed" && s != "stopped");
                if !live {
                    seen.insert(path.clone());
                    rows.push(Check::new("residue","WARN",format!("relay の作業ツリーが残っています: {}（ブランチ many-ai-cli/relay/{id}。relay は進行中ではありません）",path.display()),&format!("ブランチを取り込んだら `git -C {} worktree remove {}` で片付けてください（未コミットの変更があると拒否されます。捨ててよければ --force）。ブランチ many-ai-cli/relay/{id} は残るので、不要なら `git branch -D many-ai-cli/relay/{id}`",root.display(),path.display())))
                }
            }
        }
    }
    if let Ok(worktrees) = Dir::open(root)
        .and_then(|dir| dir.child_dir(".many-ai-cli", false))
        .and_then(|dir| dir.child_dir("worktrees", false))
        && let Ok(entries) = worktrees.entries()
    {
        for name in entries {
            let Ok(relay) = worktrees
                .child_dir(&name, false)
                .and_then(|dir| dir.child_dir("relay", false))
            else {
                continue;
            };
            let path = root
                .join(".many-ai-cli/worktrees")
                .join(&name)
                .join("relay");
            if !seen.contains(&path) && relay.entries().is_ok_and(|entries| entries.is_empty()) {
                rows.push(Check::new("residue","WARN",format!("relay の未登録の空ディレクトリが残っています: {}（途中で片付けに失敗した痕跡）",path.display()),&format!("relay が停止済みであることを確認してから、この空ディレクトリを削除してください: {}",path.display())))
            }
        }
    }

    rows
}
