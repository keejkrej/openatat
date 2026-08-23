# OpenAtat

Open clone of [Atat](https://atatapp.com): type `@@` in any text field. OpenAtat gathers what you are looking at, runs the CLI agent you already have, and puts the answer back in the line you were typing. It never becomes the active app.

This repository is the Linux/Omarchy spike plus real macOS and Windows `@@` paths. Product contract, capture inventory C1–C19, OS API matrix, budgets, permissions, ship order, and landmines: **[SPEC.md](SPEC.md)**. Product facts follow Atat’s [manual](https://atatapp.com/manual) and [FAQ](https://atatapp.com/faq).

Apache-2.0. No bundled LLM.

## Native applet (locked)

| Process | Role |
| --- | --- |
| `openatatd` | Always-on native applet. Owns the `@@` overlay, **Orb**, C10 selection bar, trigger, insert, capture, terminal handoff, and the **C14 clipboard watch + shelf**. Idle maps the **Orb** (not the overlay / shelf) and starts **no** GPU window. |
| `openatat-ui` | gpui-ce, **on demand**. Settings + history + **C17 studio / annotation**. First-run later. Quit when the last window closes. The GPU crate is feature-gated (`--features gpui`) so `cargo test --workspace` does not pull a GPU stack. |

P0 does **not** use gpui for the overlay. gpui-ce 0.3 has LayerShell / PopUp / Transparent / `focus: false`, but that is not a nonactivating panel.

- **Omarchy / Hyprland:** native `zwlr_layer_shell_v1` surfaces inside `openatatd`, software `wl_shm`. The **Orb** is mapped at idle (`KeyboardInteractivity::None`, input region = the circle). The overlay / selection bar / clipboard shelf use `OnDemand` only while up. None of them is a gpui window.
- **macOS:** `NSPanel` + `NSWindowStyleMaskNonactivatingPanel` (`canJoinAllSpaces`, `fullScreenAuxiliary`) for the overlay **and** the Orb. Never becomes the active app. `NSStatusItem` **Show Orb** toggle (this launch). Trigger is a listen-only event tap feeding `ImeFilter` — not a global summon hotkey.
- **Windows:** `WS_EX_NOACTIVATE | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED`. Never calls `SetForegroundWindow`. Trigger is a process-local keyboard hook (Raw Input fallback) feeding `ImeFilter` — not a global summon hotkey.

Omarchy 4 is Hyprland + Quickshell (Waybar is gone). Do not add Waybar modules. The Omarchy bar chip in `omarchy/openatat/` (`openatat.chip`) is **not** the Orb and must not grow into one.

## How to run the Linux spike (Hyprland)

Build:

```bash
cargo build
```

One-shot demo (opens the native popover if `WAYLAND_DISPLAY` is set):

```bash
cargo run -p openatatd -- --demo
```

Type a prompt, `Return` runs the BYO CLI (first provider on `PATH`, or `echo` if none), review the **preview card**, `Tab` copies then tries AT-SPI insert, `R` refines with one more sentence, `Super+Return` (or the **Handoff** button on the preview card) opens your terminal with an interactive agent session already loaded, `Esc` cancels. Click **remove** to drop the C1 still tile. After a successful handoff the overlay dismisses.

Headless (CI / no compositor):

```bash
cargo run -p openatatd -- --headless --prompt='make this friendlier'
```

Always-on daemon + dev trigger (not a product hotkey):

```bash
# terminal A
cargo run -p openatatd

# terminal B — unix socket, IME-filter shaped; do not bind this to a global key as the product
cargo run -p openatatd -- trigger
```

The daemon listens on `$XDG_RUNTIME_DIR/openatat/trigger.sock`. The Fcitx5 addon sends the same JSON (`source: ime`). A Hyprland bind is not the product path — see SPEC.md §6.

### The Orb (Omarchy)

`openatatd` maps a small round `@—@` avatar on the focused output. It never becomes the active app (layer-shell Overlay, exclusive zone 0, keyboard **none**). Eyes follow the pointer (`hyprctl cursorpos` plus hover).

```bash
# terminal A — Orb is mapped at idle
cargo run -p openatatd

# click the Orb → empty @@ prompt (no C1 still, no attachments)
# type @@ in a text field → still auto-attaches C1 (grim)
# drag a file / image / text onto the Orb → those become tiles (C16)
# drag the Orb itself to move it (position persisted per output)
# right-click the Orb, or:
cargo run -p openatatd -- hide-orb
# hide lasts this launch only; @@ / selection bar still work
# cargo run -p openatatd -- show-orb
```

While the agent runs, the Orb shows a small bubble and **Esc** on the overlay is the only cancel — a stray click on the Orb does not dismiss work. An error becomes a wider pill with an Esc button.

Headless empty Orb session (no still):

```bash
cargo run -p openatatd -- --headless --orb-click --prompt='hello from the orb'
```

Presence for the Omarchy bar chip (does not summon `@@`):

```bash
# same socket; also written to $XDG_RUNTIME_DIR/openatat/status.json
# {"cmd":"status"}  →  {"status":"idle"} | {"status":"busy"} | {"status":"error","message":"…"}
cargo run -p openatatd -- status
```

### Selection bar (C10)

After a **mouse-up** in a text field, if AT-SPI reports selected text, a compact nonactivating bar appears next to the selection (extents if present, otherwise near the pointer): **Ask @@**, **Copy**, **Search**, **Summarize**, **Explain**. Keyboard selections (shift+arrow) do not summon it. Password fields are re-probed every time and never read.

Ask / Summarize / Explain run the existing agent + **preview card**. `Tab` replaces the selection (clipboard-first, then Hyprland window-address check, then AT-SPI `DeleteText` + `InsertText`). Copy and Search run immediately. Selected text is ephemeral: used for the action, never stored in history, never logged.

If AT-SPI cannot expose a selection (common in browsers / Electron), OpenAtat **skips** — it does not steal the clipboard to guess. An app exclude list can wait on Settings.

Dev probe (treat the current AT-SPI selection as a mouse-up; not a hotkey):

```bash
# terminal A: openatatd
# terminal B
cargo run -p openatatd -- selection
```

Headless (no display; selected text is not written to history):

```bash
cargo run -p openatatd -- --headless --selection='some text' --action=summarize
```

### Clipboard history shelf (C14)

A shelf of everything you copied (plain + html/rtf when present). Searchable. Return pastes (clipboard-first write, then insert if a text field is still focused — same abort-on-focus-change as Tab). Super+Return / `⌘Return` / Win+Return hands the clip to the existing BYO CLI overlay as a tile (no auto C1). Esc dismisses. Passwords never enter the shelf (secure field re-probed on every copy). History never leaves the machine. The file is `~/.local/share/openatat/clipboard-shelf.json` — **not** `history.jsonl`.

This is a **clipboard shortcut**, not a `@@` summon. The daemon does **not** install a Hyprland bind and does not steal screenshot keys.

```bash
# terminal A: openatatd
# terminal B
cargo run -p openatatd -- --shelf
# or IPC: {"cmd":"shelf"}
```

Omarchy / Hyprland — add this yourself (the applet will not write it):

```ini
# ~/.config/hypr/hyprland.conf
bind = SUPER SHIFT, V, exec, openatatd --shelf
```

macOS: `⌘⇧V` if Input Monitoring is already granted, else `--shelf`. Windows: `Win+Shift+V` on the process-local hook, else `--shelf`.

Turn recording off without clearing old items:

```toml
# ~/.config/openatat/agent.toml
[clipboard]
shelf = false
```

Default is **on**. Settings has the same switch. Overlay / Orb / studio are unchanged.

### Area and display stills (C2 / C4)

Atat `⌘⇧4` (area) and `⌘⇧3` (display). A successful still opens the existing `@@` overlay with **that tile only** — no second auto C1. Esc on the picker cancels without opening `@@`. Typed `@@` still auto-attaches C1. Nothing is uploaded.

The picker is a native nonactivating surface in `openatatd` (layer-shell Overlay, dimmed rubber-band, `KeyboardInteractivity::OnDemand` only while picking). It is not gpui, not slurp, not an xdg-desktop-portal Screenshot chooser. After a rect is chosen, grim crops that geometry (or the full output, then a CPU crop). `slurp` is a documented fallback only if the native picker cannot map, like `--demo` for trigger.

This is a **capture shortcut**, not a `@@` summon. The daemon does **not** install a Hyprland bind and does not steal OS screenshot keys behind your back.

```bash
# terminal A: openatatd
# terminal B
cargo run -p openatatd -- --capture area
cargo run -p openatatd -- --capture display
# or IPC: {"cmd":"capture","kind":"area"}
#         {"cmd":"capture","kind":"display"}
```

Omarchy / Hyprland — add this yourself (the applet will not write it):

```ini
# ~/.config/hypr/hyprland.conf
bind = SUPER SHIFT, 4, exec, openatatd --capture area
bind = SUPER SHIFT, 3, exec, openatatd --capture display
```

macOS: `⌘⇧3` / `⌘⇧4` if Input Monitoring is already granted, else `--capture`. Windows: `Win+Shift+3/4` on the process-local hook, else `--capture`.

## Product `@@` trigger (Fcitx5)

This is the real path: type `@@` in any text field. Requires Fcitx5 and the
`fcitx5-openatat` module. Copy-paste on Omarchy / Arch:

```bash
sudo pacman -S --needed fcitx5 fcitx5-gtk fcitx5-qt extra-cmake-modules cmake ninja pkgconf gcc

cd ime/fcitx5-openatat
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build
./build/openatat-filter-test          # no display; no Fcitx5 session
sudo cmake --install build            # only if CMake found Fcitx5Core
fcitx5 -r
```

Then run `openatatd` and type `@@` in a text field. The two `@` characters are
swallowed and the overlay opens. CJK preedit must not fire. Password fields are
probed every key (never cached). If the daemon socket is missing, the addon
fails quietly.

Enable (usually already on): `fcitx5-configtool` → Addons → OpenAtat.

User-local install: `-DCMAKE_INSTALL_PREFIX=$HOME/.local` and skip `sudo`.
If CMake warns that Fcitx5 headers are missing, the addon was **not** linked —
see `ime/fcitx5-openatat/README.md`.

## BYO CLI (P1)

OpenAtat does not ship a model. It launches a CLI you already have. Default is the **first** of these binaries found on `PATH`:

| Provider | Binary | Default argv (prompt is data, not a shell string) |
| --- | --- | --- |
| Claude Code | `claude` | `claude --print --permission-mode plan {prompt}` |
| Codex | `codex` | `codex exec --sandbox read-only --ephemeral -` (prompt on stdin) |
| Grok | `grok` | `grok --sandbox read-only --prompt-file {prompt_file}` |
| Cursor | `cursor-agent` or `agent` | `… --print --mode ask --trust {prompt}` |
| Pi | `pi` | `pi --print {prompt}` |
| Hermes | `hermes` | `hermes -z {prompt}` |
| OpenCode | `opencode` | `opencode run {prompt}` |

If none are installed, `openatatd` logs that no provider is installed and runs `echo` (dummy). `{prompt}` is one argv element; `{prompt_file}` is a temp file OpenAtat writes. Quotes and newlines stay intact.

Pick or override in `~/.config/openatat/agent.toml` (XDG):

```toml
# auto | claude | codex | grok | cursor | pi | hermes | opencode | custom | dummy
provider = "auto"

# Optional argv override (not a shell line):
# argv = ["claude", "--print", "--permission-mode", "plan", "{prompt}"]

# [handoff]
# terminal = "kitty"

# [clipboard]
# shelf = true   # default on. false stops recording; existing items stay.
```

`OPENATAT_AGENT` remains an escape hatch: that binary is exec’d directly and the prompt is written to stdin (still not interpolated into a shell).

Every run uses a **scratch workspace** under `~/.cache/openatat/scratch/<id>/`. cwd is set there. Quick answers never run in the folder you happened to have focused — file-manager tiles are not implemented, so it is always scratch. If launch fails, the prompt is copied to the clipboard before the error is shown.

### Handoff (Super+Return)

Some work should not end in a text snippet. `Super+Return` (Atat `⌘Return`) or the **Handoff** button on the preview card opens a real interactive session in your terminal with the gathered prompt (and still tile, if present) already loaded. OpenAtat then gets out of the way.

- **cwd** is always a new scratch dir (`~/.cache/openatat/scratch/<id>/`). Finder tiles (Mac Automation) are attachments only — the window title is never used as a project folder. Nautilus tiles are not implemented.
- **Terminal** is the first of these on `PATH`: `ghostty`, `kitty`, `alacritty`, `wezterm`, `foot`, `gnome-terminal`, `xterm`. Override in `~/.config/openatat/agent.toml`:

```toml
[handoff]
terminal = "kitty"
```

Flags come from each emulator's docs (`ghostty --working-directory=DIR -e …`, `kitty --directory DIR`, `alacritty --working-directory DIR -e`, `wezterm start --cwd DIR --`, `foot -D DIR`, `gnome-terminal --working-directory=DIR --`, `xterm -e` with cwd set on the process). The prompt is never interpolated into `sh -c`. Hyprland `hyprctl dispatch exec` is a last resort and is refused if the prompt would appear in that command.
- **Interactive CLI** (not `--print` / plan-mode / one-shot): `claude {prompt}`, `codex {prompt}`, `cursor-agent {prompt}`, `pi {prompt}`, `opencode --prompt {prompt}`. Grok and Hermes have no documented TUI-preload flag — the terminal opens in scratch on `grok` / `hermes` with `prompt.txt` already written. Dummy (`echo`) opens the terminal in scratch only.
- If launch fails, the prompt is copied to the clipboard **before** the error is shown.

macOS handoff uses `NSWorkspace.openApplication` with `OpenConfiguration` (`arguments`, `currentDirectoryURL`) and the same scratch cwd. The prompt is never spliced into `open -a` / `sh -c`. Windows handoff uses `CreateProcessW` with `lpCurrentDirectory` = scratch, preferring Windows Terminal (`wt.exe -d <scratch> -- <cli>`). Never `cmd.exe /c`.

A machine with `claude` on `PATH`:

```bash
cargo run -p openatatd -- --headless --prompt='make this friendlier'
```

invokes `claude --print --permission-mode plan …` in that scratch cwd.

Optional env:

| Variable | Meaning |
| --- | --- |
| `OPENATAT_AGENT` | Escape-hatch binary. Prompt on stdin (never a shell string). |
| `OPENATAT_PROMPT` | Headless / `--demo` default prompt. |
| `OPENATAT_INSERT` | Headless path also runs clipboard + insert. |
| `OPENATAT_DEMO` | Same as `--demo`. |

### Settings + History + Studio (`openatat-ui`)

`openatat-ui` is a **separate** gpui-ce process. Activating it is fine — this is not the `@@` overlay. It exits when the last window closes.

```bash
# Real window (needs gpui-ce + Linux GPU/Wayland headers; see below)
cargo run -p openatat-ui --features gpui -- --settings
cargo run -p openatat-ui --features gpui -- --history
cargo run -p openatat-ui --features gpui -- --studio --image some.png

# From a running build: daemon just execs the sibling binary
cargo build -p openatat-ui --features gpui
cargo run -p openatatd -- --settings    # or --history or --studio --image=some.png
# IPC (newline JSON on $XDG_RUNTIME_DIR/openatat/trigger.sock):
# {"cmd":"open-ui","page":"settings"}
# {"cmd":"open-ui","page":"history"}
# {"cmd":"open-ui","page":"studio","image":"/path/to/shot.png"}
```

Settings writes `~/.config/openatat/agent.toml` (provider + argv list). Unknown keys and comments are kept. History reads `~/.local/share/openatat/history.jsonl` newest first; **Reuse** copies/prints the prompt only. Clear History asks for a second click. Screenshots and agent output are not in the file and are never shown.

**Studio (C17)** annotates a **local** PNG or JPEG: arrows, shapes, freehand, highlighter, text, step counters, blur / pixelate / spotlight, crop. Edits stay individually undoable until **Export**, which writes `<stem>-annotated.png` next to the source (or a working copy under `~/.cache/openatat/studio/` if that directory is not writable). Nothing is uploaded. Tools → Open Annotate… in Settings takes a local path. Overlay still tiles have **Edit**: that only spawns `openatat-ui --studio --image <still>`. If `openatat-ui` is missing or was built without `gpui`, the applet logs the same rebuild hint Settings already uses and does **not** open a gpui window. If the @@ overlay is still up after export, the next click or keystroke on the popover reloads the annotated sibling into the tile. Otherwise fire `@@` again, or drop the annotated PNG on the Orb. Video trim / export (C18) is not built.

The Permissions tab names what each optional Linux grant unlocks (grim, hyprctl, clipboard, AT-SPI, Fcitx5). It does **not** request OS permissions.

`cargo test --workspace` builds `openatat-ui` **without** gpui-ce. A binary built that way exits 2 and tells you to rebuild with `--features gpui`.

gpui-ce 0.3 on Linux needs compile-time headers that this cloud VM often lacks: `libxkbcommon-dev`, `libwayland-dev`, `libvulkan-dev`, and usually `libfontconfig-dev`. Runtime still needs Vulkan + a Wayland or X11 display. Missing those libs is a real build gap — do not treat a header-less compile as success.

### Omarchy 4 Quickshell bar chip

Omarchy 4’s shell is Quickshell (`omarchy-shell`). The presence chip is a third-party **bar-widget** plugin, not a Waybar module. Copy-paste on Omarchy:

```bash
# from a clone of this repo
mkdir -p ~/.config/omarchy/plugins
rm -rf ~/.config/omarchy/plugins/openatat.chip
cp -a omarchy/openatat ~/.config/omarchy/plugins/openatat.chip

omarchy-shell shell rescanPlugins
omarchy plugin enable openatat.chip
omarchy-restart-shell
```

Or add `{ "id": "openatat.chip" }` to `bar.layout.right` in `~/.config/omarchy/shell.json`, then `omarchy-restart-shell`. Do **not** install Waybar. `omarchy plugin add` against this git URL will not work — that command wants `manifest.json` at the repository root.

The chip watches `$XDG_RUNTIME_DIR/openatat/status.json` (`idle` / `busy` / `error`). Click sends `{"cmd":"open-ui","page":"settings"}` on `trigger.sock`. That must not summon `@@`. If `openatat-ui` was built without gpui, the click is a no-op.

Full notes: [`omarchy/openatat/README.md`](omarchy/openatat/README.md).

## How to run on macOS

Build (needs the Apple SDK — this Linux CI image does not have it):

```bash
cargo build -p openatatd
```

Always-on daemon (accessory `NSApplication`, no Dock bounce):

```bash
cargo run -p openatatd
```

The Orb is a second nonactivating `NSPanel` (circle hit-test, never the active app). Click it for an empty prompt (no C1 still). Drag files / images / text onto it for tiles. **Show Orb** in the `@@` menu extra hides it for this launch only. Typed `@@` still auto-attaches C1.

Type `@@` in a text field (Input Monitoring + Accessibility). The two `@` characters are swallowed via AX replace (never in a password field) and an `NSPanel` nonactivating overlay opens. `Esc` cancels. `Tab` copies to `NSPasteboard` then AX-inserts if the frontmost app + focused element still match. `⌘Return` hands off to Terminal / iTerm / Ghostty / kitty in a scratch cwd.

Without Input Monitoring, the listen-only tap is idle. `--demo` and the unix socket still work:

```bash
# terminal A
cargo run -p openatatd

# terminal B
cargo run -p openatatd -- trigger
# or
cargo run -p openatatd -- --demo
```

Headless (no NSPanel):

```bash
cargo run -p openatatd -- --headless --prompt='make this friendlier'
```

This cloud / Linux agent **cannot** `cargo build --target aarch64-apple-darwin` — the macOS SDK is not installed. That is a real gap, not a successful cross-compile.

## How to run on Windows

Needs a native MSVC toolchain (`x86_64-pc-windows-msvc`) and the Windows 10+ SDK. This Linux CI image does **not** have that toolchain — do not treat a missing cross-compile as success.

```bat
cargo build -p openatatd
cargo run -p openatatd
```

The Orb is a second `WS_EX_NOACTIVATE | LAYERED` window (circle hit region). Click it for an empty prompt (no C1). Drag files onto it (`DragAcceptFiles`). Right-click or `openatatd hide-orb` hides it for this launch. Tray comes later. Never `SetForegroundWindow`.

Type `@@` in a text field. The process-local keyboard hook feeds `ImeFilter` (UIA `IsPassword` / Win32 `ES_PASSWORD` re-probed every key; IME composition ignored). The two `@` characters are swallowed via UIA ValuePattern replace (never into a password field) and a `WS_EX_NOACTIVATE` overlay opens. OpenAtat never calls `SetForegroundWindow`. `Esc` cancels. `Tab` writes the clipboard first, re-checks `GetForegroundWindow` + UIA RuntimeId, then ValuePattern / a single Ctrl+V. Browsers paste; they are not typed per-key. `Win+Return` (or the **Handoff** button) opens Windows Terminal (`wt.exe -d <scratch> -- <cli>`) with the prompt as data.

If the hook cannot install, `--demo` and the trigger socket still work:

```bat
REM terminal A
cargo run -p openatatd

REM terminal B
cargo run -p openatatd -- trigger
REM or
cargo run -p openatatd -- --demo
```

Headless (no Win32 popover):

```bat
cargo run -p openatatd -- --headless --prompt="make this friendlier"
```

C1 uses Windows.Graphics.Capture `CreateForMonitor` of the monitor that owns the foreground window (one frame, then close). First use may prompt for **graphics capture / screenshots** privacy consent. Deny it and the still tile is skipped. `GraphicsCapturePicker` is never used. DXGI Desktop Duplication is fallback only. The overlay HWND is excluded (`WDA_EXCLUDEFROMCAPTURE`).

History lives under `%LOCALAPPDATA%\openatat\history.jsonl` unless `XDG_DATA_HOME` is set. Scratch workspaces: `%LOCALAPPDATA%\openatat\cache\scratch\<id>\`. The trigger client talks to `127.0.0.1` using `%TEMP%\openatat\trigger.port`.

This cloud / Linux agent **cannot** `cargo build --target x86_64-pc-windows-msvc` unless that target and the Windows SDK are installed. That is a real gap, not a successful cross-compile.

## Permissions (Linux)

Optional; deny one and the rest still works.

- **grim** — C1 auto-still of the focused Hyprland output, plus C2/C4 stills after the native picker. Silent. Do not route auto-attach through the xdg-desktop-portal screenshot picker.
- **hyprctl** — active output name + `activewindow` address (insert abort).
- **wlr-data-control** or **wl-copy** — clipboard-first insert and C14 shelf watch.
- **AT-SPI** (`org.a11y.Bus`) — insert into a focused text field; password-role probe every key; C10 selection (`GetText` + selection offsets).
- **Fcitx5 addon (`fcitx5-openatat`)** — product `@@` trigger. See above.
- **layer-shell** — the popover. Hyprland provides it.
- **Quickshell bar chip (`omarchy/openatat`)** — Omarchy presence. Reads `status.json` / `{"cmd":"status"}` on the trigger socket. Not Waybar.

## Permissions (macOS)

Optional TCC grants. Deny one and the rest still works. First-run can finish with none of them.

- **Input Monitoring** — listen-only `@@` event tap. System Settings → Privacy & Security → Input Monitoring → openatatd. Missing: log a grant hint; `--demo` / `trigger.sock` stay up.
- **Accessibility** — AX insert, @@ swallow, C10 `AXSelectedText`, `AXSecureTextField` probe every key. System Settings → Privacy & Security → Accessibility.
- **Screen Recording** — C1 / C2 / C4 via `SCScreenshotManager` + display `SCContentFilter` (OpenAtat windows excluded). Denied: skip the tile. Not `CGWindowListCreateImage`. C2 rubber-band is a nonactivating `NSPanel`.
- **Finder Automation** — insertion location + selection as real POSIX paths (cwd tile + file tiles). Denied: do **not** guess from the title bar. Right-click Service waits.

## Permissions (Windows)

Optional. Deny one and the rest still works.

- **UI Automation** — insert, @@ swallow, C10 `TextPattern`, `IsPassword` / `ES_PASSWORD` every key. Password fields are never read.
- **Graphics Capture** — C1 / C2 / C4 via WGC `CreateForMonitor`. Settings → Privacy & security → Screenshots and apps. Denied: skip the tile. Not `GraphicsCapturePicker`. C2 rubber-band is `WS_EX_NOACTIVATE` with `WDA_EXCLUDEFROMCAPTURE`.
- **Explorer shell** — cwd + selected PIDLs via `IShellWindows` → `IFolderView` when Explorer is frontmost. Title bar is never parsed. Context-menu DLL waits.

History is local: `~/.local/share/openatat/history.jsonl` on Linux/macOS (`id`, `timestamp`, `entry`, `prompt` only). Windows: `%LOCALAPPDATA%\openatat\history.jsonl`. The C14 shelf is a separate file (`clipboard-shelf.json`). Prompts never go through our servers.

## What is stubbed

- IBus engine (optional later). Fcitx5 product trigger is `ime/fcitx5-openatat`.
- `openatat-ui` first-run tutorial (Settings + history + C17 studio are implemented).
- Nautilus (no selection D-Bus API).
- Recording, scrolling capture, OCR, C13 current-clipboard-as-tile, video studio / trim (C18). C2 area, C4 display, C14 clipboard shelf, and C17 still annotation are live.
- Right-click Finder Service / Explorer context-menu DLL.
- Windows tray icon (socket + right-click hide the Orb is enough for v1).

C1 is grim on Linux, ScreenCaptureKit on Mac, and WGC `CreateForMonitor` on Windows (long-edge ~1760, removable tile). Typed `@@` still auto-attaches C1; **Orb click does not**. C2 (`--capture area`) rubber-bands a native nonactivating picker and opens `@@` with that still only. C4 (`--capture display`) is one output still, same overlay, no second C1. C10 is mouse-up + AT-SPI / `AXSelectedText` / UIA TextPattern. C16 (drag-drop onto the Orb) is live on Wayland / macOS / Win32 drop targets — file-manager titles are never scraped. Terminal handoff is implemented on Linux, macOS, and Windows. The Orb is a native layer-shell / NSPanel / `WS_EX_NOACTIVATE` surface in `openatatd`, not gpui and not the Quickshell chip. Overlay/trigger/capture/insert on Mac and Windows are no longer stubs. The Omarchy 4 bar chip is the Quickshell plugin in `omarchy/openatat/`.

## Crate layout

```
crates/openatat-ipc     shared JSON protocol
crates/openatatd        native applet
crates/openatat-ui      gpui-ce Settings + history + C17 studio (`--features gpui`)
ime/fcitx5-openatat     Fcitx5 module (product @@ trigger)
omarchy/openatat        Omarchy 4 Quickshell bar chip (not Waybar)
```

Overlay crates: `wayland-client` + `smithay-client-toolkit` (layer-shell), `wl-clipboard-rs` (data-control), `zbus` (AT-SPI), `image` (downscale). No iced, gtk4-layer-shell, AGS, astal, or Waybar.
