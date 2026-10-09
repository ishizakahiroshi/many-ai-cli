use crate::{
    profile::store::{ProviderRegistryStore, StoreError},
    proto::provider::BUILTIN_PROVIDER_IDS,
};
use std::{io, sync::Arc};
fn error(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}
fn expected(args: &[String], allow_empty: bool) -> io::Result<&str> {
    let mut value = None;
    for pair in args.windows(2) {
        if pair[0] == "--expected-revision" {
            value = Some(pair[1].as_str());
        }
    }
    value
        .filter(|value| allow_empty || !value.is_empty())
        .ok_or_else(|| error("--expected-revision is required"))
}
pub fn provider_command(store: &Arc<ProviderRegistryStore>, args: &[String]) -> io::Result<String> {
    let command = args
        .first()
        .ok_or_else(|| error("provider <backup|reset|recover>"))?;
    let _guard = store
        .mutation_gate
        .lock()
        .map_err(|_| error("provider mutation lock unavailable"))?;
    let mut output = String::new();
    let result: Result<(), StoreError> = match command.as_str() {
        "backup" => {
            if args.len() < 3 {
                return Err(error(
                    "provider backup <list|verify|restore> <provider-id> [backup-id]",
                ));
            }
            let action = &args[1];
            let id = &args[2];
            match action.as_str() {
                "list" => {
                    for backup in store.history.list_backups(id).map_err(io::Error::other)? {
                        output.push_str(&format!(
                            "{}\t{}\t{}\t{}\n",
                            backup.revision,
                            backup.created_at,
                            backup.reason,
                            backup.content_digest
                        ));
                    }
                    Ok(())
                }
                "verify" => {
                    let backup = args
                        .get(3)
                        .ok_or_else(|| error("provider backup verify <provider-id> <backup-id>"))?;
                    let backup = store
                        .history
                        .verify_backup(id, backup)
                        .map_err(io::Error::other)?;
                    output = format!("verified\t{}\t{}\n", backup.revision, backup.content_digest);
                    Ok(())
                }
                "restore" => {
                    let backup=args.get(3).ok_or_else(||error("provider backup restore <provider-id> <backup-id> [--expected-revision <revision>]"))?;
                    let revision = store
                        .history
                        .restore_backup(id, backup, expected(&args[4..], false)?)
                        .map_err(io::Error::other)?;
                    output = format!("restored\t{}\n", revision.revision);
                    store.reload().map(|_| ())
                }
                _ => return Err(error(format!("unknown backup action: {action}"))),
            }
        }
        "reset" => {
            if args.len() < 3 || args[1] != "--distributed" {
                return Err(error(
                    "provider reset --distributed <provider-id> --expected-revision <revision>",
                ));
            }
            let id = &args[2];
            let revision = expected(&args[3..], true)?;
            if !BUILTIN_PROVIDER_IDS.contains(&id.as_str()) {
                return Err(error(format!("built-in provider {id:?} was not found")));
            }
            let revision = store
                .history
                .reset(id, revision)
                .map_err(io::Error::other)?;
            output = format!("reset\t{}\n", revision.revision);
            store.reload().map(|_| ())
        }
        "recover" => {
            let id = args
                .get(1)
                .ok_or_else(|| error("provider recover <provider-id> [revision|--list]"))?;
            if args.get(2).is_some_and(|value| value == "--list") {
                if let Some(revision) = store
                    .history
                    .last_verified_revision(id)
                    .map_err(io::Error::other)?
                {
                    output.push_str(&format!(
                        "candidate\tuser_revision\t{}\t{}\n",
                        revision.revision, revision.created_at
                    ));
                }
                output.push_str("candidate\tbase\n");
                Ok(())
            } else {
                let selected = args.get(2).map(String::as_str).unwrap_or("");
                let recovered = store
                    .history
                    .recover_head(id, selected)
                    .map_err(io::Error::other)?;
                output = format!("recovered\t{}\n", recovered.revision);
                store.reload().map(|_| ())
            }
        }
        _ => return Err(error(format!("unknown provider command: {command}"))),
    };
    result.map_err(io::Error::other)?;
    Ok(output)
}
