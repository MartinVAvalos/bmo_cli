# bmo

**bmo** copies code context to your clipboard so you can paste it straight into an AI.

Run `bmo` from your project root, pick files interactively, choose a format (structure, contents, or both), and the result lands on your clipboard—ready to paste into Copilot, ChatGPT, Claude, or any other AI assistant.

## Installation

### Rust (recommended)

```sh
git clone https://github.com/MartinVAvalos/bmo_cli.git
cd bmo_cli
make install          # or: cargo install --path . --locked
```

Ensure `~/.cargo/bin` (or wherever `cargo install` puts binaries) is on your `PATH`.

### Docker (no Rust required)

```sh
make docker-install   # wraps docker run; clipboard output prints to stdout
make docker-extract   # Linux only – installs a native static binary with clipboard support
```

## Usage

```sh
bmo              # Pick files/folders interactively, then choose a format
bmo -last        # Reuse the previous selection (skips the picker)
bmo -clip        # Read a file path from clipboard (e.g. a Copilot file-change card)
bmo -clip-debug  # Show clipboard contents with invisible characters visible
bmo -help        # Show help
```

Run `bmo` from the root of the project you want to capture. Output is copied to your clipboard automatically; if no clipboard tool is found it falls back to stdout.

## Example output

Selecting `src/main.rs` with the **Contents** format produces:

````
// @File: src/main.rs
```rust
fn main() { … }
```
````

Selecting **Structure** produces a nested-brace tree:

```
{
  src {
    main.rs,
    utilities {
      clipboard.rs,
      file_tools.rs,
      terminal.rs
    }
  }
}
```

## What gets excluded

bmo never exports the following, even if selected:

| Category | Entries |
|----------|---------|
| Version control | `.git`, `.github` |
| Build output | `target`, `dist`, `build` |
| Dependencies | `node_modules` |
| IDE state | `.idea`, `.vscode` |
| Secrets | `.env`, `.env.*` (e.g. `.env.local`), `*.env` (e.g. `vm_back.env`) |
| macOS metadata | `.DS_Store` |

## Testing

```sh
cargo test
```

## Packaging a clean archive

```sh
make package        # produces bmo_cli.tar.gz from committed files only
```

`git archive` archives only **committed** files, so files that are untracked
(e.g. `target/` artifacts built locally) are automatically omitted. Any file
that is tracked in git—even if listed in `.gitignore`—will still be included.
Keep secrets untracked (`git rm --cached .env`) to ensure they stay out of the
archive.

## Picker controls

### Normal mode

| Key | Action |
|-----|--------|
| ↑ / ↓ | Move up / down |
| ← | Jump 10 entries |
| → | Reveal folder contents |
| Space | Select / deselect |
| Enter | Confirm selection |
| Esc | Cancel |

### Search mode

Press `i` to enter search mode for quick file/folder filtering.

| Key | Action |
|-----|--------|
| `i` | Enter search mode |
| Type | Filter by path (case-insensitive) |
| Backspace | Remove last character |
| ↑ / ↓ | Move one result |
| ← / → | Jump 10 results |
| Space | Select / deselect a result |
| Esc | Return to normal mode |

Selections made while searching persist after exiting search mode.

