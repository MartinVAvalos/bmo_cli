use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

mod utilities;
use utilities::terminal::{self, Mode};
use utilities::file_tools;

/// Which command-line shortcut the user invoked.
enum Flag {
    /// No flag: pick files interactively.
    None,
    /// `-last`: reuse the previous selection.
    Last,
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
        Flag::Last => match file_tools::load_selection() {
            Some(saved) => saved,
            None => {
                eprintln!("No previous files saved yet — pick some now.");
                new_selection()
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
        Some("-help") | Some("--help") | Some("-h") | Some("help") => Flag::Help,
        Some(other) => Flag::Unknown(other.to_string()),
    }
}

/// Usage text. Add new shortcuts here as they're introduced.
fn print_help() {
    println!("bmo — copy code context to your clipboard for feeding to an AI.");
    println!();
    println!("Usage:");
    println!("  bmo          Pick files/folders interactively, then choose a format.");
    println!("  bmo -last    Reuse the previous selection (skips the picker).");
    println!("  bmo -help    Show this help.");
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
