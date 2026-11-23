use std::fs;
use std::path::{Path, PathBuf};

use crate::database_file_format::save;
use crate::database_index_blocks::index_blocks;
use crate::database_index_files::index_file;
use crate::database_index_links::{broken_links, index_links};
use crate::database_index_layouts::index_layouts;
use crate::database_index_names::{duplicate_names, index_names};
use crate::database_index_tags::index_tags;
use crate::database_index_todos::index_todos;
use crate::database_store::Store;

fn skipped_folder(name: &str) -> bool {
    name == "target" || name == "compiled" || name == ".runfile" || name == ".git"
}

fn collect_run_files(folder: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(folder)
        .map_err(|e| format!("cannot read folder {}: {}", folder.display(), e))?;
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => String::new(),
        };
        if path.is_dir() {
            if !skipped_folder(&name) {
                collect_run_files(&path, out)?;
            }
        } else if name.ends_with(".run") {
            out.push(path);
        }
    }
    Ok(())
}

fn source_name(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    parts.join("/")
}

pub fn index_one(root: &Path, path: &Path, store: &mut Store) -> Result<(), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    if !name.ends_with(".run") {
        return Ok(());
    }
    let source = source_name(root, path);
    let text = fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    index_file(store, &source, &text)?;
    index_blocks(store, &source, &text)?;
    index_tags(store, &source, &text)?;
    index_todos(store, &source, &text)?;
    index_links(store, &source, &text)?;
    index_layouts(store, &source, &text)?;
    index_names(store, &source, &text)?;
    let database = root.join(".runfile").join("index.db");
    save(store, &database).map_err(|e| format!("cannot save {}: {}", database.display(), e))
}

pub fn index_folder(root: &Path) -> Result<String, String> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_run_files(root, &mut files)?;
    if let Some(first) = files.first() {
        if let Some(found) = crate::block_registry::load_near(first.as_path()) {
            eprintln!("runfile: block types from {}", found);
        }
    }
    let mut store = Store::new();
    let mut unreadable = 0usize;
    let mut indexed = 0usize;
    for path in &files {
        let source = source_name(root, path);
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        index_file(&mut store, &source, &text)?;
        index_blocks(&mut store, &source, &text)?;
        index_tags(&mut store, &source, &text)?;
        index_todos(&mut store, &source, &text)?;
        index_links(&mut store, &source, &text)?;
        index_layouts(&mut store, &source, &text)?;
        index_names(&mut store, &source, &text)?;
        indexed += 1;
    }
    for (name, places) in duplicate_names(&store) {
        eprintln!("runfile: duplicate name {} at {}", name, places.join(", "));
    }
    let broken = broken_links(&store);
    for (place, target) in &broken {
        let shown = if target.is_empty() { "<nothing>" } else { target.as_str() };
        eprintln!("runfile: broken link to {} at {}", shown, place);
    }
    let dups = duplicate_names(&store).len();
    let database = root.join(".runfile").join("index.db");
    save(&store, &database).map_err(|e| format!("cannot save {}: {}", database.display(), e))?;
    Ok(format!(
        "indexed {} files, {} records, {} unreadable, {} duplicate names, {} broken links, saved {}",
        indexed,
        store.len(),
        unreadable,
        dups,
        broken.len(),
        database.display()
    ))
}

