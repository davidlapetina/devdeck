# DevDeck

DevDeck is a terminal workspace for repository browsing and command-line development. It gives you a live file explorer, source/Markdown previews, and configurable PTY-backed command tabs in one focused TUI.

DevDeck runs inside your terminal emulator. It is not a terminal multiplexer, shell replacement, or editor.

## Screenshots

Repository browser with file tree and preview:

![DevDeck Files tab showing the repository browser and preview pane](docs/assets/devdeck-files-tab.png)

Interactive Git tab running `lazygit` inside DevDeck:

![DevDeck Git tab running lazygit in a PTY-backed terminal session](docs/assets/devdeck-git-tab.png)

## Features

- Repository tree navigation with hidden-file toggle and configurable generated-directory filtering
- Plain text, source-code, binary metadata, and rendered Markdown previews with link navigation
- Independent preview scrolling and automatic refresh on filesystem changes
- Filename search
- File and folder actions for rename, filename copy, relative/absolute path copy, command launch, and agent launch
- External file opening
- Temporary in-DevDeck editor tabs
- Configurable terminal tabs for arbitrary commands
- Interactive PTY sessions with ANSI colors, cursor movement, alternate screens, and terminal resizing
- Lazy-start and auto-start terminal profiles
- Background activity markers
- Restart, stop, close, and process-exit states
- Temporary command tabs
- Configuration reload without killing running sessions
- Help, rename, confirmation, and prompt overlays

Claude Code, Codex, shells, Git tools, and editors are all just configured commands. DevDeck does not hardcode special behavior for any of them.

## Release Notes

### 0.20.1

- Fixed `Shift+Tab` in running terminal tabs by forwarding the reverse-tab escape sequence to the child process.

### 0.2.0

- Added a file actions overlay on `a` for selected files and folders.
- Added file/folder rename from the Files tab.
- Added filename, relative path, and absolute path copy actions.
- Added command launch with the selected path appended as the final argument.
- Added Codex/Claude agent launch from configured profiles with a prompt scoped to the selected path.
- Fixed multiline paste in terminal tabs by forwarding supported pastes as bracketed paste blocks.

## Install From Source

Install Rust stable, then build DevDeck:

```bash
. "$HOME/.cargo/env"
cargo build --release
```

The binary is created at:

```bash
./target/release/devdeck
```

Run the test suite:

```bash
cargo test
```

Run from the checkout:

```bash
cargo run -- .
```

## Usage

```bash
devdeck
devdeck .
devdeck /path/to/project
devdeck . --hidden
devdeck . --no-watch
```

If no path is supplied, DevDeck opens the current working directory.

## Configuration

DevDeck loads two TOML files:

1. `~/.config/devdeck/config.toml`
2. `.devdeck.toml` in the repository root

The global file is loaded first. The project file is loaded second. Project profiles replace global profiles with the same `name`, project-only profiles are appended, and global-only profiles remain available.

`Files` is reserved for the repository browser tab and cannot be used as a terminal profile name.

Example:

```toml
version = 1

[workspace]
default_tab = "Files"
# Directory names omitted from the file tree. Set to [] to disable this filter.
ignored_directories = [".git", "target", "node_modules", ".dart_tool", "dist", "coverage", ".idea"]

[[tabs]]
name = "Claude"
command = "claude"
auto_start = false

[[tabs]]
name = "Codex"
command = "codex"
auto_start = false

[[tabs]]
name = "Shell"
command = "${SHELL}"
auto_start = true

[[tabs]]
name = "Git"
command = "lazygit"
auto_start = false
```

The Git example uses `lazygit`. DevDeck does not bundle `lazygit`; install it separately and make sure it is on `PATH` before starting DevDeck:

```bash
command -v lazygit
```

If you install `lazygit` while DevDeck is already running, restart DevDeck from a shell that can find `lazygit`, or retry the Git tab after confirming the inherited `PATH` is correct.

Each terminal profile supports:

```toml
name = "Shell"
command = "${SHELL}"
args = []
cwd = "."
env = { RUST_BACKTRACE = "1" }
auto_start = false
restart_on_exit = false
```

Expansion rules:

- `~` and environment variables are expanded in `command`, `args`, `cwd`, and environment values.
- Relative `cwd` values resolve against the repository root.
- `workspace.ignored_directories` matches directory names case-insensitively. Project config replaces the global list when set.
- Claude and Codex profiles without an explicit `cwd` start in the current file-browser folder. If a file is selected, they start in that file's parent directory. Set `cwd` to pin them to a fixed directory.
- Commands launch as executable plus argument vector, not through an implicit shell.
- If a configured command is missing, its tab shows `Executable not found: <command>` and DevDeck keeps running.

A copyable sample is available at [examples/devdeck.toml](examples/devdeck.toml).

## Key Bindings

Files tab:

```text
j / Down       Move down
k / Up         Move up
l / Right      Expand or enter
h / Left       Collapse or go to parent
Enter          Open or expand selected entry
g              First entry
G              Last entry
Home           Repository root
.              Toggle hidden files
/              Filename search
m              Toggle rendered/raw Markdown
Ctrl-d         Preview page down
Ctrl-u         Preview page up
J / K          Preview line down/up
0 / $          Preview top/bottom
] / [          Next/previous Markdown preview link
y / Y          Copy relative/absolute path
a              File actions: rename, copy name/path, run command, run configured agent
e              Open externally
v              Open selected file in a temporary editor tab
r / R          Reload file/tree
1..9           Select tab by position
Tab / BackTab  Next/previous tab
c              Create temporary command tab from a known command, shell, or custom command
?              Help overlay
q              Quit with confirmation
```

When a terminal tab is running, normal keyboard input goes directly to the child process. Use the command prefix for DevDeck commands that would otherwise be typed into Claude, Codex, a shell, Vim, Less, or another terminal program.

Command prefix:

```text
Ctrl-b 1..9     Select tab by position
Ctrl-b n        Next tab
Ctrl-b p        Previous tab
Ctrl-b f        Select Files tab
Ctrl-b c        Create temporary command tab from a known command, shell, or custom command
Ctrl-b x        Stop or close current terminal tab
Ctrl-b r        Restart current terminal tab
Ctrl-b e        Reload configuration
Ctrl-b q        Quit with confirmation
Ctrl-b ?        Help overlay
Ctrl-b ,        Rename temporary tab
Ctrl-b Ctrl-b   Send literal Ctrl-b to the child process
```

Inactive terminal tabs also accept direct commands:

```text
Enter / r       Start or restart an exited/failed terminal tab
x               Close a temporary tab or reset a configured tab
1..9            Select tab by position
Tab / BackTab   Next/previous tab
?               Help overlay
q               Quit with confirmation
```

Tab activity markers:

```text
>               Terminal shell emitted an OSC 133 prompt marker and is waiting at a prompt
*               Inactive terminal tab has recent output
.               Inactive terminal tab produced output, then went quiet
!               Terminal process exited or failed
```

New tab launcher:

```text
Up / Down       Choose configured command, new shell, or custom command
Tab / BackTab   Move between source, name, command, and cwd fields
Enter           Launch temporary tab
Esc             Cancel
```

File actions:

```text
a               Open file actions for the selected file or folder
r               Rename selected file or folder
n               Copy selected file/folder name
y / Y           Copy relative/absolute path
!               Run a command with the selected path appended as the final argument
g               Run a configured Codex or Claude agent with a prompt for the selected path
v               Open selected file in a temporary editor tab
Esc             Cancel
```

Prompt overlay:

```text
Ctrl-g          Open prompt overlay for the active running terminal
Enter           Send text plus newline
Alt-Enter       Insert newline
Esc             Cancel
```

When a terminal tab is active, normal keyboard input is sent directly to the running process.

## Editor Behavior

The `e` command opens the selected file outside DevDeck and temporarily restores the terminal while the editor runs.

The `v` command opens the selected file inside DevDeck as a temporary terminal tab. The editor resolves in this order:

1. `$VISUAL`
2. `$EDITOR`
3. `vi`

To use Vim:

```bash
export EDITOR=vim
export VISUAL=vim
devdeck .
```

When an in-DevDeck editor tab exits, focus returns to the Files tab. The exited editor tab remains visible and can be closed with `x` when selected or with `Ctrl-b x` from any running terminal tab.

## Configuration Reload

Use `Ctrl-b e` to reload configuration.

Reload behavior:

- New configured tabs are added.
- Not-started and exited configured tabs are updated in place.
- Changed running configured tabs keep running and are marked `restart required`.
- Removed running configured tabs stay visible and are marked `removed from config`.
- Removed non-running configured tabs are removed.
- Temporary tabs are preserved.

## License

MIT. See [LICENSE](LICENSE).
