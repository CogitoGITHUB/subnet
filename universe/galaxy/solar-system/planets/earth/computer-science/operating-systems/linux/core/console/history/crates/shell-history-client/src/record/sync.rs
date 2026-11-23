// do a sync :O
use std::{cmp::Ordering, fmt::Write};

use eyre::Result;
use thiserror::Error;

use super::{encryption::PASETO_V4, store::Store};
use crate::{api_client::Client, settings::Settings};

use shell_history_common::record::{Diff, HostId, RecordId, RecordIdx, RecordStatus};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};

#[derive(Error, Debug)]
pub enum SyncError {
    #[error("the local store is ahead of the remote, but for another host. has remote lost data?")]
    LocalAheadOtherHost,

    #[error("an issue with the local database occurred: {msg:?}")]
    LocalStoreError { msg: String },

    #[error("something has gone wrong with the sync logic: {msg:?}")]
    SyncLogicError { msg: String },

    #[error("operational error: {msg:?}")]
    OperationalError { msg: String },

    #[error("a request to the sync server failed: {msg:?}")]
    RemoteRequestError { msg: String },

    #[error(
        "the encryption key on this machine does not match the data on the server. \
         this usually means a new machine was set up without copying the existing key. \
         to fix: run `shell_history key` on a machine that already syncs correctly, then run \
         `shell_history store rekey <key>` on this machine with the value from the other machine"
    )]
    WrongKey,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Operation {
    // Either upload or download until the states matches the below
    Upload {
        local: RecordIdx,
        remote: Option<RecordIdx>,
        host: HostId,
        tag: String,
    },
    Download {
        local: Option<RecordIdx>,
        remote: RecordIdx,
        host: HostId,
        tag: String,
    },
    Noop {
        host: HostId,
        tag: String,
    },
}

pub async fn build_client(settings: &Settings) -> Result<Client<'_>, SyncError> {
    Client::new(
        &settings.sync_address,
        settings
            .sync_auth_token()
            .await
            .map_err(|e| SyncError::RemoteRequestError { msg: e.to_string() })?,
        settings.network_connect_timeout,
        settings.network_timeout,
    )
    .map_err(|e| SyncError::OperationalError { msg: e.to_string() })
}

pub async fn diff(
    client: &Client<'_>,
    store: &impl Store,
) -> Result<(Vec<Diff>, RecordStatus), SyncError> {
    let local_index = store
        .status()
        .await
        .map_err(|e| SyncError::LocalStoreError { msg: e.to_string() })?;

    let remote_index = client
        .record_status()
        .await
        .map_err(|e| SyncError::RemoteRequestError { msg: e.to_string() })?;

    let diff = local_index.diff(&remote_index);

    Ok((diff, remote_index))
}

// Take a diff, along with a local store, and resolve it into a set of operations.
// With the store as context, we can determine if a tail exists locally or not and therefore if it needs uploading or download.
// In theory this could be done as a part of the diffing stage, but it's easier to reason
// about and test this way
pub async fn operations(
    diffs: Vec<Diff>,
    _store: &impl Store,
) -> Result<Vec<Operation>, SyncError> {
    let mut operations = Vec::with_capacity(diffs.len());

    for diff in diffs {
        let op = match (diff.local, diff.remote) {
            // We both have it! Could be either. Compare.
            (Some(local), Some(remote)) => match local.cmp(&remote) {
                Ordering::Equal => Operation::Noop {
                    host: diff.host,
                    tag: diff.tag,
                },
                Ordering::Greater => Operation::Upload {
                    local,
                    remote: Some(remote),
                    host: diff.host,
                    tag: diff.tag,
                },
                Ordering::Less => Operation::Download {
                    local: Some(local),
                    remote,
                    host: diff.host,
                    tag: diff.tag,
                },
            },

            // Remote has it, we don't. Gotta be download
            (None, Some(remote)) => Operation::Download {
                local: None,
                remote,
                host: diff.host,
                tag: diff.tag,
            },

            // We have it, remote doesn't. Gotta be upload.
            (Some(local), None) => Operation::Upload {
                local,
                remote: None,
                host: diff.host,
                tag: diff.tag,
            },

            // something is pretty fucked.
            (None, None) => {
                return Err(SyncError::SyncLogicError {
                    msg: String::from(
                        "diff has nothing for local or remote - (host, tag) does not exist",
                    ),
                });
            }
        };

        operations.push(op);
    }

    // sort them - purely so we have a stable testing order, and can rely on
    // same input = same output
    // We can sort by ID so long as we continue to use UUIDv7 or something
    // with the same properties

    operations.sort_by_key(|op| match op {
        Operation::Noop { host, tag } => (0, *host, tag.clone()),

        Operation::Upload { host, tag, .. } => (1, *host, tag.clone()),

        Operation::Download { host, tag, .. } => (2, *host, tag.clone()),
    });

    Ok(operations)
}

async fn sync_upload(
    store: &impl Store,
    client: &Client<'_>,
    host: HostId,
    tag: String,
    local: RecordIdx,
    remote: Option<RecordIdx>,
    page_size: u64,
) -> Result<i64, SyncError> {
    let remote = remote.unwrap_or(0);
    let expected = local - remote;
    let mut progress = 0;

    let pb = ProgressBar::new(expected);
    pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {human_pos}/{human_len} ({eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-"));

    println!(
        "Uploading {} records to {}/{}",
        expected,
        host.0.as_simple(),
        tag
    );

    loop {
        let page = store
            .next(host, tag.as_str(), remote + progress, page_size)
            .await
            .map_err(|e| {
                error!("failed to read upload page: {e:?}");

                SyncError::LocalStoreError { msg: e.to_string() }
            })?;

        if page.is_empty() {
            break;
        }

        client.post_records(&page).await.map_err(|e| {
            error!("failed to post records: {e:?}");

            SyncError::RemoteRequestError { msg: e.to_string() }
        })?;

        progress += page.len() as u64;
        pb.set_position(progress);

        if progress >= expected {
            break;
        }
    }

    pb.finish_with_message("Uploaded records");

    Ok(progress as i64)
}

async fn sync_download(
    store: &impl Store,
    client: &Client<'_>,
    host: HostId,
    tag: String,
    local: Option<RecordIdx>,
    remote: RecordIdx,
    page_size: u64,
) -> Result<Vec<RecordId>, SyncError> {
    let local = local.unwrap_or(0);
    let expected = remote - local;
    let mut progress = 0;
    let mut ret = Vec::new();

    println!(
        "Downloading {} records from {}/{}",
        expected,
        host.0.as_simple(),
        tag
    );

    let pb = ProgressBar::new(expected);
    pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {human_pos}/{human_len} ({eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-"));

    loop {
        let page = client
            .next_records(host, tag.clone(), local + progress, page_size)
            .await
            .map_err(|e| SyncError::RemoteRequestError { msg: e.to_string() })?;

        if page.is_empty() {
            break;
        }

        store
            .push_batch(page.iter())
            .await
            .map_err(|e| SyncError::LocalStoreError { msg: e.to_string() })?;

        ret.extend(page.iter().map(|f| f.id));

        progress += page.len() as u64;
        pb.set_position(progress);

        if progress >= expected {
            break;
        }
    }

    pb.finish_with_message("Downloaded records");

    Ok(ret)
}

pub async fn sync_remote(
    client: &Client<'_>,
    operations: Vec<Operation>,
    local_store: &impl Store,
    page_size: u64,
) -> Result<(i64, Vec<RecordId>), SyncError> {
    let mut uploaded = 0;
    let mut downloaded = Vec::new();

    // this can totally run in parallel, but lets get it working first
    for i in operations {
        match i {
            Operation::Upload {
                host,
                tag,
                local,
                remote,
            } => {
                uploaded +=
                    sync_upload(local_store, client, host, tag, local, remote, page_size).await?
            }

            Operation::Download {
                host,
                tag,
                local,
                remote,
            } => {
                let mut d =
                    sync_download(local_store, client, host, tag, local, remote, page_size).await?;
                downloaded.append(&mut d)
            }

            Operation::Noop { .. } => continue,
        }
    }

    Ok((uploaded, downloaded))
}

pub async fn check_encryption_key(
    client: &Client<'_>,
    remote_index: &RecordStatus,
    encryption_key: &[u8; 32],
) -> Result<(), SyncError> {
    let sample = remote_index
        .hosts
        .iter()
        .flat_map(|(host, tags)| tags.keys().map(move |tag| (*host, tag.clone())))
        .next();

    let Some((host, tag)) = sample else {
        return Ok(());
    };

    let records = client
        .next_records(host, tag, 0, 1)
        .await
        .map_err(|e| SyncError::RemoteRequestError { msg: e.to_string() })?;

    let Some(record) = records.into_iter().next() else {
        return Ok(());
    };

    record
        .decrypt::<PASETO_V4>(encryption_key)
        .map_err(|_| SyncError::WrongKey)?;

    Ok(())
}

pub async fn sync(
    settings: &Settings,
    store: &impl Store,
    encryption_key: &[u8; 32],
) -> Result<(i64, Vec<RecordId>), SyncError> {
    let client = build_client(settings).await?;
    let (diff, remote_index) = diff(&client, store).await?;

    // Bail before mutating either side if the local key can't read the remote.
    check_encryption_key(&client, &remote_index, encryption_key).await?;

    let operations = operations(diff, store).await?;
    let (uploaded, downloaded) = sync_remote(&client, operations, store, 100).await?;

    Ok((uploaded, downloaded))
}

