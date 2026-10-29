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
    ".env", // plain environment file – likely contains secrets
];

/// Name prefixes that are always excluded (e.g. `.env.local`, `.env.production`).
const IGNORED_PREFIXES: &[&str] = &[".env."];

/// Returns true if `path` should never appear in export output regardless of
/// how it entered the selection (interactive picker, `-clip`, or `-last`).
///
/// Checks every component of the path relative to `base` so that files nested
/// inside an ignored directory (e.g. `target/release/bmo`) are caught even
/// when they arrive as absolute paths via `-clip`.
fn is_excluded_from_export(base: &Path, path: &Path, gitignore: &[String]) -> bool {
    let rel = path.strip_prefix(base).unwrap_or(path);

    for component in rel.components() {
        let name = component.as_os_str().to_string_lossy();
        let n: &str = &name;
        if IGNORED.contains(&n) || IGNORED_PREFIXES.iter().any(|p| n.starts_with(p)) {
            return true;
        }
    }

    // Catch env files by extension: `vm_back.env`, `example.env`, etc.
    if path.extension().is_some_and(|e| e == "env") {
        return true;
    }

    matches_gitignore(base, path, gitignore)
}

/// Read patterns from `<base>/.gitignore`. Blank lines, comments (`#`), and
/// negations (`!`) are dropped; a leading `/` is stripped so patterns remain
/// relative to the project root.
fn load_gitignore(base: &Path) -> Vec<String> {
    match fs::read_to_string(base.join(".gitignore")) {
        Ok(content) => content
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
            .map(|l| l.trim_start_matches('/').trim_end_matches('/').to_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Returns true if `path` matches any of the supplied gitignore `patterns`.
///
/// Supports:
/// - **Plain names** (`target`, `.env`): match against the filename *or* any
///   component of the relative path.
/// - **Filename globs** (`*.env`, `*.log`): `*` matches zero or more
///   characters within a single filename component.
/// - **Path-relative patterns** (`config/secrets.json`): matched against the
///   full path relative to `base`.
fn matches_gitignore(base: &Path, path: &Path, patterns: &[String]) -> bool {
    let rel = match path.strip_prefix(base) {
        Ok(r) => r,
        Err(_) => return false,
    };
    let rel_fwd: String = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    for pattern in patterns {
        if pattern.contains('/') {
            // Path-relative: match against the forward-slash relative path.
            if rel_fwd == *pattern || rel_fwd.starts_with(&format!("{}/", pattern)) {
                return true;
            }
        } else if pattern.contains('*') {
            // Filename glob: only applied to the filename component.
            if glob_filename_match(pattern, &file_name) {
                return true;
            }
        } else {
            // Plain name: match the filename or any component in the path.
            if file_name == *pattern
                || rel
                    .components()
                    .any(|c| c.as_os_str() == pattern.as_str())
            {
                return true;
            }
        }
    }
    false
}

/// Single-level glob using `*` as the only wildcard character (no `**`, no `?`).
/// `prefix*suffix` style patterns with multiple `*` are not supported and
/// return false.
fn glob_filename_match(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => {
            name.starts_with(prefix)
                && name.ends_with(suffix)
                && name.len() >= prefix.len() + suffix.len()
        }
        None => pattern == name,
    }
}

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

    // Collect (path, is_dir) using the entry's own file type, which does NOT
    // follow symlinks. Skipping symlinks means a cyclic link (a folder linking
    // back up its own tree) can't send the walk into an infinite loop.
    let mut items: Vec<(PathBuf, bool)> = Vec::new();
    for entry in read.filter_map(|e| e.ok()) {
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            continue;
        }
        items.push((entry.path(), file_type.is_dir()));
    }

    // Folders first, then files; alphabetical within each group.
    items.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.file_name().cmp(&b.0.file_name())));

    for (path, is_dir) in items {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if IGNORED.contains(&name.as_str())
            || IGNORED_PREFIXES.iter().any(|p| name.starts_with(p))
        {
            continue;
        }

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

/// The files to output: every checked path that is a real file and is not
/// excluded by the shared export rules (IGNORED names, `.env` extension,
/// `.gitignore` patterns).
fn collect_files(base: &Path, selected: &[PathBuf]) -> Vec<PathBuf> {
    let gitignore = load_gitignore(base);
    let mut files: Vec<PathBuf> = selected
        .iter()
        .filter(|path| path.is_file() && !is_excluded_from_export(base, path, &gitignore))
        .cloned()
        .collect();

    files.sort();
    files.dedup();
    files
}

/// Mode 1: a nested-brace view of the selection. Applies the same exclusion
/// rules as `build_contents` so the two outputs stay in sync.
pub fn build_structure(base: &Path, selected: &[PathBuf]) -> String {
    let gitignore = load_gitignore(base);
    let mut root = TreeNode::default();

    for path in selected {
        if is_excluded_from_export(base, path, &gitignore) {
            continue;
        }
        insert_path(&mut root, base, path, path.is_dir());
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
    let files = collect_files(base, selected);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    fn make_temp_file(base: &Path, rel: &str) {
        let full = base.join(rel);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, b"").unwrap();
    }

    // ---------------------------------------------------------------------------
    // glob_filename_match
    // ---------------------------------------------------------------------------

    #[test]
    fn glob_matches_suffix_wildcard() {
        assert!(glob_filename_match("*.env", "vm_back.env"));
        assert!(glob_filename_match("*.env", "example.env"));
        assert!(glob_filename_match("*.log", "app.log"));
        assert!(!glob_filename_match("*.env", "dotenv"));
        assert!(!glob_filename_match("*.env", ".env.local"));
    }

    #[test]
    fn glob_matches_prefix_wildcard() {
        assert!(glob_filename_match(".env*", ".env.local"));
        assert!(glob_filename_match(".env*", ".env.production"));
        assert!(!glob_filename_match(".env*", "vm_back.env"));
    }

    #[test]
    fn glob_exact_without_wildcard() {
        assert!(glob_filename_match(".env", ".env"));
        assert!(!glob_filename_match(".env", "dotenv"));
    }

    // ---------------------------------------------------------------------------
    // matches_gitignore
    // ---------------------------------------------------------------------------

    #[test]
    fn gitignore_plain_name_matches_any_component() {
        let base = temp_dir().join("bmo_gi_plain");
        let path = base.join("src/secrets/config.rs");
        assert!(matches_gitignore(&base, &path, &["secrets".to_owned()]));
        assert!(!matches_gitignore(&base, &path, &["other".to_owned()]));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn gitignore_plain_name_matches_filename() {
        let base = temp_dir().join("bmo_gi_fname");
        let path = base.join(".env");
        assert!(matches_gitignore(&base, &path, &[".env".to_owned()]));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn gitignore_glob_matches_extension() {
        let base = temp_dir().join("bmo_gi_glob");
        let vm = base.join("vm_back.env");
        let ex = base.join("example.env");
        let src = base.join("src/main.rs");
        let patterns = vec!["*.env".to_owned()];
        assert!(matches_gitignore(&base, &vm, &patterns));
        assert!(matches_gitignore(&base, &ex, &patterns));
        assert!(!matches_gitignore(&base, &src, &patterns));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn gitignore_path_pattern_matches_relative() {
        let base = temp_dir().join("bmo_gi_path");
        let path = base.join("config/secrets.json");
        assert!(matches_gitignore(
            &base,
            &path,
            &["config/secrets.json".to_owned()]
        ));
        assert!(!matches_gitignore(
            &base,
            &path,
            &["other/secrets.json".to_owned()]
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    // ---------------------------------------------------------------------------
    // is_excluded_from_export
    // ---------------------------------------------------------------------------

    #[test]
    fn excludes_env_by_name() {
        let base = temp_dir().join("bmo_excl_name");
        assert!(is_excluded_from_export(&base, &base.join(".env"), &[]));
        assert!(is_excluded_from_export(
            &base,
            &base.join(".env.local"),
            &[]
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn excludes_env_by_extension() {
        let base = temp_dir().join("bmo_excl_ext");
        assert!(is_excluded_from_export(
            &base,
            &base.join("vm_back.env"),
            &[]
        ));
        assert!(is_excluded_from_export(
            &base,
            &base.join("example.env"),
            &[]
        ));
        assert!(!is_excluded_from_export(
            &base,
            &base.join("src/main.rs"),
            &[]
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn excludes_files_nested_inside_ignored_dir() {
        let base = temp_dir().join("bmo_excl_nested");
        // A file inside target/ should be excluded even as an absolute path
        let nested = base.join("target/release/bmo");
        assert!(is_excluded_from_export(&base, &nested, &[]));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn excludes_via_gitignore_pattern() {
        let base = temp_dir().join("bmo_excl_gi");
        let secret = base.join("db_password.env");
        let patterns = vec!["*.env".to_owned()];
        assert!(is_excluded_from_export(&base, &secret, &patterns));
        let _ = std::fs::remove_dir_all(&base);
    }

    // ---------------------------------------------------------------------------
    // collect_files / build_contents  (export-stage guard)
    // ---------------------------------------------------------------------------

    #[test]
    fn collect_files_excludes_env_files() {
        let base = temp_dir().join("bmo_collect_env");
        make_temp_file(&base, "src/main.rs");
        make_temp_file(&base, ".env");
        make_temp_file(&base, ".env.local");
        make_temp_file(&base, "vm_back.env");

        let selected = vec![
            base.join("src/main.rs"),
            base.join(".env"),
            base.join(".env.local"),
            base.join("vm_back.env"),
        ];
        let files = collect_files(&base, &selected);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("main.rs"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn collect_files_respects_gitignore() {
        let base = temp_dir().join("bmo_collect_gi");
        make_temp_file(&base, "src/main.rs");
        make_temp_file(&base, "secrets/key.txt");
        fs::write(base.join(".gitignore"), "secrets/\n").unwrap();

        let selected = vec![base.join("src/main.rs"), base.join("secrets/key.txt")];
        let files = collect_files(&base, &selected);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("main.rs"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
