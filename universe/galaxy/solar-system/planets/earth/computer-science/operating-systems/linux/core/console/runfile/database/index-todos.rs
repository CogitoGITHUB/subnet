use crate::buffer::Buffer;
use crate::database_record::Record;
use crate::database_store::Store;
use crate::parser::{parse, Node};

const KIND: &str = "todo";

fn collect_opaque_lines(nodes: &[Node], buffer: &Buffer, out: &mut Vec<(usize, usize)>) {
    for node in nodes {
        if let Node::Block(block) = node {
            if block.is_opaque() {
                let end_line = buffer.line_at(block.span.end);
                if end_line > block.line + 1 {
                    out.push((block.line + 1, end_line - 1));
                }
            }
            collect_opaque_lines(&block.children, buffer, out);
        }
    }
}

pub fn todo_records(source: &str, text: &str) -> Vec<Record> {
    let buffer = Buffer::from_text(text);
    let tree = parse(&buffer);
    let mut opaque: Vec<(usize, usize)> = Vec::new();
    collect_opaque_lines(&tree.nodes, &buffer, &mut opaque);
    let mut out: Vec<Record> = Vec::new();
    for (index, line) in text.split("\n").enumerate() {
        let number = index + 1;
        let hidden = opaque.iter().any(|(first, last)| number >= *first && number <= *last);
        if hidden {
            continue;
        }
        if let Some(words) = line.trim_start().strip_prefix("todo:") {
            let key = format!("{}:{}", source, number);
            out.push(
                Record::new(KIND, &key, source)
                    .with_field("line", &number.to_string())
                    .with_field("text", words.trim()),
            );
        }
    }
    out
}

pub fn index_todos(store: &mut Store, source: &str, text: &str) -> Result<(), String> {
    store.replace_source(source, KIND, todo_records(source, text))
}

pub fn forget_todos(store: &mut Store, source: &str) -> Result<(), String> {
    store.replace_source(source, KIND, Vec::new())
}

