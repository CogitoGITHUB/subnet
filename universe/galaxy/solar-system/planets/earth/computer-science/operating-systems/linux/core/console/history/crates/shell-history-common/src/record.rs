use std::collections::HashMap;

use eyre::Result;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, derive_more::Deref, derive_more::From)]
pub struct DecryptedData(pub Vec<u8>);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncryptedData {
    pub data: String,
    pub content_encryption_key: String,
}

#[derive(Debug, PartialEq, PartialOrd, Ord, Eq)]
pub struct Diff {
    pub host: HostId,
    pub tag: String,
    pub local: Option<RecordIdx>,
    pub remote: Option<RecordIdx>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Host {
    pub id: HostId,
    pub name: String,
}

impl Host {
    pub fn new(id: HostId) -> Self {
        Host {
            id,
            name: String::new(),
        }
    }
}

new_uuid!(RecordId);
new_uuid!(HostId);

pub type RecordIdx = u64;

/// A single record stored inside of our local database
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TypedBuilder)]
pub struct Record<Data> {
    /// a unique ID
    #[builder(default = RecordId(crate::utils::uuid_v7()))]
    pub id: RecordId,

    /// The integer record ID. This is only unique per (host, tag).
    pub idx: RecordIdx,

    /// The unique ID of the host.
    // TODO(console): Optimize the storage here. We use a bunch of IDs, and currently store
    // as strings. I would rather avoid normalization, so store as UUID binary instead of
    // encoding to a string and wasting much more storage.
    pub host: Host,

    /// The creation time in nanoseconds since unix epoch
    #[builder(default = time::OffsetDateTime::now_utc().unix_timestamp_nanos() as u64)]
    pub timestamp: u64,

    /// The version the data in the entry conforms to
    // However we want to track versions for this tag, eg v2
    pub version: String,

    /// The type of data we are storing here. Eg, "history"
    pub tag: String,

    /// Some data. This can be anything you wish to store. Use the tag field to know how to handle it.
    pub data: Data,
}

/// Extra data from the record that should be encoded in the data
#[derive(Debug, Copy, Clone)]
pub struct AdditionalData<'a> {
    pub id: &'a RecordId,
    pub idx: &'a u64,
    pub version: &'a str,
    pub tag: &'a str,
    pub host: &'a HostId,
}

impl<Data> Record<Data> {
    pub fn append(&self, data: Vec<u8>) -> Record<DecryptedData> {
        Record::builder()
            .host(self.host.clone())
            .version(self.version.clone())
            .idx(self.idx + 1)
            .tag(self.tag.clone())
            .data(DecryptedData(data))
            .build()
    }
}

/// An index representing the current state of the record stores
/// This can be both remote, or local, and compared in either direction
#[derive(Debug, Serialize, Deserialize)]
pub struct RecordStatus {
    // A map of host -> tag -> max(idx)
    pub hosts: HashMap<HostId, HashMap<String, RecordIdx>>,
}

impl Default for RecordStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl Extend<(HostId, String, RecordIdx)> for RecordStatus {
    fn extend<T: IntoIterator<Item = (HostId, String, RecordIdx)>>(&mut self, iter: T) {
        for (host, tag, tail_idx) in iter {
            self.set_raw(host, tag, tail_idx);
        }
    }
}

impl RecordStatus {
    pub fn new() -> RecordStatus {
        RecordStatus {
            hosts: HashMap::new(),
        }
    }

    /// Insert a new tail record into the store
    pub fn set(&mut self, tail: Record<DecryptedData>) {
        self.set_raw(tail.host.id, tail.tag, tail.idx)
    }

    pub fn set_raw(&mut self, host: HostId, tag: String, tail_id: RecordIdx) {
        self.hosts.entry(host).or_default().insert(tag, tail_id);
    }

    pub fn get(&self, host: HostId, tag: String) -> Option<RecordIdx> {
        self.hosts.get(&host).and_then(|v| v.get(&tag)).cloned()
    }

    /// Diff this index with another, likely remote index.
    /// The two diffs can then be reconciled, and the optimal change set calculated
    /// Returns a tuple, with (host, tag, Option(OTHER))
    /// OTHER is set to the value of the idx on the other machine. If it is greater than our index,
    /// then we need to do some downloading. If it is smaller, then we need to do some uploading
    /// Note that we cannot upload if we are not the owner of the record store - hosts can only
    /// write to their own store.
    pub fn diff(&self, other: &Self) -> Vec<Diff> {
        let mut ret = Vec::new();

        // First, we check if other has everything that self has
        for (host, tag_map) in self.hosts.iter() {
            for (tag, idx) in tag_map.iter() {
                match other.get(*host, tag.clone()) {
                    // The other store is all up to date! No diff.
                    Some(t) if t.eq(idx) => continue,

                    // The other store does exist, and it is either ahead or behind us. A diff regardless
                    Some(t) => ret.push(Diff {
                        host: *host,
                        tag: tag.clone(),
                        local: Some(*idx),
                        remote: Some(t),
                    }),

                    // The other store does not exist :O
                    None => ret.push(Diff {
                        host: *host,
                        tag: tag.clone(),
                        local: Some(*idx),
                        remote: None,
                    }),
                };
            }
        }

        // At this point, there is a single case we have not yet considered.
        // If the other store knows of a tag that we are not yet aware of, then the diff will be missed

        // account for that!
        for (host, tag_map) in other.hosts.iter() {
            for (tag, idx) in tag_map.iter() {
                match self.get(*host, tag.clone()) {
                    // If we have this host/tag combo, the comparison and diff will have already happened above
                    Some(_) => continue,

                    None => ret.push(Diff {
                        host: *host,
                        tag: tag.clone(),
                        remote: Some(*idx),
                        local: None,
                    }),
                };
            }
        }

        // Stability is a nice property to have
        ret.sort();
        ret
    }
}

pub trait Encryption {
    fn re_encrypt(
        data: EncryptedData,
        ad: AdditionalData,
        old_key: &[u8; 32],
        new_key: &[u8; 32],
    ) -> Result<EncryptedData> {
        let data = Self::decrypt(data, ad, old_key)?;
        Ok(Self::encrypt(data, ad, new_key))
    }
    fn encrypt(data: DecryptedData, ad: AdditionalData, key: &[u8; 32]) -> EncryptedData;
    fn decrypt(data: EncryptedData, ad: AdditionalData, key: &[u8; 32]) -> Result<DecryptedData>;
}

impl Record<DecryptedData> {
    pub fn encrypt<E: Encryption>(self, key: &[u8; 32]) -> Record<EncryptedData> {
        let ad = AdditionalData {
            id: &self.id,
            version: &self.version,
            tag: &self.tag,
            host: &self.host.id,
            idx: &self.idx,
        };
        Record {
            data: E::encrypt(self.data, ad, key),
            id: self.id,
            host: self.host,
            idx: self.idx,
            timestamp: self.timestamp,
            version: self.version,
            tag: self.tag,
        }
    }
}

impl Record<EncryptedData> {
    pub fn decrypt<E: Encryption>(self, key: &[u8; 32]) -> Result<Record<DecryptedData>> {
        let ad = AdditionalData {
            id: &self.id,
            version: &self.version,
            tag: &self.tag,
            host: &self.host.id,
            idx: &self.idx,
        };
        Ok(Record {
            data: E::decrypt(self.data, ad, key)?,
            id: self.id,
            host: self.host,
            idx: self.idx,
            timestamp: self.timestamp,
            version: self.version,
            tag: self.tag,
        })
    }

    pub fn re_encrypt<E: Encryption>(
        self,
        old_key: &[u8; 32],
        new_key: &[u8; 32],
    ) -> Result<Record<EncryptedData>> {
        let ad = AdditionalData {
            id: &self.id,
            version: &self.version,
            tag: &self.tag,
            host: &self.host.id,
            idx: &self.idx,
        };
        Ok(Record {
            data: E::re_encrypt(self.data, ad, old_key, new_key)?,
            id: self.id,
            host: self.host,
            idx: self.idx,
            timestamp: self.timestamp,
            version: self.version,
            tag: self.tag,
        })
    }
}

