use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(test)]
use std::env;

/// Read text from the system clipboard.
/// Returns None if no clipboard reader is available.
/// Prefer `read_clipboard_candidates()` for better multi-representation support.
#[allow(dead_code)]
pub fn read_from_clipboard() -> Option<String> {
    if cfg!(target_os = "windows") {
        let output = match Command::new("powershell")
            .args(["-Command", "Get-Clipboard"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return None,
        };

        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).to_string());
        }
    } else if cfg!(target_os = "macos") {
        // Try pbpaste with -Prefer txt for plain text first
        match Command::new("pbpaste").args(["-Prefer", "txt"]).output() {
            Ok(output) if output.status.success() => {
                return Some(String::from_utf8_lossy(&output.stdout).to_string());
            }
            _ => {
                // Fall back to ordinary pbpaste
                match Command::new("pbpaste").output() {
                    Ok(output) if output.status.success() => {
                        return Some(String::from_utf8_lossy(&output.stdout).to_string());
                    }
                    _ => {}
                }
            }
        }

        return None;
    } else {
        // Linux: try multiple clipboard readers in order
        for cmd in &["wl-paste", "xclip", "xsel"] {
            let output = match *cmd {
                "wl-paste" => Command::new(cmd).output(),
                "xclip" => Command::new(cmd)
                    .args(["-selection", "clipboard", "-o"])
                    .output(),
                "xsel" => Command::new(cmd).args(["--clipboard", "--output"]).output(),
                _ => continue,
            };

            match output {
                Ok(out) if out.status.success() => {
                    return Some(String::from_utf8_lossy(&out.stdout).to_string());
                }
                _ => continue,
            }
        }
    }

    None
}

/// Read multiple possible clipboard representations.
/// On macOS, tries different clipboard flavors and returns all successful ones.
/// On other platforms, returns at most one representation.
pub fn read_clipboard_candidates() -> Vec<String> {
    let mut candidates = Vec::new();

    if cfg!(target_os = "macos") {
        // Try pbpaste with -Prefer txt
        if let Ok(output) = Command::new("pbpaste").args(["-Prefer", "txt"]).output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if !text.is_empty() {
                    candidates.push(text);
                }
            }
        }

        // Try osascript: the clipboard as text
        if let Ok(output) = Command::new("osascript")
            .args(["-e", "the clipboard as text"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if !text.is_empty() && !candidates.contains(&text) {
                    candidates.push(text);
                }
            }
        }

        // Fall back to regular pbpaste
        if let Ok(output) = Command::new("pbpaste").output() {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if !text.is_empty() && !candidates.contains(&text) {
                    candidates.push(text);
                }
            }
        }
    } else if cfg!(target_os = "windows") {
        if let Ok(output) = Command::new("powershell")
            .args(["-Command", "Get-Clipboard"])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout).to_string();
                if !text.is_empty() {
                    candidates.push(text);
                }
            }
        }
    } else {
        // Linux: try multiple clipboard readers in order
        for cmd in &["wl-paste", "xclip", "xsel"] {
            let output = match *cmd {
                "wl-paste" => Command::new(cmd).output(),
                "xclip" => Command::new(cmd)
                    .args(["-selection", "clipboard", "-o"])
                    .output(),
                "xsel" => Command::new(cmd).args(["--clipboard", "--output"]).output(),
                _ => continue,
            };

            if let Ok(out) = output {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).to_string();
                    if !text.is_empty() {
                        candidates.push(text);
                        break; // Use first successful Linux reader
                    }
                }
            }
        }
    }

    candidates
}

/// Normalize clipboard text by removing formatting artifacts and invisible characters.
/// This makes parsing robust to rich-text clipboard content from editors/IDEs.
pub fn normalize_clipboard_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());

    for c in text.chars() {
        match c {
            // Convert line endings to \n
            '\r' => {
                // Skip \r, but \r\n will be handled as just \n on the next iteration
                if !text[result.len()..].starts_with('\n') {
                    result.push('\n');
                }
            }
            // Remove invisible/formatting characters
            '\u{200B}' => {}                 // ZERO WIDTH SPACE
            '\u{200C}' => {}                 // ZERO WIDTH NON-JOINER
            '\u{200D}' => {}                 // ZERO WIDTH JOINER
            '\u{FEFF}' => {}                 // ZERO WIDTH NO-BREAK SPACE / BOM
            '\u{2028}' => result.push('\n'), // LINE SEPARATOR -> newline
            '\u{2029}' => result.push('\n'), // PARAGRAPH SEPARATOR -> newline
            '\u{00A0}' => result.push(' '),  // NON-BREAKING SPACE -> normal space
            // Normalize typographic backticks to ASCII backtick
            '\u{2018}' | '\u{2019}' => result.push('`'), // ' ' -> `
            _ => result.push(c),
        }
    }

    result
}

/// Parse clipboard content for file paths.
/// Accepts:
/// - Normal paths: `src/utilities/terminal.rs`
/// - Paths in backticks: `` `src/utilities/terminal.rs` ``
/// - Copilot breadcrumbs: `` `terminal.rs`src/utilities ``
///
/// Returns a vector of valid project files. All returned paths exist under the project root.
pub fn extract_clipboard_paths(base: &Path, text: &str) -> Vec<PathBuf> {
    // Normalize clipboard text before parsing
    let normalized_text = normalize_clipboard_text(text);

    let mut candidates = Vec::new();

    // Try to extract Copilot breadcrumbs and normal paths
    for line in normalized_text.lines() {
        let line = line.trim();

        // Skip empty lines and diff metadata
        if line.is_empty() || is_diff_metadata(line) {
            continue;
        }

        // Try Copilot format: `filename`directory/path
        if let Some(path) = extract_copilot_breadcrumb(line) {
            candidates.push(path);
            continue;
        }

        // Try full path in backticks: `src/utilities/terminal.rs`
        if let Some(path) = extract_backtick_path(line) {
            candidates.push(path);
            continue;
        }

        // Try normal path: src/utilities/terminal.rs
        let normalized = line.replace('\\', "/");
        if !normalized.is_empty() && !is_diff_metadata(&normalized) {
            candidates.push(normalized);
        }
    }

    // Validate candidates against the actual filesystem
    validate_candidates(base, candidates)
}

/// Check if a line looks like diff metadata (e.g., `+13-2`, `-2`, `+`, `-`)
fn is_diff_metadata(line: &str) -> bool {
    let trimmed = line.trim();

    // Must start with + or -
    if !trimmed.starts_with('+') && !trimmed.starts_with('-') {
        return false;
    }

    // If it starts with + or -, check if the rest is only + - and digits
    trimmed
        .chars()
        .skip(1)
        .all(|c| c == '+' || c == '-' || c.is_ascii_digit())
}

/// Extract Copilot breadcrumb format: `filename`directory/path
/// Returns: Some("directory/path/filename")
fn extract_copilot_breadcrumb(line: &str) -> Option<String> {
    // Find the backtick-enclosed filename
    let start = line.find('`')?;
    let end = line[start + 1..].find('`')?;
    let filename = &line[start + 1..start + 1 + end];

    // The directory path follows the closing backtick
    let dir_part = line[start + 1 + end + 1..].trim();

    if filename.is_empty() || dir_part.is_empty() {
        return None;
    }

    // Construct the full path
    let full_path = if dir_part.ends_with('/') || dir_part.ends_with('\\') {
        format!("{}{}", dir_part, filename)
    } else {
        format!("{}/{}", dir_part, filename)
    };

    Some(full_path)
}

/// Extract path inside backticks: `src/utilities/terminal.rs`
fn extract_backtick_path(line: &str) -> Option<String> {
    // Check if entire line (or the content) is wrapped in backticks
    let trimmed = line.trim();

    if trimmed.starts_with('`') && trimmed.ends_with('`') && trimmed.len() > 2 {
        let content = &trimmed[1..trimmed.len() - 1];
        // Make sure it looks like a path (has at least one slash or looks like a file)
        if content.contains('/') || content.contains('\\') || content.contains('.') {
            return Some(content.to_string());
        }
    }

    None
}

/// Validate that candidate paths exist and are under the project root.
fn validate_candidates(base: &Path, candidates: Vec<String>) -> Vec<PathBuf> {
    let mut valid = Vec::new();

    for candidate in candidates {
        // Normalize path separators
        let normalized = candidate.replace('\\', "/");

        // Create the full path
        let full_path = base.join(&normalized);

        // Check if it's a file
        if !full_path.is_file() {
            continue;
        }

        // Verify it's under the project root (prevent path traversal)
        if let Ok(canonical_base) = base.canonicalize() {
            if let Ok(canonical_full) = full_path.canonicalize() {
                if canonical_full.starts_with(&canonical_base) {
                    valid.push(full_path);
                }
            }
        }
    }

    // Remove duplicates while preserving order
    let mut seen = std::collections::HashSet::new();
    valid.retain(|p| seen.insert(p.clone()));

    valid
}

/// Helper function to simulate clipboard reader fallback.
/// Used for testing that failure of one candidate doesn't prevent trying the next.
#[cfg(test)]
fn try_clipboard_readers_sequence<F>(readers: &[&str], mut executor: F) -> Option<String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    for reader in readers {
        match executor(reader) {
            Ok(content) => return Some(content),
            Err(_) => continue,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_diff_metadata_recognizes_patch_stats() {
        assert!(is_diff_metadata("+13-2"));
        assert!(is_diff_metadata("+13"));
        assert!(is_diff_metadata("-2"));
        assert!(is_diff_metadata("-"));
        assert!(is_diff_metadata("+"));
        assert!(is_diff_metadata("+100-24"));
        assert!(is_diff_metadata("-87"));
        assert!(is_diff_metadata("+1"));
    }

    #[test]
    fn is_diff_metadata_rejects_paths() {
        assert!(!is_diff_metadata("src/utilities/terminal.rs"));
        assert!(!is_diff_metadata("terminal.rs"));
        assert!(!is_diff_metadata(""));
    }

    #[test]
    fn extract_copilot_breadcrumb_formats_correctly() {
        let result = extract_copilot_breadcrumb("`terminal.rs`src/utilities");
        assert_eq!(result, Some("src/utilities/terminal.rs".to_string()));

        let result = extract_copilot_breadcrumb("`file_tools.rs`src/utilities");
        assert_eq!(result, Some("src/utilities/file_tools.rs".to_string()));
    }

    #[test]
    fn extract_copilot_breadcrumb_handles_trailing_slashes() {
        let result = extract_copilot_breadcrumb("`terminal.rs`src/utilities/");
        assert_eq!(result, Some("src/utilities/terminal.rs".to_string()));
    }

    #[test]
    fn extract_copilot_breadcrumb_handles_backslashes() {
        let result = extract_copilot_breadcrumb("`terminal.rs`src\\utilities");
        assert_eq!(result, Some("src\\utilities/terminal.rs".to_string()));
    }

    #[test]
    fn extract_copilot_breadcrumb_with_diff_stats() {
        // Test that breadcrumbs with leading diff stats (+106-33) work
        let result = extract_copilot_breadcrumb("+106-33`clipboard.rs`src/utilities");
        assert_eq!(result, Some("src/utilities/clipboard.rs".to_string()));

        let result = extract_copilot_breadcrumb("+7-3`main.rs`src");
        assert_eq!(result, Some("src/main.rs".to_string()));
    }

    #[test]
    fn extract_backtick_path_full_path() {
        let result = extract_backtick_path("`src/utilities/terminal.rs`");
        assert_eq!(result, Some("src/utilities/terminal.rs".to_string()));
    }

    #[test]
    fn extract_backtick_path_ignores_partial_backticks() {
        let result = extract_backtick_path("start`src/utilities/terminal.rs");
        assert_eq!(result, None);
    }

    #[test]
    fn validate_candidates_filters_nonexistent() {
        let base = PathBuf::from(".");
        let candidates = vec!["nonexistent_file_xyz_123.rs".to_string()];
        let result = validate_candidates(&base, candidates);
        assert!(result.is_empty());
    }

    #[test]
    fn validate_candidates_deduplicates() {
        let base = env::temp_dir();
        // Create a real temp file for testing
        let test_file = base.join("test_bmo_dedup.txt");
        let _ = std::fs::write(&test_file, "test");

        let candidates = vec![
            format!("{}/test_bmo_dedup.txt", base.display()),
            format!("{}/test_bmo_dedup.txt", base.display()),
        ];

        let result = validate_candidates(&base, candidates);
        assert_eq!(result.len(), 1);

        let _ = std::fs::remove_file(&test_file);
    }

    #[test]
    fn extract_clipboard_paths_with_copilot_sample() {
        // This test uses a real file that exists in the project
        let base = PathBuf::from(".");
        if base.join("src/utilities/terminal.rs").exists() {
            let clipboard = "+13-2\n\n`terminal.rs`src/utilities\n\n+13\n\n-2\n\n-";
            let result = extract_clipboard_paths(&base, clipboard);
            assert!(!result.is_empty());
            assert!(result[0].ends_with("terminal.rs"));
        }
    }

    #[test]
    fn extract_clipboard_paths_ignores_metadata() {
        let base = PathBuf::from(".");
        let clipboard = "+13-2\n+13\n-2\n-\n+";
        let result = extract_clipboard_paths(&base, clipboard);
        assert!(result.is_empty());
    }

    #[test]
    fn extract_clipboard_paths_handles_normal_path() {
        let base = PathBuf::from(".");
        if base.join("src/utilities/terminal.rs").exists() {
            let clipboard = "src/utilities/terminal.rs";
            let result = extract_clipboard_paths(&base, clipboard);
            assert!(!result.is_empty());
            assert!(result[0].ends_with("terminal.rs"));
        }
    }

    #[test]
    fn extract_clipboard_paths_handles_windows_separators() {
        let base = PathBuf::from(".");
        if base.join("src/utilities/terminal.rs").exists() {
            let clipboard = "src\\utilities\\terminal.rs";
            let result = extract_clipboard_paths(&base, clipboard);
            assert!(!result.is_empty());
            assert!(result[0].ends_with("terminal.rs"));
        }
    }

    #[test]
    fn validate_candidates_rejects_directories() {
        let base = PathBuf::from(".");
        // "src" is a directory in the project
        if base.join("src").is_dir() {
            let candidates = vec!["src".to_string()];
            let result = validate_candidates(&base, candidates);
            // Directory should be rejected; result should be empty
            assert!(
                result.is_empty(),
                "Directories should not be accepted as candidates"
            );
        }
    }

    #[test]
    fn clipboard_reader_fallback_tries_all_candidates() {
        // Test that the fallback logic continues even if early candidates fail
        let readers = vec!["first", "second", "third"];
        let mut attempt_count = 0;

        let result = try_clipboard_readers_sequence(&readers, |cmd| {
            attempt_count += 1;
            match cmd {
                "first" | "second" => Err("not available".to_string()),
                "third" => Ok("success from third".to_string()),
                _ => Err("unknown".to_string()),
            }
        });

        // Should have tried all three and gotten the result from the third
        assert_eq!(result, Some("success from third".to_string()));
        assert_eq!(attempt_count, 3, "Should have tried all three readers");
    }

    #[test]
    fn clipboard_reader_fallback_returns_none_if_all_fail() {
        // Test that None is returned when all candidates fail
        let readers = vec!["first", "second"];
        let mut attempt_count = 0;

        let result = try_clipboard_readers_sequence(&readers, |_cmd| {
            attempt_count += 1;
            Err("not available".to_string())
        });

        // Should have tried all and gotten None
        assert_eq!(result, None);
        assert_eq!(attempt_count, 2, "Should have tried all readers");
    }

    #[test]
    fn extract_clipboard_paths_mixed_copilot_breadcrumbs_and_ui_noise() {
        // Regression test: Multiple valid Copilot breadcrumbs with diff stats and UI noise
        let base = PathBuf::from(".");
        if base.join("src/utilities/clipboard.rs").exists() && base.join("src/main.rs").exists() {
            let clipboard = r#"+106-33`clipboard.rs`src/utilities
+99
-30

`main.rs`src
+7
-3

.monaco-list.list_id_11:focus .monaco-list-row.focused { background-color: red; }
.monaco-list.list_id_11 .monaco-list-row.selected { color: white; }

-"#;
            let result = extract_clipboard_paths(&base, clipboard);

            // Should extract exactly 2 files, in encounter order
            assert_eq!(result.len(), 2, "Should find exactly 2 valid files");

            // Check that they are the expected files
            let rel1 = result[0].strip_prefix(&base).unwrap();
            let rel2 = result[1].strip_prefix(&base).unwrap();

            assert!(
                rel1.to_string_lossy().contains("clipboard.rs"),
                "First file should be clipboard.rs"
            );
            assert!(
                rel1.to_string_lossy().contains("utilities"),
                "First file should be in utilities"
            );
            assert!(
                rel2.to_string_lossy().contains("main.rs"),
                "Second file should be main.rs"
            );

            // CSS/UI text should never appear in results
            for path in &result {
                let path_str = path.to_string_lossy();
                assert!(
                    !path_str.contains("monaco"),
                    "Monaco CSS should not be in results"
                );
                assert!(
                    !path_str.contains("focus"),
                    "UI styles should not be in results"
                );
            }
        }
    }

    #[test]
    fn normalize_clipboard_text_crlf_newlines() {
        let text = "+99\r\n-30\r\n\r\n`main.rs`src\r\n+7\r\n-3";
        let normalized = normalize_clipboard_text(text);
        // Should convert \r\n to \n
        assert!(!normalized.contains("\r\n"));
        assert!(normalized.contains("\n"));
        // Should still parse correctly
        assert!(normalized.contains("`main.rs`src"));
    }

    #[test]
    fn normalize_clipboard_text_zero_width_space() {
        // U+200B (zero-width space) should be removed
        let text = "+106-33`clip\u{200B}board.rs`src/utilities";
        let normalized = normalize_clipboard_text(text);
        assert!(!normalized.contains("\u{200B}"));
        assert!(normalized.contains("clipboard.rs"));
    }

    #[test]
    fn normalize_clipboard_text_nbsp() {
        // U+00A0 (non-breaking space) should become normal space
        let text = "+106-33`clipboard.rs`src/utilities\u{00A0}";
        let normalized = normalize_clipboard_text(text);
        assert!(normalized.contains(' '));
        assert!(!normalized.contains("\u{00A0}"));
    }

    #[test]
    fn normalize_clipboard_text_unicode_line_separators() {
        // U+2028 (line separator) and U+2029 (paragraph separator) should become \n
        let text = "`clipboard.rs`src/utilities\u{2028}`main.rs`src";
        let normalized = normalize_clipboard_text(text);
        assert_eq!(normalized.matches('\n').count(), 1);
        assert!(!normalized.contains("\u{2028}"));
    }

    #[test]
    fn normalize_clipboard_text_typographic_backticks() {
        // Typographic backticks ' ' should become ASCII `
        let text = "file.rs with \u{2018}quoted\u{2019} text";
        let normalized = normalize_clipboard_text(text);
        assert_eq!(normalized.matches('`').count(), 2);
        assert!(!normalized.contains("\u{2018}"));
        assert!(!normalized.contains("\u{2019}"));
    }

    #[test]
    fn extract_clipboard_paths_survives_mixed_unicode_artifacts() {
        // Test that normalization happens before parsing
        let base = PathBuf::from(".");
        if base.join("src/utilities/clipboard.rs").exists() && base.join("src/main.rs").exists() {
            // Content with CRLF, zero-width spaces, NBSP, and typographic backticks
            let clipboard =
                "+106-33`clipboard.rs`src/utilities\r\n+99\r\n-30\r\n\r\n`main.rs`src\u{00A0}";
            let result = extract_clipboard_paths(&base, clipboard);

            // Should extract both files despite the Unicode artifacts
            assert_eq!(
                result.len(),
                2,
                "Should find both files despite Unicode artifacts"
            );

            let rel0 = result[0].strip_prefix(&base).unwrap().to_string_lossy();
            let rel1 = result[1].strip_prefix(&base).unwrap().to_string_lossy();

            assert!(rel0.contains("clipboard.rs"));
            assert!(rel1.contains("main.rs"));
        }
    }

    #[test]
    fn regression_copilot_css_vs_breadcrumbs() {
        // REGRESSION TEST for the discovered bug:
        // Copilot clipboard can contain multiple representations.
        // First representation: Monaco CSS (0 valid project files)
        // Second representation: Valid breadcrumbs (2 valid project files)
        //
        // BMO must parse both and choose the second because it has more valid files.
        // The bug was that BMO stopped at the first "successful" command output,
        // even if it contained only CSS noise.

        let base = PathBuf::from(".");
        if base.join("src/utilities/clipboard.rs").exists() && base.join("src/main.rs").exists() {
            // First representation: pure Monaco CSS (appears when copying directly from Copilot)
            let css_repr = ".monaco-list.list_id_186:focus .monaco-list-row.focused {
    background-color: red;
}";
            let css_files = extract_clipboard_paths(&base, css_repr);
            assert_eq!(
                css_files.len(),
                0,
                "CSS representation should produce 0 valid files"
            );

            // Second representation: valid Copilot breadcrumbs (appears in other clipboard flavors)
            let breadcrumbs_repr =
                "+157-11\n`clipboard.rs`src/utilities\n+122\n-7\n`main.rs`src\n+35\n-4";
            let breadcrumbs_files = extract_clipboard_paths(&base, breadcrumbs_repr);
            assert_eq!(
                breadcrumbs_files.len(),
                2,
                "Breadcrumb representation should produce 2 valid files"
            );

            // The key: if BMO had chosen CSS (first), it would fail with no files.
            // The bug was treating "command succeeded" as "representation is useful".
            // The fix is to parse both and choose the one with most files.
            assert!(
                breadcrumbs_files.len() > css_files.len(),
                "Breadcrumb representation should win over CSS representation"
            );
        }
    }
}
