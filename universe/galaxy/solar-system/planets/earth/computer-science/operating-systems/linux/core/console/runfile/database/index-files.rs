use crate::buffer::Buffer;
use crate::database_record::Record;
use crate::database_store::Store;
use crate::parser::{parse, Node};

const KIND: &str = "file";

fn count_blocks(nodes: &[Node]) -> usize {
    let mut total = 0usize;
    for node in nodes {
        if let Node::Block(block) = node {
            total += 1 + count_blocks(&block.children);
        }
    }
    total
}

fn declared_kind(nodes: &[Node]) -> String {
    for node in nodes {
        if let Node::Block(block) = node {
            if block.name == "title" {
                for declaration in &block.declarations {
                    if declaration.keyword == "kind" {
                        return declaration.value.trim().to_string();
                    }
                }
            }
        }
    }
    String::new()
}

fn declared_title(buffer: &Buffer, nodes: &[Node]) -> String {
    for node in nodes {
        if let Node::Block(block) = node {
            if block.name == "title" {
                let raw = buffer.slice(block.span.start, block.span.end);
                let text = String::from_utf8_lossy(raw).to_string();
                let mut parts: Vec<String> = Vec::new();
                let all: Vec<&str> = text.lines().collect();
                if all.len() < 3 {
                    return String::new();
                }
                for line in &all[1..all.len() - 1] {
                    let t = line.trim();
                    let decl = block.declarations.iter().any(|d| t.starts_with(&format!("{}:", d.keyword)));
                    if t.ends_with(" start") { break; }
                    if !decl && !t.is_empty() {
                        parts.push(t.to_string());
                    }
                }
                return parts.join(" ");
            }
        }
    }
    String::new()
}

fn declared_caption(nodes: &[Node]) -> String {
for node in nodes {
if let Node::Block(block) = node {
if block.name == "title" {
for d in &block.declarations {
if d.keyword == "caption" {
return d.value.trim().to_string();
}
}
}
}
}
String::new()
}

pub fn file_record(source: &str, text: &str) -> Record {
    let buffer = Buffer::from_text(text);
    let tree = parse(&buffer);
    Record::new(KIND, source, source)
        .with_field("kind", &declared_kind(&tree.nodes))
        .with_field("title", &declared_title(&buffer, &tree.nodes))
        .with_field("caption", &declared_caption(&tree.nodes))
        .with_field("lines", &buffer.line_count().to_string())
        .with_field("bytes", &buffer.len().to_string())
        .with_field("blocks", &count_blocks(&tree.nodes).to_string())
        .with_field("errors", &tree.errors().len().to_string())
}

pub fn index_file(store: &mut Store, source: &str, text: &str) -> Result<(), String> {
    store.replace_source(source, KIND, vec![file_record(source, text)])
}

pub fn forget_file(store: &mut Store, source: &str) -> Result<(), String> {
    store.replace_source(source, KIND, Vec::new())
}

