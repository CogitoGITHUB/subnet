use std::collections::BTreeMap;

use crate::database_record::Record;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Store {
    records: BTreeMap<(String, String), Record>,
}

impl Store {
    pub fn new() -> Store {
        Store::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn put(&mut self, record: Record) -> Option<Record> {
        self.records
            .insert((record.kind.clone(), record.key.clone()), record)
    }

    pub fn get(&self, kind: &str, key: &str) -> Option<&Record> {
        self.records.get(&(kind.to_string(), key.to_string()))
    }

    pub fn remove(&mut self, kind: &str, key: &str) -> Option<Record> {
        self.records.remove(&(kind.to_string(), key.to_string()))
    }

    pub fn all(&self) -> Vec<&Record> {
        self.records.values().collect()
    }

    pub fn of_kind(&self, kind: &str) -> Vec<&Record> {
        self.records.values().filter(|r| r.kind == kind).collect()
    }

    pub fn from_source(&self, source: &str) -> Vec<&Record> {
        self.records
            .values()
            .filter(|r| r.source == source)
            .collect()
    }

    pub fn drop_source(&mut self, source: &str) -> usize {
        let before = self.records.len();
        self.records.retain(|_, r| r.source != source);
        before - self.records.len()
    }

    pub fn replace_source(
        &mut self,
        source: &str,
        kind: &str,
        records: Vec<Record>,
    ) -> Result<(), String> {
        for r in &records {
            if r.source != source || r.kind != kind {
                return Err(format!(
                    "record {} {} of {} does not belong to kind {} of {}",
                    r.kind, r.key, r.source, kind, source
                ));
            }
        }
        self.records
            .retain(|_, r| !(r.source == source && r.kind == kind));
        for r in records {
            self.put(r);
        }
        Ok(())
    }
}

