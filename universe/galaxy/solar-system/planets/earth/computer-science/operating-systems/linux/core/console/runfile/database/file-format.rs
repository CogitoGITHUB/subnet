use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::database_record::Record;
use crate::database_store::Store;

const HEADER: &str = "runfile-index";
const VERSION: &str = "1";

const BACKSLASH: u8 = 92;
const LETTER_T: u8 = 116;
const LETTER_N: u8 = 110;
const LETTER_R: u8 = 114;
const TAB: u8 = 9;
const NEWLINE: u8 = 10;
const RETURN: u8 = 13;

fn escape_text(text: &str) -> String {
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            BACKSLASH => {
                out.push(BACKSLASH);
                out.push(BACKSLASH);
            }
            TAB => {
                out.push(BACKSLASH);
                out.push(LETTER_T);
            }
            NEWLINE => {
                out.push(BACKSLASH);
                out.push(LETTER_N);
            }
            RETURN => {
                out.push(BACKSLASH);
                out.push(LETTER_R);
            }
            _ => out.push(b),
        }
    }
    String::from_utf8(out).expect("escaping only adds ascii bytes")
}

fn unescape_text(text: &str) -> Result<String, String> {
    let mut out: Vec<u8> = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(b) = bytes.next() {
        if b != BACKSLASH {
            out.push(b);
            continue;
        }
        match bytes.next() {
            Some(BACKSLASH) => out.push(BACKSLASH),
            Some(LETTER_T) => out.push(TAB),
            Some(LETTER_N) => out.push(NEWLINE),
            Some(LETTER_R) => out.push(RETURN),
            _ => return Err("a backslash is followed by something that is not an escape".to_string()),
        }
    }
    String::from_utf8(out).map_err(|_| "the text is not valid utf-8".to_string())
}

fn unescape_at_line(number: usize, text: &str) -> Result<String, String> {
    unescape_text(text).map_err(|e| format!("line {}: {}", number, e))
}

pub fn encode(store: &Store) -> String {
    let mut out = format!("{}\t{}\n", HEADER, VERSION);
    for r in store.all() {
        out.push_str(&format!(
            "R\t{}\t{}\t{}\n",
            escape_text(&r.kind),
            escape_text(&r.key),
            escape_text(&r.source)
        ));
        for (name, value) in &r.fields {
            out.push_str(&format!("F\t{}\t{}\n", escape_text(name), escape_text(value)));
        }
    }
    out
}

pub fn decode(text: &str) -> Result<Store, String> {
    let mut lines = text.lines().enumerate();
    match lines.next() {
        Some((_, first)) if first == format!("{}\t{}", HEADER, VERSION) => {}
        _ => return Err("line 1: not a runfile index of version 1".to_string()),
    }
    let mut store = Store::new();
    let mut open: Option<Record> = None;
    for (index, line) in lines {
        let number = index + 1;
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split("\t").collect();
        match parts[0] {
            "R" => {
                if parts.len() != 4 {
                    return Err(format!("line {}: a record line has 4 fields", number));
                }
                if let Some(done) = open.take() {
                    store.put(done);
                }
                open = Some(Record::new(
                    &unescape_at_line(number, parts[1])?,
                    &unescape_at_line(number, parts[2])?,
                    &unescape_at_line(number, parts[3])?,
                ));
            }
            "F" => {
                if parts.len() != 3 {
                    return Err(format!("line {}: a field line has 3 fields", number));
                }
                let name = unescape_at_line(number, parts[1])?;
                let value = unescape_at_line(number, parts[2])?;
                match open.as_mut() {
                    Some(record) => record.fields.push((name, value)),
                    None => {
                        return Err(format!(
                            "line {}: a field line comes before any record line",
                            number
                        ))
                    }
                }
            }
            other => return Err(format!("line {}: unknown line type {}", number, other)),
        }
    }
    if let Some(done) = open.take() {
        store.put(done);
    }
    Ok(store)
}

pub fn save(store: &Store, path: &Path) -> io::Result<()> {
    let mut writing = path.as_os_str().to_owned();
    writing.push(".writing");
    let writing = PathBuf::from(writing);
    if let Some(folder) = path.parent() {
        if !folder.as_os_str().is_empty() {
            fs::create_dir_all(folder)?;
        }
    }
    fs::write(&writing, encode(store))?;
    fs::rename(&writing, path)
}

pub fn load(path: &Path) -> Result<Store, String> {
    match fs::read_to_string(path) {
        Ok(text) => decode(&text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Store::new()),
        Err(e) => Err(format!("cannot read {}: {}", path.display(), e)),
    }
}

