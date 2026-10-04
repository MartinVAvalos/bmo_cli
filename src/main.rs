use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

mod utilities;
use utilities::clipboard;
use utilities::file_tools;
use utilities::terminal::{self, Mode};

/// Which command-line shortcut the user invoked.
enum Flag {
    /// No flag: pick files interactively.
    None,
    /// `-last`: reuse the previous selection.
    Last,
    /// `-clip`/`--clip`/`-copilot`/`--copilot`: read from clipboard.
    Clip,
    /// `-clip-debug`/`--clip-debug`: show clipboard contents with invisible characters visible.
    ClipDebug,
    /// `-help`: print usage and exit.
    Help,
    /// An unrecognized option.
    Unknown(String),
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    let (base_path, selected) = match parse_flag(&args) {
        Flag::Help => {
            print_help();
            return;
        }
        Flag::Unknown(opt) => {
            eprintln!("Unknown option: {}", opt);
            eprintln!("Run `bmo -help` to see the available options.");
            return;
        }
        Flag::ClipDebug => {
            clipboard_debug();
            return;
        }
        Flag::Last => match file_tools::load_selection() {
            Some(saved) => saved,
            None => {
                eprintln!("No previous files saved yet — pick some now.");
                new_selection()
            }
        },
        Flag::Clip => match clipboard_selection() {
            Ok(Some(result)) => result,
            Ok(None) => {
                eprintln!("No clipboard reader available.");
                eprintln!("Install wl-clipboard, xclip, or xsel, or ensure pbpaste/Get-Clipboard is available.");
                return;
            }
            Err(msg) => {
                eprintln!("{}", msg);
                return;
            }
        },
        Flag::None => new_selection(),
    };

    if selected.is_empty() {
        eprintln!("Nothing selected — exiting.");
        return;
    }

    let mode = terminal::select_mode();

    let output = match mode {
        Mode::Structure => file_tools::build_structure(&base_path, &selected),
        Mode::Contents => file_tools::build_contents(&base_path, &selected),
        Mode::Both => {
            let mut s = String::from("# File structure\n\n```\n");
            s.push_str(&file_tools::build_structure(&base_path, &selected));
            s.push_str("```\n\n# Files\n\n");
            s.push_str(&file_tools::build_contents(&base_path, &selected));
            s
        }
    };

    // Copy straight to the system clipboard. If no clipboard tool is found
    // (e.g. running headless over SSH), fall back to printing so the output
    // is never lost.
    if copy_to_clipboard(&output) {
        eprintln!("Copied {} bytes to the clipboard.", output.len());
    } else {
        eprintln!("(No clipboard tool found — printing to stdout instead.)");
        print!("{}", output);
    }
}

/// Decide which shortcut (if any) was passed. Only the first argument matters.
fn parse_flag(args: &[String]) -> Flag {
    match args.first().map(|s| s.as_str()) {
        None => Flag::None,
        Some("-last") | Some("--last") => Flag::Last,
        Some("-clip") | Some("--clip") | Some("-copilot") | Some("--copilot") => Flag::Clip,
        Some("-clip-debug") | Some("--clip-debug") => Flag::ClipDebug,
        Some("-help") | Some("--help") | Some("-h") | Some("help") => Flag::Help,
        Some(other) => Flag::Unknown(other.to_string()),
    }
}

/// Usage text. Add new shortcuts here as they're introduced.
fn print_help() {
    println!("bmo — copy code context to your clipboard for feeding to an AI.");
    println!();
    println!("Usage:");
    println!("  bmo            Pick files/folders interactively, then choose a format.");
    println!("  bmo -last      Reuse the previous selection (skips the picker).");
    println!("  bmo -clip      Read a file path from clipboard content such as a Copilot file-change card.");
    println!("  bmo -clip-debug  Show clipboard contents with invisible characters visible (for debugging).");
    println!("  bmo -help      Show this help.");
    println!();
    println!("Run bmo from the root of the project you want to capture.");
    println!("Whatever you produce is copied to your clipboard automatically.");
}

/// Run the interactive picker from the current directory and remember the
/// result so `-last` can reuse it next time.
fn new_selection() -> (PathBuf, Vec<PathBuf>) {
    let base = env::current_dir().expect("could not read the current directory");
    let picked = terminal::select_paths(&base);
    file_tools::save_selection(&base, &picked);
    (base, picked)
}

/// Read file path from clipboard. Returns the base path and selected files,
/// or an error message if parsing fails.
fn clipboard_selection() -> Result<Option<(PathBuf, Vec<PathBuf>)>, String> {
    let base =
        env::current_dir().map_err(|e| format!("Could not read current directory: {}", e))?;

    // Try multiple clipboard representations and choose the one with most valid files
    let representations = clipboard::read_clipboard_candidates();

    if representations.is_empty() {
        return Ok(None);
    }

    let mut best_files: Vec<PathBuf> = Vec::new();

    for text in representations {
        let files = clipboard::extract_clipboard_paths(&base, &text);
        if files.len() > best_files.len() {
            best_files = files;
        }
    }

    if best_files.is_empty() {
        return Err("No valid project files found in clipboard.".to_string());
    }

    // Display all found files
    if best_files.len() == 1 {
        let display_path = best_files[0].strip_prefix(&base).unwrap_or(&best_files[0]);
        eprintln!("Found:");
        eprintln!("  {}", display_path.display());
    } else {
        eprintln!("Found {} files:", best_files.len());
        for path in &best_files {
            let display_path = path.strip_prefix(&base).unwrap_or(path);
            eprintln!("  {}", display_path.display());
        }
    }

    file_tools::save_selection(&base, &best_files);
    Ok(Some((base, best_files)))
}

/// Display clipboard contents with invisible characters visible for debugging.
/// Shows multiple representations and how many valid files each produces.
fn clipboard_debug() {
    let representations = clipboard::read_clipboard_candidates();

    if representations.is_empty() {
        eprintln!("No clipboard reader available.");
        eprintln!(
            "Install wl-clipboard, xclip, or xsel, or ensure pbpaste/Get-Clipboard is available."
        );
        return;
    }

    let base = match env::current_dir() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Could not read current directory: {}", e);
            return;
        }
    };

    eprintln!("Clipboard representations:\n");

    let strategies = if cfg!(target_os = "macos") {
        vec!["pbpaste -Prefer txt", "osascript text", "pbpaste default"]
    } else if cfg!(target_os = "windows") {
        vec!["Get-Clipboard"]
    } else {
        vec!["wl-paste / xclip / xsel"]
    };

    for (i, text) in representations.iter().enumerate() {
        let strategy = strategies.get(i).copied().unwrap_or("unknown");
        let files = clipboard::extract_clipboard_paths(&base, text);

        // Show a short escaped preview (first 80 chars)
        let preview = if text.len() > 80 {
            format!("{:?}...", &text[..80])
        } else {
            format!("{:?}", text)
        };

        eprintln!("[{}] {}", i + 1, strategy);
        eprintln!("    length: {} bytes", text.len());
        eprintln!("    preview: {}", preview);
        eprintln!("    valid project files: {}", files.len());

        for file in &files {
            if let Ok(rel) = file.strip_prefix(&base) {
                eprintln!("      - {}", rel.display());
            }
        }

        eprintln!();
    }
}

/// Pipe `text` into the platform's native clipboard command.
/// Returns true once one of them accepts the input successfully.
fn copy_to_clipboard(text: &str) -> bool {
    // Each entry is (command, args); we try them in order until one works.
    let candidates: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("pbcopy", &[])]
    } else if cfg!(target_os = "windows") {
        &[("clip", &[])]
    } else {
        &[
            ("wl-copy", &[]),
            ("xclip", &["-selection", "clipboard"]),
            ("xsel", &["--clipboard", "--input"]),
        ]
    };

    for (command, args) in candidates {
        let spawned = Command::new(command)
            .args(*args)
            .stdin(Stdio::piped())
            .spawn();

        let mut child = match spawned {
            Ok(child) => child,
            Err(_) => continue, // command not installed — try the next one
        };

        // Write into the command's stdin, then close it by dropping the handle.
        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(text.as_bytes()).is_err() {
                continue;
            }
        }

        if child.wait().map(|status| status.success()).unwrap_or(false) {
            return true;
        }
    }

    false
}
