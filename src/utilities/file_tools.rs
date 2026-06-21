use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Folders/files we never want in AI context. Add to this list as needed.
const IGNORED: &[&str] = &[
    ".git",
    ".github",
    "target",
    "node_modules",
    ".DS_Store",
    "dist",
    "build",
    ".idea",
    ".vscode",
    ".gitignore",
];

/// One node from a directory walk: where it is, how deep, and whether it's a folder.
pub struct Entry {
    pub path: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
}

/// Recursively walk `base`, returning every (non-ignored) folder and file.
/// Folders come before files at each level, both sorted alphabetically.
pub fn walk_tree(base: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    walk(base, 0, &mut entries);
    entries
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<Entry>) {
    let read = match fs::read_dir(dir) {
        Ok(read) => read,
        Err(_) => return,
    };

    let mut paths: Vec<PathBuf> = read.filter_map(|e| e.ok()).map(|e| e.path()).collect();

    // Folders first, then files; alphabetical within each group.
    paths.sort_by(|a, b| {
        b.is_dir()
            .cmp(&a.is_dir())
            .then(a.file_name().cmp(&b.file_name()))
    });

    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if IGNORED.contains(&name.as_str()) {
            continue;
        }

        let is_dir = path.is_dir();
        out.push(Entry {
            path: path.clone(),
            depth,
            is_dir,
        });

        if is_dir {
            walk(&path, depth + 1, out);
        }
    }
}

/// Expand the user's selection into a flat, sorted, de-duplicated list of files.
/// A selected folder contributes every file inside it; a selected file is kept.
fn collect_files(selected: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();

    for path in selected {
        if path.is_dir() {
            for entry in walk_tree(path) {
                if !entry.is_dir {
                    files.push(entry.path);
                }
            }
        } else if path.is_file() {
            files.push(path.clone());
        }
    }

    files.sort();
    files.dedup();
    files
}

/// Mode 1: a nested-brace view of everything in the selection. Folders are
/// rendered as `name { ... }`, empty folders as `name {}`, and files as bare
/// names. Siblings are comma-separated, one per line, indented by depth.
pub fn build_structure(base: &Path, selected: &[PathBuf]) -> String {
    let mut root = TreeNode::default();

    for path in selected {
        if path.is_dir() {
            // The selected folder itself, plus everything inside it
            // (including empty subfolders, which `walk_tree` still reports).
            insert_path(&mut root, base, path, true);
            for entry in walk_tree(path) {
                insert_path(&mut root, base, &entry.path, entry.is_dir);
            }
        } else if path.is_file() {
            insert_path(&mut root, base, path, false);
        }
    }

    let mut out = String::from("{\n");
    root.render_children(&mut out, 1);
    out.push_str("}\n");
    out
}

/// Insert one path into the tree, marking the final component as a dir or file.
fn insert_path(root: &mut TreeNode, base: &Path, path: &Path, is_dir: bool) {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    root.insert(&parts, is_dir);
}

/// Mode 2: each file's relative path followed by its contents in a code fence.
pub fn build_contents(base: &Path, selected: &[PathBuf]) -> String {
    let files = collect_files(selected);
    let mut out = String::new();

    for file in &files {
        let rel = file.strip_prefix(base).unwrap_or(file);
        let rel_str = rel.display();

        match fs::read_to_string(file) {
            Ok(contents) => {
                out.push_str(&format!("// @File: {}\n", rel_str));
                out.push_str(&format!("```{}\n", lang_for(file)));
                out.push_str(&contents);
                if !contents.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n\n");
            }
            Err(_) => {
                // Binary or non-UTF-8 file — note it and move on.
                out.push_str(&format!("// @File: {} (skipped: not text)\n\n", rel_str));
            }
        }
    }

    out
}

/// Path to the small state file that remembers the previous selection.
/// Lives in the user's home directory.
fn state_file_path() -> Option<PathBuf> {
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".bmo_cli_last_files"))
}

/// Remember the project root and the chosen paths so "Last files" can reuse
/// them next time. Format: line 1 is the base path, the rest are the files.
pub fn save_selection(base: &Path, paths: &[PathBuf]) {
    let file = match state_file_path() {
        Some(file) => file,
        None => return,
    };

    let mut body = base.to_string_lossy().to_string();
    for path in paths {
        body.push('\n');
        body.push_str(&path.to_string_lossy());
    }

    let _ = fs::write(file, body); // best-effort; a failed save just skips memory
}

/// Load the previous selection, dropping any paths that no longer exist.
/// Returns the saved base path alongside the surviving files, or `None` if
/// there's nothing usable to reuse.
pub fn load_selection() -> Option<(PathBuf, Vec<PathBuf>)> {
    let file = state_file_path()?;
    let contents = fs::read_to_string(&file).ok()?;

    let mut lines = contents.lines();
    let base = PathBuf::from(lines.next()?.trim());

    let files: Vec<PathBuf> = lines
        .filter(|line| !line.trim().is_empty())
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .collect();

    if files.is_empty() {
        return None;
    }

    Some((base, files))
}

/// Map a file extension to a Markdown code-fence language hint.
fn lang_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("jsx") => "jsx",
        Some("tsx") => "tsx",
        Some("go") => "go",
        Some("java") => "java",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("cxx") | Some("hpp") => "cpp",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("sh") => "bash",
        Some("html") => "html",
        Some("css") => "css",
        Some("json") => "json",
        Some("toml") => "toml",
        Some("yaml") | Some("yml") => "yaml",
        Some("md") => "markdown",
        Some("sql") => "sql",
        _ => "",
    }
}

/// A simple nested tree used to render the structure view. `is_dir` lets us
/// tell an empty folder (rendered `{}`) apart from a file (rendered bare).
#[derive(Default)]
struct TreeNode {
    is_dir: bool,
    children: BTreeMap<String, TreeNode>,
}

impl TreeNode {
    fn insert(&mut self, parts: &[String], is_dir: bool) {
        if let Some((first, rest)) = parts.split_first() {
            let child = self.children.entry(first.clone()).or_default();
            if rest.is_empty() {
                // Leaf: mark it a folder if this entry is one. Never downgrade
                // a node already known to be a directory.
                if is_dir {
                    child.is_dir = true;
                }
            } else {
                child.is_dir = true; // anything with children is a folder
                child.insert(rest, is_dir);
            }
        }
    }

    /// Render this node's children as comma-separated, indented entries.
    fn render_children(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        let count = self.children.len();

        for (i, (name, child)) in self.children.iter().enumerate() {
            let comma = if i + 1 < count { "," } else { "" };

            if child.is_dir {
                if child.children.is_empty() {
                    out.push_str(&format!("{}{} {{}}{}\n", indent, name, comma));
                } else {
                    out.push_str(&format!("{}{} {{\n", indent, name));
                    child.render_children(out, depth + 1);
                    out.push_str(&format!("{}}}{}\n", indent, comma));
                }
            } else {
                out.push_str(&format!("{}{}{}\n", indent, name, comma));
            }
        }
    }
}
