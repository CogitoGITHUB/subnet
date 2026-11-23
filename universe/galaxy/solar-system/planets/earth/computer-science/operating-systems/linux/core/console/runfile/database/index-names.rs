use std::collections::BTreeMap;

use crate::buffer::Buffer;
use crate::database_record::Record;
use crate::database_store::Store;
use crate::parser::{parse, Node};

const KIND: &str = "name";

fn collect_names(nodes: &[Node], source: &str, out: &mut Vec<Record>) {
    for node in nodes {
        if let Node::Block(block) = node {
            for declaration in &block.declarations {
                if declaration.keyword == "name" {
                    let value = declaration.value.trim();
                    if !value.is_empty() {
                        let key = format!("{}:{}", source, block.line);
                        out.push(
                            Record::new(KIND, &key, source)
                                .with_field("name", value)
                                .with_field("block", &block.name)
                                .with_field("line", &block.line.to_string()),
                        );
                    }
                    break;
                }
            }
            collect_names(&block.children, source, out);
        }
    }
}

pub fn name_records(source: &str, text: &str) -> Vec<Record> {
    let buffer = Buffer::from_text(text);
    let tree = parse(&buffer);
    let mut out: Vec<Record> = Vec::new();
    collect_names(&tree.nodes, source, &mut out);
    out
}

pub fn index_names(store: &mut Store, source: &str, text: &str) -> Result<(), String> {
    store.replace_source(source, KIND, name_records(source, text))
}

pub fn forget_names(store: &mut Store, source: &str) -> Result<(), String> {
    store.replace_source(source, KIND, Vec::new())
}

pub fn duplicate_names(store: &Store) -> Vec<(String, Vec<String>)> {
    let mut places: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for record in store.of_kind(KIND) {
        let name = record.field("name").unwrap_or("").to_string();
        places.entry(name).or_default().push(record.key.clone());
    }
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (name, mut where_found) in places {
        if where_found.len() > 1 {
            where_found.sort();
            out.push((name, where_found));
        }
    }
    out
}

