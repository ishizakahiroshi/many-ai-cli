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
/// Whether others than the owner can reach the object: the Unix mode bits, or
/// on Windows the DACL read through `open`, which only Windows calls (U53).
/// A test's injected "windows" platform skips the mode bits elsewhere.
fn wide_permissions(
    metadata: &std::fs::Metadata,
    platform: &str,
    open: impl FnOnce() -> io::Result<std::fs::File>,
) -> io::Result<bool> {
    #[cfg(windows)]
    {
        let _ = (metadata, platform);
        crate::config::private_io::PrivateSecurity::grants_others(&open()?)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = open;
        Ok(platform != "windows" && metadata.permissions().mode() & 0o077 != 0)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (metadata, platform, open);
        Ok(false)
    }
}
/// The configuration directory opened for its security information only.
fn open_security(path: &Path) -> io::Result<std::fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, READ_CONTROL,
        };
        std::fs::OpenOptions::new()
            .access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(path)
    }
}
const WINDOWS_WIDE: &str =
    "本人・SYSTEM・Administrators 以外への許可か、親フォルダーからの権限の引き継ぎがあります";
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
    let wide = file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("configuration basename missing"))
        .and_then(|n| {
            let dir = Dir::open(file.parent().unwrap_or(Path::new("")))?;
            let metadata = dir.metadata(n)?;
            wide_permissions(&metadata, &owner.deps.platform, || dir.open_file(n, false))
        });
    let Ok(wide) = wide else {
        return Check::new(
            "token",
            "WARN",
            "Hub トークンは設定済みですが、設定ファイルの権限を確認できません",
            "~/.many-ai-cli/config.yaml の読み取り権限を確認してください",
        );
    };
    if wide {
        return if owner.deps.platform == "windows" {
            Check::new(
                "token",
                "WARN",
                format!(
                    "Hub トークンは設定済みですが、設定ファイルの権限が広すぎます（{WINDOWS_WIDE}）"
                ),
                "エクスプローラーで ~/.many-ai-cli/config.yaml のプロパティの［セキュリティ］を開き、本人・SYSTEM・Administrators 以外の許可を外してください（意図して共有している場合はそのままで構いません）",
            )
        } else {
            Check::new(
                "token",
                "WARN",
                "Hub トークンは設定済みですが、設定ファイルの権限が広すぎます",
                "chmod 600 ~/.many-ai-cli/config.yaml を実行してください",
            )
        };
    }
    Check::new(
        "token",
        "OK",
        "Hub トークンと設定ファイル権限を確認しました（値は表示しません）",
        "",
    )
}
pub(super) fn acl(owner: &Diagnostics) -> Check {
    let root = owner.deps.paths.root();
    let wide = Dir::open(root)
        .and_then(|d| d.own_metadata())
        .and_then(|m| wide_permissions(&m, &owner.deps.platform, || open_security(root)));
    match wide {
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
        Ok(true) if owner.deps.platform == "windows" => Check::new(
            "ACL",
            "WARN",
            format!("設定ディレクトリの権限が広すぎます（{WINDOWS_WIDE}）"),
            "many-ai-cli を一度起動すると、本人・SYSTEM・Administrators だけの権限に直します。直らない場合は、~/.many-ai-cli のプロパティの［セキュリティ］を確認してください",
        ),
        Ok(true) => Check::new(
            "ACL",
            "WARN",
            "設定ディレクトリの権限が広すぎます",
            "chmod 700 ~/.many-ai-cli を実行してください",
        ),
        Ok(false) => Check::new("ACL", "OK", "設定ディレクトリへアクセスできます", ""),
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
    match walk(&dir, LOG_WALK_DEPTH) {
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
/// Subdirectory levels below the log directory that the size walk descends.
const LOG_WALK_DEPTH: usize = 8;
/// Count the regular files under `dir` and their size (U52). The entry type is
/// read without following links: links and other special entries are not
/// counted, an entry that disappears mid-walk (log cleanup) is skipped, and a
/// directory that cannot be opened fails the walk, as the Go doctor did.
fn walk(dir: &Dir, depth: usize) -> io::Result<(usize, u64)> {
    let (mut count, mut size) = (0, 0);
    for name in dir.entries()? {
        let kind = match std::fs::symlink_metadata(dir.path().join(&name)) {
            Ok(m) => m.file_type(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if depth == 0 {
                return Err(io::Error::other("log directory nests too deeply"));
            }
            let child = match dir.child_dir(&name, false) {
                Ok(d) => d,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let (c, s) = walk(&child, depth - 1)?;
            count += c;
            size += s;
        } else if kind.is_file()
            && let Ok(info) = dir.metadata(&name)
        {
            count += 1;
            size += info.len();
        }
    }
    Ok((count, size))
}
/// The session-log setting does not reach the logs each AI CLI keeps for itself.
const AI_CLI_LOGS_NOTE: &str =
    "AI CLI（Claude・Codex など）が自分で持つ生ログは、この設定とは別に残ります";
pub(super) fn session_log(owner: &Diagnostics, cfg: &Config) -> Check {
    if !cfg.log.session_enabled {
        return Check::new(
            "session log",
            "OK",
            "セッションログは無効です（log.session_enabled: false）",
            AI_CLI_LOGS_NOTE,
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
    Check::new("session log", "OK", message, AI_CLI_LOGS_NOTE)
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
/// Leftovers of the retired Whisper voice engine: the folder an older
/// many-ai-cli wrote under `~/.many-ai-cli/whisper/` (server, models, bin, tmp)
/// and a `whisper-server` it left running. Read-only: nothing is deleted or
/// ended here; the fix text says how. Links are reported but never followed.
pub(super) fn whisper_leftover(owner: &Diagnostics) -> Vec<Check> {
    let root = owner.deps.paths.root();
    let path = root.join("whisper");
    let name = "residue";
    let info = match std::fs::symlink_metadata(&path) {
        Ok(info) => info,
        Err(_) => return vec![],
    };
    let mut checks = vec![];
    if info.file_type().is_symlink() || !info.is_dir() {
        checks.push(Check::new(
            name,
            "WARN",
            format!(
                "廃止した Whisper の置き場所 {} がリンクまたはファイルとして残っています（中身は調べていません）",
                path.display()
            ),
            "リンク先は消えません。不要ならこのリンクだけを削除してください",
        ));
        return checks;
    }
    let size = Dir::open(&path)
        .and_then(|dir| walk(&dir, LOG_WALK_DEPTH))
        .map(|(count, size)| format!("{count} files, {}", human_bytes(size)))
        .unwrap_or_else(|_| "容量を確認できません".into());
    checks.push(Check::new(
        name,
        "WARN",
        format!(
            "廃止した Whisper の残りがあります: {}（{size}。旧版が取得したサーバー・モデルなど）",
            path.display()
        ),
        &format!(
            "もう使われません。Hub を止めてから、このフォルダを削除してください: {}（many-ai-cli uninstall でも消えます）",
            path.display()
        ),
    ));
    if let Ok(pids) = crate::process::image_under::processes_under(&path)
        && !pids.is_empty()
    {
        let list = pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        checks.push(Check::new(
            name,
            "WARN",
            format!(
                "旧版が起動した Whisper のサーバーが動いたままです（PID {list}。{} 配下の実行ファイル）",
                path.display()
            ),
            "many-ai-cli は終了させません。タスクマネージャーなどで上の PID を終了してから、フォルダを削除してください",
        ));
    }
    checks
}
