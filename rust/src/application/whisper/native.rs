//! Address-pinned downloads and contained, retained native server process.
use super::*;
use crate::{
    hub::network::{self, Policy},
    process::{ExitOutcome, ManagedProcess, SpawnOptions},
};
use sha2::{Digest, Sha256};
use std::io::Write;
pub struct NativeWhisperIo {
    pub paths: RuntimePaths,
}
struct PendingProcess(Option<Cancellation>);
impl Drop for PendingProcess {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            cancel.cancel();
        }
    }
}
fn error(detail: &str) -> WhisperError {
    WhisperError::new(500, "whisper_install_failed", detail)
}
#[cfg(test)]
pub(crate) fn verify_hash(bytes: &[u8], expected: &str) -> bool {
    let actual = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    expected.is_empty() || actual.eq_ignore_ascii_case(expected)
}
impl WhisperIo for NativeWhisperIo {
    fn download<'a>(
        &'a self,
        spec: Download,
        directory: Arc<Dir>,
        progress: Progress,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<(), WhisperError>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(error(
                    "trial Whisper downloads require explicitly injected fixture IO",
                ));
            }
            let temporary = format!("{}.download", spec.file_name);
            crate::files::safe_fs::basename(&temporary)?;
            let _cleanup = super::archive::Cleanup::new(directory.clone(), temporary.clone());
            let mut raw = spec.url.clone();
            for redirects in 0..3 {
                let url = network::validate_url(&raw, Policy::ExternalHttps)
                    .map_err(|_| error("Whisper download URL blocked"))?;
                let host = url
                    .host_str()
                    .ok_or_else(|| error("Whisper download host missing"))?
                    .to_owned();
                let port = url.port_or_known_default().unwrap_or(443);
                let header = async {
                    let addresses = tokio::net::lookup_host((host.as_str(), port))
                        .await?
                        .map(|value| value.ip())
                        .collect::<Vec<_>>();
                    let target = network::validated_target(url, &addresses, Policy::ExternalHttps)
                        .map_err(io::Error::other)?;
                    let client = reqwest::Client::builder()
                        .no_proxy()
                        .redirect(reqwest::redirect::Policy::none())
                        .connect_timeout(Duration::from_secs(30))
                        .resolve_to_addrs(&target.tls_host, &target.addresses)
                        .build()
                        .map_err(io::Error::other)?;
                    let response = client
                        .get(target.url.clone())
                        .header(reqwest::header::USER_AGENT, "many-ai-cli whisper installer")
                        .send()
                        .await
                        .map_err(io::Error::other)?;
                    Ok::<_, io::Error>((response, target.url))
                };
                let (mut response, url) = tokio::select! {_=cancel.cancelled()=>return Err(error("Whisper download cancelled")),value=tokio::time::timeout(Duration::from_secs(60),header)=>value.map_err(|_|error("Whisper download header deadline exceeded"))?.map_err(|_|error("Whisper download request failed"))?};
                if response.status().is_redirection() {
                    if redirects == 2 {
                        return Err(error("Whisper download redirect limit exceeded"));
                    }
                    let location = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|value| value.to_str().ok())
                        .ok_or_else(|| error("Whisper download redirect missing"))?;
                    raw = url
                        .join(location)
                        .map_err(|_| error("Whisper download redirect invalid"))?
                        .into();
                    continue;
                }
                if !response.status().is_success() {
                    return Err(error("Whisper download HTTP failure"));
                }
                let total = response.content_length();
                let mut output = directory.open_write_or_create(&temporary, 0o600)?;
                output.set_len(0)?;
                let mut hash = Sha256::new();
                let mut count = 0u64;
                progress(0, total);
                loop {
                    let chunk = tokio::select! {_=cancel.cancelled()=>return Err(error("Whisper download cancelled")),value=tokio::time::timeout(Duration::from_secs(90),response.chunk())=>value.map_err(|_|error("Whisper download stalled"))?.map_err(|_|error("Whisper download body failed"))?};
                    let Some(chunk) = chunk else { break };
                    count = count
                        .checked_add(chunk.len() as u64)
                        .filter(|count| *count <= spec.maximum_bytes)
                        .ok_or_else(|| error("Whisper download size bound exceeded"))?;
                    hash.update(&chunk);
                    output.write_all(&chunk)?;
                    progress(count, total);
                }
                let actual = hash
                    .finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                if !spec.sha256.is_empty() && !actual.eq_ignore_ascii_case(&spec.sha256) {
                    return Err(error("Whisper download SHA256 mismatch"));
                }
                output.sync_all()?;
                drop(output);
                directory.rename_to(&temporary, &directory, &spec.file_name)?;
                return Ok(());
            }
            Err(error("Whisper download redirect limit exceeded"))
        })
    }
    fn room(&self, directory: &Dir, required: u64) -> Result<(), WhisperError> {
        if available(directory.path())? < required {
            return Err(error("not enough disk space for Whisper model"));
        }
        Ok(())
    }
    fn start<'a>(
        &'a self,
        plan: ProcessPlan,
        log: Arc<Dir>,
        tasks: HubTaskHandle,
    ) -> CoreFuture<'a, Result<WhisperProcess, WhisperError>> {
        Box::pin(async move {
            if self.paths.is_trial() {
                return Err(error(
                    "trial Whisper processes require explicitly injected fixture IO",
                ));
            }
            let permit = tasks
                .effect_permit()
                .map_err(|_| error("Whisper process owner unavailable"))?;
            let file = log.open_append("whisper-server.log")?;
            let (mut process, mut events) = ManagedProcess::spawn_owned_with_log_file(
                plan,
                16,
                SpawnOptions {
                    no_window: true,
                    stdin_null: true,
                    env_clear: true,
                    ..Default::default()
                },
                file,
            );
            let cancellation = Cancellation::default();
            let mut pending = PendingProcess(Some(cancellation.clone()));
            let process_cancel = cancellation.clone();
            let owner_cancel = permit.cancellation();
            let (sender, mut done) = tokio::sync::watch::channel(None);
            drop(permit.start(async move{let outcome=tokio::select!{result=process.wait()=>result,_=owner_cancel.token().cancelled()=>{process.close();process.wait().await},_=process_cancel.cancelled()=>{process.close();process.wait().await}};let failed=!matches!(outcome,Ok(crate::process::ProcessOutput{outcome:ExitOutcome::Exited{code:Some(0),..}|ExitOutcome::Cancelled,..}));let _=sender.send(Some(ProcessExit{failed}));}));
            tokio::select! {
                event=events.recv()=>if !matches!(event,Ok(crate::process::ProcessEvent::Started{..})){return Err(error("Whisper server failed to start"));},
                _=done.changed()=>return Err(error("Whisper server failed to start")),
            }
            pending.0 = None;
            Ok(WhisperProcess { cancellation, done })
        })
    }
    fn ready<'a>(
        &'a self,
        port: u16,
        cancel: &'a Cancellation,
    ) -> CoreFuture<'a, Result<(), WhisperError>> {
        Box::pin(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
            loop {
                let attempt = tokio::time::timeout(
                    Duration::from_millis(300),
                    tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
                );
                tokio::select! {_=cancel.cancelled()=>return Err(error("Whisper startup cancelled")),value=attempt=>if matches!(value,Ok(Ok(_))){return Ok(())}}
                if tokio::time::Instant::now() >= deadline {
                    return Err(error("Whisper server did not become ready"));
                }
                tokio::select! {_=cancel.cancelled()=>return Err(error("Whisper startup cancelled")),_=tokio::time::sleep(Duration::from_millis(150))=>{}}
            }
        })
    }
}
#[cfg(windows)]
fn available(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    let name = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut free = 0;
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            name.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(free)
    }
}
#[cfg(unix)]
fn available(path: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    let mut data = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(name.as_ptr(), data.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let data = unsafe { data.assume_init() };
    Ok((data.f_bavail as u64).saturating_mul(data.f_frsize as u64))
}
