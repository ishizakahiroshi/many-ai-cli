use super::*;
use crate::{config::Resource, files::safe_fs::Dir};
pub(super) fn port(cfg: &Config) -> Check {
    let address = format!("127.0.0.1:{}", cfg.hub.port);
    match std::net::TcpListener::bind(&address) {
        Ok(_) => Check::new("port", "OK", format!("{address} は空いています"), ""),
        Err(_) => Check::new(
            "port",
            "WARN",
            format!("{address} は使用中です（Hub が起動中の可能性があります）"),
            "many-ai-cli status で Hub の状態を確認してください",
        ),
    }
}
fn wide_permissions(metadata: &std::fs::Metadata, platform: &str) -> bool {
    if platform == "windows" {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o077 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}
pub(super) fn token(owner: &Diagnostics, cfg: &Config) -> Check {
    if cfg.token.trim().is_empty() {
        return Check::new(
            "token",
            "FAIL",
            "Hub トークンが未設定です",
            "設定を再生成するには ~/.many-ai-cli/config.yaml を安全な場所へ退避してから再起動してください",
        );
    }
    let file = owner.deps.paths.resource(Resource::Config);
    let metadata = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("configuration basename missing"))
        .and_then(|n| Dir::open(file.parent().unwrap_or(Path::new("")))?.metadata(n));
    let Ok(metadata) = metadata else {
        return Check::new(
            "token",
            "WARN",
            "Hub トークンは設定済みですが、設定ファイルを確認できません",
            "~/.many-ai-cli/config.yaml の読み取り権限を確認してください",
        );
    };
    if wide_permissions(&metadata, &owner.deps.platform) {
        return Check::new(
            "token",
            "WARN",
            "Hub トークンは設定済みですが、設定ファイルの権限が広すぎます",
            "chmod 600 ~/.many-ai-cli/config.yaml を実行してください",
        );
    }
    Check::new(
        "token",
        "OK",
        "Hub トークンと設定ファイル権限を確認しました（値は表示しません）",
        "",
    )
}
pub(super) fn acl(owner: &Diagnostics) -> Check {
    match Dir::open(owner.deps.paths.root()).and_then(|d| d.own_metadata()) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Check::new(
            "ACL",
            "WARN",
            "設定ディレクトリがまだありません",
            "many-ai-cli を一度起動して設定を作成してください",
        ),
        Err(_) => Check::new(
            "ACL",
            "WARN",
            "設定ディレクトリを確認できません",
            "ディレクトリの読み書き権限を確認してください",
        ),
        Ok(m) if wide_permissions(&m, &owner.deps.platform) => Check::new(
            "ACL",
            "WARN",
            "設定ディレクトリの権限が広すぎます",
            "chmod 700 ~/.many-ai-cli を実行してください",
        ),
        Ok(_) => Check::new("ACL", "OK", "設定ディレクトリへアクセスできます", ""),
    }
}
fn log_dir(owner: &Diagnostics, cfg: &Config) -> io::Result<PathBuf> {
    Ok(owner
        .deps
        .paths
        .clone()
        .with_log_dir(Path::new(&cfg.hub.log_dir))?
        .resource(Resource::Logs))
}
pub(super) fn logs(owner: &Diagnostics, cfg: &Config) -> Check {
    let Ok(path) = log_dir(owner, cfg) else {
        return Check::new(
            "log",
            "WARN",
            "ログディレクトリを特定できません",
            "設定を確認してください",
        );
    };
    let dir = match Dir::open(&path) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Check::new(
                "log",
                "WARN",
                "ログディレクトリは未作成です",
                "Hub を起動すると必要に応じて作成されます",
            );
        }
        Err(_) => {
            return Check::new(
                "log",
                "FAIL",
                "ログディレクトリへアクセスできません",
                "hub.log_dir の書き込み権限を確認してください",
            );
        }
    };
    match walk(&dir) {
        Ok((count, size)) => Check::new(
            "log",
            "OK",
            format!(
                "ログディレクトリへアクセスできます（{count} files, {:.1} MB）",
                size as f64 / (1024. * 1024.)
            ),
            "",
        ),
        Err(_) => Check::new(
            "log",
            "WARN",
            "ログディレクトリの容量を確認できません",
            "hub.log_dir の読み取り権限を確認してください",
        ),
    }
}
fn walk(dir: &Dir) -> io::Result<(usize, u64)> {
    let (mut count, mut size) = (0, 0);
    for name in dir.entries()? {
        let info = dir.metadata(&name)?;
        if info.is_dir() {
            let (c, s) = walk(&dir.child_dir(&name, false)?)?;
            count += c;
            size += s;
        } else {
            count += 1;
            size += info.len();
        }
    }
    Ok((count, size))
}
pub(super) fn session_log(owner: &Diagnostics, cfg: &Config) -> Check {
    if !cfg.log.session_enabled {
        return Check::new(
            "session log",
            "OK",
            "セッションログは無効です（log.session_enabled: false）",
            "",
        );
    }
    let missing = || {
        Check::new(
            "session log",
            "WARN",
            "セッションログは有効ですが、まだ 1 本も作られていません",
            "セッションを 1 本起動してから再実行してください",
        )
    };
    let Ok(path) = log_dir(owner, cfg) else {
        return Check::new(
            "session log",
            "WARN",
            "ログディレクトリを特定できません",
            "設定を確認してください",
        );
    };
    let dir = match Dir::open(&path.join("sessions")) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return missing(),
        Err(_) => {
            return Check::new(
                "session log",
                "WARN",
                "セッションログのディレクトリを読めません",
                "hub.log_dir の読み取り権限を確認してください",
            );
        }
    };
    let entries = match dir.entries() {
        Ok(v) => v,
        Err(_) => {
            return Check::new(
                "session log",
                "WARN",
                "セッションログのディレクトリを読めません",
                "hub.log_dir の読み取り権限を確認してください",
            );
        }
    };
    let latest = entries
        .into_iter()
        .filter(|n| n.ends_with(".log"))
        .filter_map(|n| {
            dir.metadata(&n)
                .ok()
                .filter(|m| m.is_file())
                .and_then(|m| m.modified().ok().map(|t| (t, n, m.len())))
        })
        .max_by(|a, b| a.0.cmp(&b.0));
    let Some((_, name, entry_size)) = latest else {
        return missing();
    };
    let size = match dir.open_file(&name, false).and_then(|f| f.metadata()) {
        Ok(m) => m.len(),
        Err(_) => {
            return Check::new(
                "session log",
                "WARN",
                format!("最新のセッションログを開けません（{name}）"),
                "hub.log_dir の読み取り権限と、他プロセスによるロックを確認してください",
            );
        }
    };
    if size == 0 {
        return Check::new(
            "session log",
            "WARN",
            format!("最新のセッションログが 0 バイトです（{name}）"),
            "Hub を再起動し、セッションでやり取りしてから再実行してください",
        );
    }
    let mut message = format!(
        "最新のセッションログに書き込まれています（{name} / {}）",
        human_bytes(size)
    );
    if size != entry_size {
        message += &format!(
            "。ディレクトリ一覧では {} に見えますが、これは書き込み中のファイルのサイズ・更新時刻が遅延反映されるためで故障ではありません",
            human_bytes(entry_size)
        )
    }
    Check::new("session log", "OK", message, "")
}
fn human_bytes(n: u64) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MB", n as f64 / (1024. * 1024.))
    } else if n >= 1024 {
        format!("{:.1} KB", n as f64 / 1024.)
    } else {
        format!("{n} B")
    }
}
pub(super) fn handoff(owner: &Diagnostics, cfg: &Config) -> Check {
    if !cfg.handoff.enabled_or_default() {
        return Check::new(
            "handoff",
            "OK",
            "引き継ぎ記録 (handoff) は無効です（handoff.enabled: false）",
            "",
        );
    }
    let dir = match Dir::open(&owner.deps.paths.resource(Resource::Handoff)) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Check::new(
                "handoff",
                "OK",
                "引き継ぎ記録 (handoff) はまだありません",
                "",
            );
        }
        Err(_) => {
            return Check::new(
                "handoff",
                "WARN",
                "handoff ディレクトリを確認できません",
                "~/.many-ai-cli/handoff の読み取り権限を確認してください",
            );
        }
    };
    let items = match dir.entries() {
        Ok(v) => v,
        Err(_) => {
            return Check::new(
                "handoff",
                "WARN",
                "handoff ディレクトリを確認できません",
                "~/.many-ai-cli/handoff の読み取り権限を確認してください",
            );
        }
    };
    let now = std::time::SystemTime::now();
    let ages = items
        .into_iter()
        .filter(|n| n.ends_with(".jsonl"))
        .filter_map(|n| {
            dir.metadata(&n)
                .ok()
                .filter(|m| m.is_file())
                .and_then(|m| m.modified().ok())
                .map(|t| now.duration_since(t).unwrap_or_default())
        })
        .collect::<Vec<_>>();
    if ages.is_empty() {
        return Check::new(
            "handoff",
            "OK",
            "引き継ぎ記録 (handoff) はまだありません",
            "",
        );
    }
    let oldest = ages.iter().copied().max().unwrap_or_default();
    let days = oldest.as_secs() / 86400;
    let age = if days == 0 {
        "1 日未満".into()
    } else {
        format!("{days} 日前")
    };
    let retention = cfg.handoff.retention_days_or_default();
    let msg = format!(
        "引き継ぎ記録 (handoff) {} 件（最古 {age} / 保持 {retention} 日）",
        ages.len()
    );
    if oldest.as_secs() > retention as u64 * 86400 {
        Check::new(
            "handoff",
            "WARN",
            msg + "。保持期限を超えたファイルがあります",
            "Hub を起動すると次回の定期処理（内部の maintenance loop）で自動削除されます",
        )
    } else {
        Check::new("handoff", "OK", msg, "")
    }
}
