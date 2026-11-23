use crate::buffer::Buffer;
use crate::database_record::Record;
use crate::database_store::Store;
use crate::parser::{parse, Node};

const KIND: &str = "tag";

fn collect_title_tags(
    nodes: &[Node],
    buffer: &Buffer,
    lines: &[&str],
    source: &str,
    out: &mut Vec<Record>,
) {
    for node in nodes {
        if let Node::Block(block) = node {
            if block.name == "title" {
                let end_line = buffer.line_at(block.span.end);
                let mut number = block.line + 1;
                while number < end_line {
                    if let Some(line) = lines.get(number - 1) {
                        if let Some(words) = line.trim_start().strip_prefix("tags:") {
                            let spaced = words.replace(",", " ");
                            for tag in spaced.split_whitespace() {
                                let key = format!("{}:{}", source, tag);
                                let known = out.iter().any(|r| r.key == key);
                                if !known {
                                    out.push(
                                        Record::new(KIND, &key, source)
                                            .with_field("tag", tag)
                                            .with_field("line", &number.to_string()),
                                    );
                                }
                            }
                        }
                    }
                    number += 1;
                }
            }
            collect_title_tags(&block.children, buffer, lines, source, out);
        }
    }
}

pub fn tag_records(source: &str, text: &str) -> Vec<Record> {
    let buffer = Buffer::from_text(text);
    let tree = parse(&buffer);
    let lines: Vec<&str> = text.split("\n").collect();
    let mut out: Vec<Record> = Vec::new();
    collect_title_tags(&tree.nodes, &buffer, &lines, source, &mut out);
    out
}

pub fn index_tags(store: &mut Store, source: &str, text: &str) -> Result<(), String> {
    store.replace_source(source, KIND, tag_records(source, text))
}

pub fn forget_tags(store: &mut Store, source: &str) -> Result<(), String> {
    store.replace_source(source, KIND, Vec::new())
}

