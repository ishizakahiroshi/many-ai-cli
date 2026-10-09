use super::*;
use futures_util::StreamExt;
use std::{io::Read, net::IpAddr, path::Path};
pub fn validate_source(source: &str, paths: &RuntimePaths) -> io::Result<()> {
    let source = source.trim();
    if source.is_empty() {
        return Ok(());
    }
    if source.contains("://") {
        let parsed = url::Url::parse(source).map_err(|_| io::Error::other("invalid URL"))?;
        if parsed.host_str().is_none_or(str::is_empty) {
            return Err(io::Error::other("invalid URL"));
        }
        if parsed.scheme() != "https" {
            return Err(io::Error::other("URL scheme must be https"));
        }
        let host = simple_lower(parsed.host_str().unwrap())
            .trim_end_matches('.')
            .to_owned();
        if blocked_host(&host) {
            return Err(io::Error::other("URL host is not allowed"));
        }
        if host != "raw.githubusercontent.com" {
            return Err(io::Error::other(format!(
                "URL host {} is not allowed",
                crate::proto::go_quote::quote(&host)
            )));
        }
        // An empty userinfo (https://@host) is still Go URL.User != nil.
        if source
            .split_once("://")
            .unwrap()
            .1
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .contains('@')
        {
            return Err(io::Error::other("URL credentials are not allowed"));
        }
        return Ok(());
    }
    let path = Path::new(source);
    if !path.is_absolute() {
        return Err(io::Error::other("local source path must be absolute"));
    }
    under_root(path, paths.root())
}
pub(super) fn under_root(path: &Path, root: &Path) -> io::Result<()> {
    if relative_source(root, path).is_ok_and(|relative| {
        relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
    }) {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "local source path must be under {}",
            root.display()
        )))
    }
}
fn blocked_host(host: &str) -> bool {
    host.trim_matches(['[', ']'])
        .split('%')
        .next()
        .and_then(|s| s.parse::<IpAddr>().ok())
        .is_some_and(blocked_ip)
}
fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_unspecified()
                || ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_multicast()
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                blocked_ip(IpAddr::V4(ip))
            } else {
                ip.is_unspecified()
                    || ip.is_loopback()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    || ip.is_multicast()
            }
        }
    }
}
fn external_url(url: &url::Url) -> io::Result<()> {
    if url.scheme() != "https" {
        return Err(io::Error::other("non-https request blocked"));
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(io::Error::other("missing request host"));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url
            .as_str()
            .split_once("://")
            .unwrap()
            .1
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .contains('@')
    {
        return Err(io::Error::other("request credentials blocked"));
    }
    if blocked_host(url.host_str().unwrap()) {
        return Err(io::Error::other("private network host blocked"));
    }
    Ok(())
}
struct PublicDns;
impl reqwest::dns::Resolve for PublicDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((name.as_str(), 0))
                .await?
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err(io::Error::other("no ip resolved").into());
            }
            if addresses.iter().any(|addr| blocked_ip(addr.ip())) {
                return Err(io::Error::other("private network host blocked").into());
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
pub struct NativeSlashIo {
    paths: RuntimePaths,
    environment: Vec<String>,
    home: Option<PathBuf>,
    client: reqwest::Client,
}
impl NativeSlashIo {
    pub fn new(
        paths: RuntimePaths,
        environment: Vec<String>,
        home: Option<PathBuf>,
    ) -> io::Result<Self> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .dns_resolver(Arc::new(PublicDns))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 3 {
                    attempt.error("too many redirects")
                } else if let Err(error) = external_url(attempt.url()) {
                    attempt.error(error)
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            paths,
            environment,
            home,
            client,
        })
    }
}
impl SlashIo for NativeSlashIo {
    fn read<'a>(&'a self, source: &'a str) -> CoreFuture<'a, io::Result<Vec<u8>>> {
        Box::pin(async move {
            let source = source.trim();
            if source.is_empty() {
                return Err(io::Error::other("source is empty"));
            }
            validate_source(source, &self.paths)?;
            if !source.contains("://") {
                let file = held_source(&self.paths, Path::new(source))?;
                if !file.metadata()?.is_file() {
                    return Err(io::Error::other("source is not a regular file"));
                }
                let mut body = Vec::new();
                file.take(2 * 1024 * 1024 + 1).read_to_end(&mut body)?;
                if body.len() > 2 * 1024 * 1024 {
                    return Err(io::Error::other(format!(
                        "source {source} exceeds {} bytes",
                        2 * 1024 * 1024
                    )));
                }
                return Ok(body);
            }
            if !self.paths.automatic_external_actions_allowed() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "remote slash source reads are disabled in trial",
                ));
            }
            let url = url::Url::parse(source).map_err(io::Error::other)?;
            external_url(&url)?;
            let response = self
                .client
                .get(url)
                .send()
                .await
                .map_err(io::Error::other)?;
            if !response.status().is_success() {
                return Err(io::Error::other(format!(
                    "fetch {source}: {}",
                    response.status()
                )));
            }
            let mut stream = response.bytes_stream();
            let mut body = Vec::new();
            while body.len() < 2 * 1024 * 1024 {
                let Some(chunk) = stream.next().await else {
                    break;
                };
                let chunk = chunk.map_err(io::Error::other)?;
                let available = (2 * 1024 * 1024 - body.len()).min(chunk.len());
                body.extend_from_slice(&chunk[..available]);
            }
            Ok(body)
        })
    }
    fn skills(&self, provider: &str, context: &SearchContext) -> Vec<SlashCmd> {
        skills::discover(
            provider,
            context,
            &self.environment,
            self.home.as_deref(),
            &self.paths,
        )
    }
}

fn relative_source(root: &Path, path: &Path) -> io::Result<PathBuf> {
    #[cfg(windows)]
    use std::path::Component;
    let relative = path.strip_prefix(root).map(Path::to_path_buf);
    #[cfg(windows)]
    let relative = relative.or_else(|_| {
        use std::path::Prefix;
        let mut components = root.components();
        let mut ordinary = PathBuf::new();
        if let Some(Component::Prefix(prefix)) = components.next() {
            match prefix.kind() {
                Prefix::VerbatimDisk(drive) => ordinary.push(format!("{}:", char::from(drive))),
                Prefix::VerbatimUNC(server, share) => {
                    ordinary.push(Path::new(r"\\").join(server).join(share))
                }
                _ => return path.strip_prefix(root).map(Path::to_path_buf),
            }
            for component in components {
                ordinary.push(component.as_os_str());
            }
        }
        path.strip_prefix(ordinary).map(Path::to_path_buf)
    });
    relative.map_err(|_| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "source escapes selected root",
        )
    })
}

fn held_source(paths: &RuntimePaths, path: &Path) -> io::Result<std::fs::File> {
    use std::path::Component;
    let relative = relative_source(paths.root(), path)?;
    let mut directory = crate::files::safe_fs::Dir::open(paths.root())?;
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unclean source path",
            ));
        };
        let name = name
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF8 source component"))?;
        if components.peek().is_none() {
            return directory.open_file(name, false);
        }
        directory = directory.child_dir(name, false)?;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "source names selected root",
    ))
}
