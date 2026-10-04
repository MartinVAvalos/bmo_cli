use std::path::{Path, PathBuf};

use console::{style, truncate_str, Key, Term};
use dialoguer::{theme::ColorfulTheme, Select};

use crate::utilities::file_tools::{self, Entry};

/// What the user wants the tool to print.
pub enum Mode {
    Structure,
    Contents,
    Both,
}

/// Ask which kind of output to produce.
pub fn select_mode() -> Mode {
    let options = ["Code", "File structure", "Both"];

    let index = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("What should I copy?")
        .items(&options)
        .default(0)
        .interact()
        .unwrap();

    match index {
        0 => Mode::Contents,
        1 => Mode::Structure,
        _ => Mode::Both,
    }
}

/// Interactive picker. Renders the whole tree and lets the user move with the
/// arrow keys, toggle with space, and confirm with enter. Toggling a folder
/// cascades to everything inside it AND reveals its contents; an unselected
/// folder stays collapsed so the list stays short. Top-level rows always show.
pub fn select_paths(base: &Path) -> Vec<PathBuf> {
    let entries = file_tools::walk_tree(base);

    if entries.is_empty() {
        eprintln!("No files found under {}", base.display());
        return Vec::new();
    }

    let parents = parent_indices(&entries);
    let mut checked = vec![false; entries.len()]; // selection (for output)
    let mut expanded = vec![false; entries.len()]; // which folders are open
    let mut cursor = 0usize; // position within the currently-visible rows
    let mut scroll = 0usize;
    let mut jump = 0usize; // built up by ← (+10 each), consumed by the next ↑/↓

    // Search state
    let mut search_active = false;
    let mut search_query = String::new();

    // Draw on stderr so stdout stays clean for piping.
    let term = Term::stderr();
    let _ = term.hide_cursor();

    let mut drawn = 0usize;
    let mut cancelled = false;

    loop {
        // Determine visible indices: either from tree or from search
        let visible = if search_active {
            search_indices(&entries, base, &search_query)
        } else {
            visible_indices(&entries, &parents, &expanded)
        };

        if cursor >= visible.len() {
            cursor = visible.len().saturating_sub(1);
        }

        // Recompute the viewport every frame so terminal resizes are handled.
        let (rows, cols) = term.size();
        let cols = cols as usize;
        // Reserve lines: title + footer (2 lines).
        // Search mode adds a search query line, so reserve 3 total.
        let reserved = if search_active { 3 } else { 2 };
        let viewport = (rows as usize).saturating_sub(reserved).max(1);
        scroll = clamp_scroll(cursor, scroll, viewport, visible.len());

        if drawn > 0 {
            let _ = term.clear_last_lines(drawn);
        }
        drawn = render_with_mode(
            &term,
            &entries,
            &checked,
            &expanded,
            &visible,
            cursor,
            scroll,
            viewport,
            jump,
            cols,
            search_active,
            &search_query,
            base,
        );

        match term.read_key() {
            Ok(Key::Char('i')) if !search_active => {
                search_active = true;
                search_query.clear();
                cursor = 0;
                scroll = 0;
            }
            Ok(Key::Escape) if search_active => {
                search_active = false;
                search_query.clear();
                cursor = 0;
                scroll = 0;
            }
            Ok(Key::Char(' ')) if search_active => {
                if !visible.is_empty() {
                    toggle(&entries, &mut checked, visible[cursor]);
                }
            }
            Ok(Key::Char(c)) if search_active => {
                search_query.push(c);
                cursor = 0;
                scroll = 0;
            }
            Ok(Key::Backspace) if search_active => {
                search_query.pop();
                cursor = 0;
                scroll = 0;
            }
            Ok(Key::ArrowUp) if search_active => {
                cursor = move_up(cursor, 1);
            }
            Ok(Key::ArrowDown) if search_active => {
                cursor = move_down(cursor, 1, visible.len());
            }
            Ok(Key::ArrowLeft) if search_active => {
                cursor = move_up(cursor, 10);
            }
            Ok(Key::ArrowRight) if search_active => {
                cursor = move_down(cursor, 10, visible.len());
            }
            Ok(Key::ArrowUp) if !search_active => {
                cursor = move_up(cursor, jump.max(1));
                jump = 0;
            }
            Ok(Key::ArrowDown) if !search_active => {
                cursor = move_down(cursor, jump.max(1), visible.len());
                jump = 0;
            }
            Ok(Key::ArrowLeft) if !search_active => {
                jump += 10;
            }
            Ok(Key::ArrowRight) if !search_active => {
                let i = visible[cursor];
                if entries[i].is_dir {
                    expanded[i] = !expanded[i];
                }
            }
            Ok(Key::Char(' ')) if !search_active => toggle(&entries, &mut checked, visible[cursor]),
            Ok(Key::Enter) if !search_active => break,
            Ok(Key::Escape) if !search_active => {
                cancelled = true;
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    if drawn > 0 {
        let _ = term.clear_last_lines(drawn);
    }
    let _ = term.show_cursor();

    if cancelled {
        return Vec::new();
    }

    entries
        .iter()
        .zip(checked.iter())
        .filter(|(_, &is_checked)| is_checked)
        .map(|(entry, _)| entry.path.clone())
        .collect()
}

/// Render with mode support (normal tree or search)
#[allow(clippy::too_many_arguments)]
fn render_with_mode(
    term: &Term,
    entries: &[Entry],
    checked: &[bool],
    expanded: &[bool],
    visible: &[usize],
    cursor: usize,
    scroll: usize,
    viewport: usize,
    jump: usize,
    cols: usize,
    search_active: bool,
    search_query: &str,
    base: &Path,
) -> usize {
    if search_active {
        render_search(
            term,
            entries,
            checked,
            visible,
            cursor,
            scroll,
            viewport,
            cols,
            search_query,
            base,
        )
    } else {
        render(
            term, entries, checked, expanded, visible, cursor, scroll, viewport, jump, cols,
        )
    }
}

/// Draw the title, the visible window of rows, and a footer; returns the number
/// of lines written so the caller can clear exactly that many next frame. Long
/// rows are truncated to the terminal width so nothing wraps onto a second line.
#[allow(clippy::too_many_arguments)]
fn render(
    term: &Term,
    entries: &[Entry],
    checked: &[bool],
    expanded: &[bool],
    visible: &[usize],
    cursor: usize,
    scroll: usize,
    viewport: usize,
    jump: usize,
    cols: usize,
) -> usize {
    let mut lines = 0;

    let title = style("Select files/folders").bold();
    let _ = term.write_line(&title.to_string());
    lines += 1;

    let end = (scroll + viewport).min(visible.len());
    for (pos, &i) in visible.iter().enumerate().skip(scroll).take(end - scroll) {
        let k = pos;
        let entry = &entries[i];
        let pointer = if k == cursor { "❯" } else { " " };
        let mark = if checked[i] { "[x]" } else { "[ ]" };
        let indent = "  ".repeat(entry.depth);
        let name = entry
            .path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // Folders show ▾ when open and ▸ when closed (independent of selection).
        // Files get two spaces so names line up under the folder names.
        let name = if entry.is_dir {
            let arrow = if expanded[i] { "▾" } else { "▸" };
            format!("{} {}/", arrow, name)
        } else {
            format!("  {}", name)
        };

        let raw = format!("{} {} {}{}", pointer, mark, indent, name);
        // Truncate on the plain text (before styling) so width math is correct.
        let raw = truncate_str(&raw, cols, "…").to_string();
        let line = if k == cursor {
            style(raw).cyan().bold().to_string()
        } else {
            raw
        };
        let _ = term.write_line(&line);
        lines += 1;
    }

    // Footer: position, selected count, scroll markers, pending jump, key hints.
    let selected = checked.iter().filter(|&&c| c).count();
    let up = if scroll > 0 { "↑" } else { " " };
    let down = if end < visible.len() { "↓" } else { " " };
    let jump_note = if jump > 0 {
        format!(" · jump {}", jump)
    } else {
        String::new()
    };
    let footer = format!(
        "{}{} {}/{} shown · {} selected{} · → open · ← +10 · space select · enter ok · esc cancel",
        up,
        down,
        cursor + 1,
        visible.len(),
        selected,
        jump_note,
    );
    let footer = truncate_str(&footer, cols, "").to_string();
    let _ = term.write_line(&style(footer).dim().to_string());
    lines += 1;

    lines
}

/// Render search results with full paths
#[allow(clippy::too_many_arguments)]
fn render_search(
    term: &Term,
    entries: &[Entry],
    checked: &[bool],
    visible: &[usize],
    cursor: usize,
    scroll: usize,
    viewport: usize,
    cols: usize,
    search_query: &str,
    base: &Path,
) -> usize {
    let mut lines = 0;

    let title = style("Select files/folders — SEARCH").bold();
    let _ = term.write_line(&title.to_string());
    lines += 1;

    // Show search query
    let search_line = format!("Search: {}", search_query);
    let _ = term.write_line(&search_line);
    lines += 1;

    if visible.is_empty() {
        let _ = term.write_line("No matches");
        lines += 1;
        let footer = "0 results · type to search · esc exit".to_string();
        let _ = term.write_line(&style(footer).dim().to_string());
        lines += 1;
        return lines;
    }

    let end = (scroll + viewport).min(visible.len());
    for (pos, &i) in visible.iter().enumerate().skip(scroll).take(end - scroll) {
        let k = pos;
        let entry = &entries[i];
        let pointer = if k == cursor { "❯" } else { " " };
        let mark = if checked[i] { "[x]" } else { "[ ]" };

        let rel_path = entry
            .path
            .strip_prefix(base)
            .unwrap_or(&entry.path)
            .to_string_lossy()
            .to_string()
            .replace('\\', "/");
        let display_path = if entry.is_dir {
            format!("{}/", rel_path)
        } else {
            rel_path
        };

        let raw = format!("{} {} {}", pointer, mark, display_path);
        let raw = truncate_str(&raw, cols, "…").to_string();
        let line = if k == cursor {
            style(raw).cyan().bold().to_string()
        } else {
            raw
        };
        let _ = term.write_line(&line);
        lines += 1;
    }

    // Footer for search mode
    let up = if scroll > 0 { "↑" } else { " " };
    let down = if end < visible.len() { "↓" } else { " " };
    let footer = format!(
        "{}{} {}/{} results · ↑↓ navigate · ←→ jump · space select · esc exit",
        up,
        down,
        visible.len().min(cursor + 1),
        visible.len()
    );
    let footer = truncate_str(&footer, cols, "").to_string();
    let _ = term.write_line(&style(footer).dim().to_string());
    lines += 1;

    lines
}

/// Search all entries for paths matching the query (case-insensitive substring).
/// Returns original entry indices in their deterministic order.
/// Normalizes paths to forward slashes for cross-platform compatibility.
fn search_indices(entries: &[Entry], base: &Path, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..entries.len()).collect();
    }

    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for (i, entry) in entries.iter().enumerate() {
        let path_str = entry
            .path
            .strip_prefix(base)
            .unwrap_or(&entry.path)
            .to_string_lossy()
            .to_string()
            .replace('\\', "/");

        if path_str.to_lowercase().contains(&query_lower) {
            results.push(i);
        }
    }

    results
}

/// Toggle the row under the cursor. For a folder this flips the folder and all
/// of its descendants to the same new value; for a file it flips only the file.
fn toggle(entries: &[Entry], checked: &mut [bool], index: usize) {
    let new_value = !checked[index];
    let end = subtree_end(entries, index);
    for slot in checked.iter_mut().take(end).skip(index) {
        *slot = new_value;
    }
}

/// For each entry, the index of its enclosing folder (None for top-level rows).
/// Relies on `walk_tree`'s pre-order output: an entry's parent is the nearest
/// earlier folder one depth shallower.
fn parent_indices(entries: &[Entry]) -> Vec<Option<usize>> {
    let mut parents = vec![None; entries.len()];
    let mut ancestors: Vec<usize> = Vec::new(); // stack of open folder indices

    for (i, entry) in entries.iter().enumerate() {
        while let Some(&top) = ancestors.last() {
            if entries[top].depth >= entry.depth {
                ancestors.pop();
            } else {
                break;
            }
        }
        parents[i] = ancestors.last().copied();
        if entry.is_dir {
            ancestors.push(i);
        }
    }

    parents
}

/// Indices of the rows that should be shown: a row is visible when every folder
/// above it is open (expanded). Top-level rows are always visible.
fn visible_indices(entries: &[Entry], parents: &[Option<usize>], expanded: &[bool]) -> Vec<usize> {
    let mut is_visible = vec![false; entries.len()];
    let mut visible = Vec::new();

    for i in 0..entries.len() {
        let shown = match parents[i] {
            None => true,
            Some(parent) => is_visible[parent] && expanded[parent],
        };
        is_visible[i] = shown;
        if shown {
            visible.push(i);
        }
    }

    visible
}

/// Move the cursor up by `step`, stopping at the top.
fn move_up(cursor: usize, step: usize) -> usize {
    cursor.saturating_sub(step)
}

/// Move the cursor down by `step`, stopping at the last row.
fn move_down(cursor: usize, step: usize, len: usize) -> usize {
    (cursor + step).min(len.saturating_sub(1))
}

/// Compute the first-visible-row offset that keeps `cursor` within a window of
/// `viewport` rows, without scrolling past the end of a `len`-item list.
fn clamp_scroll(cursor: usize, scroll: usize, viewport: usize, len: usize) -> usize {
    let mut scroll = scroll;
    if cursor < scroll {
        scroll = cursor;
    } else if cursor >= scroll + viewport {
        scroll = cursor + 1 - viewport;
    }
    let max_scroll = len.saturating_sub(viewport);
    scroll.min(max_scroll)
}

/// Exclusive end index of the subtree rooted at `index`. Because `walk_tree`
/// emits entries in pre-order, a node's descendants are exactly the following
/// rows with a greater depth.
fn subtree_end(entries: &[Entry], index: usize) -> usize {
    let depth = entries[index].depth;
    let mut end = index + 1;
    while end < entries.len() && entries[end].depth > depth {
        end += 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn e(path: &str, depth: usize, is_dir: bool) -> Entry {
        Entry {
            path: PathBuf::from(path),
            depth,
            is_dir,
        }
    }

    // Tree used below:
    //   src/        (0)
    //     a.rs      (1)
    //     sub/      (1)
    //       b.rs    (2)
    //   root.txt    (0)
    fn sample() -> Vec<Entry> {
        vec![
            e("src", 0, true),
            e("src/a.rs", 1, false),
            e("src/sub", 1, true),
            e("src/sub/b.rs", 2, false),
            e("root.txt", 0, false),
        ]
    }

    #[test]
    fn toggling_a_folder_cascades_to_all_descendants() {
        let entries = sample();
        let mut checked = vec![false; entries.len()];

        toggle(&entries, &mut checked, 0); // toggle src/
        assert_eq!(checked, vec![true, true, true, true, false]);

        toggle(&entries, &mut checked, 0); // toggle src/ again -> clears subtree
        assert_eq!(checked, vec![false, false, false, false, false]);
    }

    #[test]
    fn toggling_a_file_leaves_its_parent_folder_alone() {
        let entries = sample();
        let mut checked = vec![true; entries.len()]; // everything on
        toggle(&entries, &mut checked, 1); // deselect src/a.rs
                                           // a.rs is off, but src/ (0) and the rest stay on
        assert_eq!(checked, vec![true, false, true, true, true]);
    }

    #[test]
    fn parent_indices_point_to_enclosing_folder() {
        let entries = sample();
        // src(0)->None, a.rs(1)->src(0), sub(2)->src(0), b.rs(3)->sub(2), root.txt(4)->None
        assert_eq!(
            parent_indices(&entries),
            vec![None, Some(0), Some(0), Some(2), None]
        );
    }

    #[test]
    fn nothing_open_shows_only_top_level() {
        let entries = sample();
        let parents = parent_indices(&entries);
        let expanded = vec![false; entries.len()];
        // Only the depth-0 rows: src (0) and root.txt (4).
        assert_eq!(visible_indices(&entries, &parents, &expanded), vec![0, 4]);
    }

    #[test]
    fn opening_a_folder_reveals_only_its_direct_children() {
        let entries = sample();
        let parents = parent_indices(&entries);
        let mut expanded = vec![false; entries.len()];
        expanded[0] = true; // open src/
                            // src's direct children show (a.rs, sub) but sub's child b.rs stays hidden.
        assert_eq!(
            visible_indices(&entries, &parents, &expanded),
            vec![0, 1, 2, 4]
        );
    }

    #[test]
    fn opening_nested_folders_reveals_deeper_rows() {
        let entries = sample();
        let parents = parent_indices(&entries);
        let mut expanded = vec![false; entries.len()];
        expanded[0] = true; // src/
        expanded[2] = true; // sub/
        assert_eq!(
            visible_indices(&entries, &parents, &expanded),
            vec![0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn selecting_a_folder_does_not_open_it() {
        // Selection (checked) must not affect visibility; only expansion does.
        let entries = sample();
        let parents = parent_indices(&entries);
        let mut checked = vec![false; entries.len()];
        toggle(&entries, &mut checked, 0); // select src/ and its subtree
        let expanded = vec![false; entries.len()]; // nothing opened
        assert_eq!(visible_indices(&entries, &parents, &expanded), vec![0, 4]);
    }

    #[test]
    fn jump_moves_by_step_and_clamps() {
        assert_eq!(move_down(0, 20, 100), 20);
        assert_eq!(move_up(20, 20), 0);
        assert_eq!(move_down(95, 20, 100), 99); // clamps at the last row
        assert_eq!(move_up(5, 20), 0); // clamps at the top
        assert_eq!(move_down(0, 1, 100), 1); // ordinary single step
    }

    #[test]
    fn subtree_end_of_a_leaf_is_the_next_row() {
        let entries = sample();
        assert_eq!(subtree_end(&entries, 1), 2); // a.rs has no descendants
        assert_eq!(subtree_end(&entries, 0), 4); // src/ covers rows 0..4
    }

    // viewport of 5 rows over a 20-item list unless noted.
    #[test]
    fn scroll_keeps_cursor_in_view() {
        // Near the top: no scrolling needed.
        assert_eq!(clamp_scroll(0, 0, 5, 20), 0);
        assert_eq!(clamp_scroll(4, 0, 5, 20), 0);
        // Stepping past the bottom edge scrolls down by one.
        assert_eq!(clamp_scroll(5, 0, 5, 20), 1);
        // Jumping to the last item pins the window to the end.
        assert_eq!(clamp_scroll(19, 1, 5, 20), 15); // max_scroll = 20 - 5
                                                    // Wrapping back to the top scrolls all the way up.
        assert_eq!(clamp_scroll(0, 15, 5, 20), 0);
        // Moving up within the window above the fold.
        assert_eq!(clamp_scroll(14, 15, 5, 20), 14);
    }

    #[test]
    fn scroll_stays_zero_when_list_fits() {
        // viewport bigger than the list: never scrolls.
        assert_eq!(clamp_scroll(0, 0, 10, 3), 0);
        assert_eq!(clamp_scroll(2, 0, 10, 3), 0);
    }

    #[test]
    fn search_case_insensitive() {
        // Create test entries with mixed case
        let entries = vec![
            e("src/Main.rs", 1, false),
            e("src/utilities/terminal.rs", 2, false),
        ];
        let results = search_indices(&entries, &PathBuf::from(""), "MAIN");
        assert_eq!(results, vec![0]); // Only Main.rs should match
    }

    #[test]
    fn search_substring_match() {
        let entries = vec![
            e("src/utilities/file_tools.rs", 2, false),
            e("src/utilities/terminal.rs", 2, false),
        ];
        let results = search_indices(&entries, &PathBuf::from(""), "utilities");
        assert_eq!(results, vec![0, 1]); // Both files under utilities should match
    }

    #[test]
    fn search_relative_path_matching() {
        let entries = vec![e("src/utilities/terminal.rs", 2, false)];
        let results = search_indices(&entries, &PathBuf::from(""), "utilities/terminal");
        assert_eq!(results, vec![0]); // Should match the path separator query
    }

    #[test]
    fn search_empty_query_returns_all() {
        let entries = sample();
        let results = search_indices(&entries, &PathBuf::from(""), "");
        assert_eq!(results.len(), entries.len());
        assert_eq!(results, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn search_no_matches() {
        let entries = sample();
        let results = search_indices(&entries, &PathBuf::from(""), "nonexistent_xyz");
        assert_eq!(results, Vec::<usize>::new()); // Exact empty vector
    }

    #[test]
    fn search_nested_file_found_when_collapsed() {
        let entries = vec![
            e("src", 0, true),
            e("src/sub", 1, true),
            e("src/sub/b.rs", 2, false),
        ];
        let results = search_indices(&entries, &PathBuf::from(""), "b.rs");
        assert_eq!(results, vec![2]); // b.rs found even though parents are collapsed
    }

    #[test]
    fn search_preserves_entry_order() {
        let entries = vec![
            e("a.rs", 0, false),
            e("b.rs", 0, false),
            e("c.rs", 0, false),
            e("d.rs", 0, false),
        ];
        let results = search_indices(&entries, &PathBuf::from(""), "rs");
        assert_eq!(results, vec![0, 1, 2, 3]); // Order preserved
    }

    #[test]
    fn search_normalizes_windows_paths() {
        // Simulate Windows-style path with backslashes
        let entries = vec![e("src\\utilities\\terminal.rs", 2, false)];
        let results = search_indices(&entries, &PathBuf::from(""), "utilities/terminal");
        assert_eq!(results, vec![0]); // Should match despite backslashes
    }
}
