use crate::buffer::Buffer;
use crate::database_record::Record;
use crate::database_store::Store;
use crate::parser::{parse, Node};

const KIND: &str = "block";

fn declared_name(declarations: &[crate::parser::Declaration]) -> String {
    for declaration in declarations {
        if declaration.keyword == "name" {
            return declaration.value.trim().to_string();
        }
    }
    String::new()
}

fn collect_blocks(
    nodes: &[Node],
    buffer: &Buffer,
    source: &str,
    parent: &str,
    depth: usize,
    out: &mut Vec<Record>,
) {
    for node in nodes {
        if let Node::Block(block) = node {
            let key = format!("{}:{}", source, block.line);
            out.push(
                Record::new(KIND, &key, source)
                    .with_field("block", &block.name)
                    .with_field("line", &block.line.to_string())
                    .with_field("end-line", &buffer.line_at(block.span.end).to_string())
                    .with_field("depth", &depth.to_string())
                    .with_field("parent", parent)
                    .with_field("declared-name", &declared_name(&block.declarations)),
            );
            collect_blocks(&block.children, buffer, source, &block.name, depth + 1, out);
        }
    }
}

pub fn block_records(source: &str, text: &str) -> Vec<Record> {
    let buffer = Buffer::from_text(text);
    let tree = parse(&buffer);
    let mut out: Vec<Record> = Vec::new();
    collect_blocks(&tree.nodes, &buffer, source, "", 0, &mut out);
    out
}

pub fn index_blocks(store: &mut Store, source: &str, text: &str) -> Result<(), String> {
    store.replace_source(source, KIND, block_records(source, text))
}

pub fn forget_blocks(store: &mut Store, source: &str) -> Result<(), String> {
    store.replace_source(source, KIND, Vec::new())
}

