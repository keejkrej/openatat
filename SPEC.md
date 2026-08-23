# OpenAtat specification

OpenAtat is an open, cross-platform clone of [Atat](https://atatapp.com) (`@@` for Mac). Product facts below come from the Atat [manual](https://atatapp.com/manual) and [FAQ](https://atatapp.com/faq). This file is the contract for the native applet, the on-demand UI process, and the ship order.

OpenAtat never becomes the active app.

## 1. Product contract

Type `@@` in any text field. OpenAtat gathers what the user is looking at, runs the CLI agent they already have, and puts the answer back in the line they were typing.

It is not another workspace. There are no sessions to maintain. Call it up, get the result, get out of the way.

### What it is

- A launcher for agents already on the machine. Bring your own CLI (Claude Code, Codex, Grok, Cursor, Pi, Hermes, OpenCode, or any custom CLI). OpenAtat does not ship a bundled LLM. If none of those binaries are on `PATH`, the applet falls back to `echo` and logs that no provider is installed.
- A gatherer. A still of the active display is attached before the agent runs. Later phases add selection, clipboard, files, recordings, and annotation. The user sees attachments as removable tiles before send.
- An inserter. Every answer lands in a **preview card** first. Nothing is typed into a document or copied without going through that card. `Tab` inserts when a text field is still focused; otherwise `Tab` copies. Insertion is clipboard-first, then a focus check, then AT-SPI (Linux) / AX (Mac) / UIA (Windows). If focus moved, insert aborts; the result is still on the clipboard.
- Local. Prompts never go through our servers. History is one local file. No OpenAtat account.

### The `@@` trigger

- The trigger is always `@@`, case-insensitive, typed in place. There is **no global summon hotkey**. Atat’s FAQ is explicit: the Orb is one click away and `@@` already works wherever you can type; a summon hotkey would add shortcut conflicts without a faster path.
- Detection buffer holds the **last two characters only**. Nothing typed before or after the trigger is collected.
- **Secure fields are skipped on every key** and the result is never cached. Password / secure-input roles are re-probed each event.
- **IME must not fire the trigger.** Composing / preedit (Chinese, Japanese, and other input methods) is ignored. Only committed characters enter the two-character buffer.
- When the trigger fires in a text field, the two `@` characters are removed from the client and a small overlay opens next to the work. Keystrokes then go to OpenAtat until `Esc` / `Tab` / dismiss. Focused on something that clearly cannot accept text, OpenAtat still opens (later: window capture + optional app-layout tile) and `Tab` copies rather than inserting.
- Terminals, web pages, text controls, and anything ambiguous stay on the normal input path: if a place might accept typing, treat it as an input.

### Overlay and focus

- The overlay is a **small popover**, not a fullscreen dim/blur.
- OpenAtat must not become the active application. The client keeps its “active app” identity.
- Keyboard interactivity is **OnDemand, and only while the prompt / preview / selection bar is up**. Idle has no mapped **overlay**. The **Orb** is mapped at idle with keyboard **none** (`KeyboardInteractivity::None` — never Exclusive or OnDemand).
- `Esc` cancels from any state.
- Preview card is **mandatory**. Refine-in-place (`R` + one more sentence) re-runs with the same attachments and replaces the preview. Handoff (`⌘Return` / Super+Return → real agent session in the user's terminal) is **done on Linux and macOS**.

### Selection bar (C10)

- Mouse-up only. Keyboard selections do not summon the bar.
- Compact native layer-shell bar (same compositor client as `@@`): Ask @@, Copy, Search, Summarize, Explain. User-defined prompts can wait.
- Selected text is ephemeral: used for the action, never stored in history, never logged.
- Secure / password fields are never read. Re-probe AT-SPI `PasswordText` every time, never cache.
- Ask / one-click prompts go through the preview card. Replace-in-place only after `Tab` (clipboard-first, then Hyprland address, then `DeleteText` + `InsertText`). No synthetic backspaces. Copy and Search run immediately.
- If AT-SPI cannot expose a selection, skip. Do not invent a clipboard save/restore. This is text selection, not Nautilus.

### Preview, insert, clipboard

1. Agent finishes → preview card shows the result.
2. User presses `Tab`.
3. Result is written to the clipboard **before** insertion starts (`wlr-data-control` / `wl-copy` on Linux).
4. Re-read the focused window identity. If the address changed, **abort insert**.
5. If a text field is still focused, insert via the platform a11y API. If not, the clipboard copy is the product outcome.

A failed insert is still a copy.

### History

Each stored entry is exactly four fields: `id`, `timestamp`, `entry` (entry point), `prompt`. Never store agent responses, screenshots, or attached context. A refine sentence is a new history row (or an updated prompt); still never the agent reply. Linux path: `~/.local/share/openatat`.

### Process split (locked)

This split is a product decision. Do not revisit it for convenience.

| Process | Owns | Lifetime |
| --- | --- | --- |
| `openatatd` (native applet) | `@@` overlay, **Orb**, C10 selection bar, terminal handoff, trigger, insert, capture | Always on. Idle has **zero GPU windows**. Orb is mapped; overlay is not. |
| `openatat-ui` (gpui-ce) | Settings, studio / annotation, history browser, first-run | Spawn on demand, quit when idle. |

P0 does **not** use gpui for the overlay. gpui-ce 0.3 has `LayerShell` / `PopUp` / `Transparent` / `focus: false`, but that is **not** a nonactivating panel.

| OS | Overlay host | Why not gpui |
| --- | --- | --- |
| macOS | `NSPanel` with `NSWindowStyleMaskNonactivatingPanel` | Required so @@ never becomes the active app. |
| Windows | `WS_EX_NOACTIVATE` (and typically `WS_EX_TOOLWINDOW`) | Same nonactivating contract. |
| Linux / Omarchy | Native `zwlr_layer_shell_v1` surface **inside the applet** | Software `wl_shm`. No GPU process at idle. |

Quickshell is Omarchy 4’s shell (Hyprland + Quickshell; Waybar is gone). A Quickshell plugin is **only** the bar chip (`omarchy/openatat`, id `openatat.chip`). It shows idle / agent running / error and may open Settings. It is not a second overlay and not the Orb. Do not add Waybar modules. Do not use iced, gtk4-layer-shell as the main UI, AGS, or astal.

Capture stays in the daemon. gpui `ScreenCaptureFrame` is a stub.

### Agents

OpenAtat is a launcher, not a model host. Quick answers use a **visible argv template** (shown on the overlay; editable in `~/.config/openatat/agent.toml`).

- **Providers (PATH lookup, product order):** `claude`, `codex`, `grok`, `cursor-agent`/`agent`, `pi`, `hermes`, `opencode`. Default is the first found. `provider = "dummy"` or a missing install uses `echo` and logs that no provider is installed.
- **Prompt as data.** Templates are an argv list, never a shell line. The prompt is one argv element (`{prompt}`), stdin (no placeholder), or a temp file the template names (`{prompt_file}`). Quotes and newlines stay intact. `OPENATAT_AGENT` is an escape hatch: that binary, prompt on stdin.
- **Scratch workspace.** Every run creates a directory under `~/.cache/openatat/scratch/`, sets cwd there, and filters the environment. Quick answers never use the user’s current folder (file-manager tiles are not implemented). Conservative flags are used only when the CLI documents them (`claude --print --permission-mode plan`, `codex exec --sandbox read-only`, `grok --sandbox read-only --prompt-file`, Cursor `--print --mode ask --trust`). Unverified flags are not invented.
- **Launch failure.** Copy the prompt to the clipboard, then show the error. Never lose what they typed.
- **Handoff** (Linux + macOS + Windows): Super+Return / `⌘Return` / Win+Return / preview **Handoff** button. `openatatd` launches the user’s terminal in a scratch cwd (`~/.cache/openatat/scratch/<id>/`, or `%LOCALAPPDATA%\openatat\cache\scratch\<id>\` on Windows) and starts the same BYO CLI as an interactive session (no `--print` / plan-mode / one-shot flags). Prompt is argv or a file, never a shell string. Terminal pick: `handoff.terminal` in `agent.toml`, else PATH order ghostty → kitty → alacritty → wezterm → foot → gnome-terminal → xterm (Mac adds iTerm, then Terminal.app; Windows prefers `wt.exe`, then wezterm / alacritty / cmd without `/c`). File-manager tiles never become the handoff cwd and are never guessed from a window title. Launch failure copies the prompt first. Esc still cancels. Successful handoff dismisses the overlay. Mac spawn is `NSWorkspace.openApplication` + `OpenConfiguration`. Windows spawn is `CreateProcessW` (`lpCurrentDirectory` = scratch).

## 2. Capture inventory (C1–C19)

| ID | Capture | P0 | Notes |
| --- | --- | --- | --- |
| C1 | Auto-still of the **active output** when `@@` fires | **Yes** | Silent. grim on Linux. No portal picker on the auto-attach path. Downscale long-edge ~1600–1920. Removable tile. |
| C2 | Area screenshot | No | Atat `⌘⇧4`. Interactive region. |
| C3 | Window screenshot | No | Atat window target. |
| C4 | Full-display / explicit display still | No | Atat `⌘⇧3`. C1 is the auto path. |
| C5 | All-in-one picker | No | Atat `⌘⇧5`: shot / window / scroll / OCR / record / Ask. |
| C6 | Scrolling capture | No | Stub / comment only. |
| C7 | Video recording | No | Stub / comment only. |
| C8 | GIF recording | No | Stub / comment only. |
| C9 | OCR | No | Stub / comment only. |
| C10 | Live text selection (selection bar) | **P1 Linux + Mac + Win** | Mouse-up only. Native overlay bar in `openatatd` (not gpui). AT-SPI / `AXSelectedText` / UIA TextPattern. Secure role re-probed every time. Selected text is ephemeral (never history / logs). Skip if a11y exposes no selection — no clipboard dance. |
| C11 | File-manager working directory | **Mac** | Finder Automation `insertion location`. If denied, do not guess from the title bar. Nautilus has **no selection D-Bus API**. |
| C12 | File-manager selected files | **Mac** | Finder Automation `selection` as POSIX paths. Right-click Service waits. |
| C13 | Current clipboard item as a tile | No | Distinct from insert’s clipboard-first write. |
| C14 | Clipboard history shelf | No | Atat `⌘⇧V`. Passwords never enter history. |
| C15 | App-layout / a11y-tree tile | No | When focus is clearly not a text field. |
| C16 | Drag-and-drop onto the Orb | **Yes** | Wayland / macOS / Win32 drop targets. Files, images, text become tiles. Never scrape Nautilus / Explorer / Finder titles. |
| C17 | Annotation / crop before send | No | Studio lives in `openatat-ui`. |
| C18 | Video trim / export | No | Studio. |
| C19 | Recording keyboard bezel | No | KeyCastr-style overlay during record. |

C1 is grim on Linux, ScreenCaptureKit on Mac, and WGC `CreateForMonitor` on Windows. C10 is live on Linux (mouse-up + AT-SPI), Mac (mouse-up + AXSelectedText), and Windows (mouse-up + UIA TextPattern). C11/C12 are live on Mac via Finder Automation and on Windows via Explorer `IShellWindows` (never the title bar). C16 is live: drag-drop onto the Orb (Wayland / Cocoa / Win32; never a title-bar scrape). C6–C19 stay in this inventory so later work does not invent a second taxonomy.

## 3. OS API matrix

| Concern | Linux / Omarchy 4 (Hyprland + Quickshell) | macOS | Windows |
| --- | --- | --- | --- |
| Overlay | Native `zwlr_layer_shell_v1` in `openatatd`, `wl_shm`, `KeyboardInteractivity::OnDemand` while up | `NSPanel` nonactivating | `WS_EX_NOACTIVATE \| TOPMOST \| TOOLWINDOW \| LAYERED`. Never `SetForegroundWindow`. |
| Orb | Same compositor client class: `zwlr_layer_shell_v1` Overlay layer, exclusive zone 0, input region = circle, `KeyboardInteractivity::None`. Not Quickshell. | `NSPanel` + `NSWindowStyleMaskNonactivatingPanel`, circle hit-test. `NSStatusItem` Show Orb. | Same `WS_EX_*` as overlay, circle hit region. Never `SetForegroundWindow`. Tray later. |
| Trigger (product) | Fcitx5 module `fcitx5-openatat`; committed text only | Listen-only CGEvent tap + `ImeFilter`. `IsSecureEventInputEnabled` / `AXSecureTextField` every key. IME composing ignored. `@@` swallowed via AX replace | Process-local `WH_KEYBOARD_LL` (Raw Input fallback) + `ImeFilter`. UIA `IsPassword` / `ES_PASSWORD` every key. IME composition ignored. `@@` swallowed via UIA replace or one paste. Not a raw hotkey. |
| Trigger (demo) | Unix socket + `--demo` / `--once`. Not a product hotkey | Same socket + `--demo` if Input Monitoring is missing | TCP `127.0.0.1` + `--demo` if the hook cannot install |
| Secure field | AT-SPI `Role::PasswordText` (and related) **every key** | Secure Event Input / AX secure role every key | UIA `IsPassword` / Win32 `ES_PASSWORD` every key |
| Screen still | **grim** (`-o` active output). No xdg-desktop-portal picker on auto-attach | ScreenCaptureKit | WGC `CreateForMonitor` (monitor of the foreground window). Not `GraphicsCapturePicker`. DXGI Desktop Duplication fallback. Overlay HWND excluded. |
| Downscale | CPU, long-edge 1600–1920 | Same policy | Same policy |
| Insert | AT-SPI `EditableText.InsertText` (replace uses `DeleteText` then insert; no synthetic backspaces) | `AXUIElement` | Clipboard-first, then `GetForegroundWindow` + UIA RuntimeId, then ValuePattern / TextPattern or one Ctrl+V. Browsers paste. |
| Selection bar (C10) | AT-SPI `GetNSelections` / `GetSelection` / `GetText` / `GetRangeExtents`; `RegisterEvent("mouse:b1r")`. Skip if no selection. | `NSEvent` left-mouse-up + `AXSelectedText`. Keyboard selections do not summon. | Mouse-up + UIA TextPattern. Keyboard selections do not summon. |
| Focus identity | `hyprctl activewindow` **address** | PID + AX window | `HWND` + UIA RuntimeId |
| Clipboard | `wlr-data-control` via `wl-clipboard-rs`, `wl-copy` fallback | `NSPasteboard` | Win32 clipboard |
| File manager | Nautilus: no selection D-Bus API — do not fake paths | Finder Automation | Explorer `IShellWindows` → `IFolderView`. Never the title bar. |
| Settings / studio | `openatat-ui` gpui-ce, on demand (Settings + history now; studio later) | same | same |
| Bar chip | Quickshell plugin `openatat.chip` (`omarchy/openatat`). Status via `{"cmd":"status"}` on `trigger.sock` and `$XDG_RUNTIME_DIR/openatat/status.json` (`idle` / `busy` / `error`). Click = `open-ui` Settings (no-op if UI lacks gpui). **Not** Waybar. | menu extra / Orb | tray later |
| Capture in gpui | `ScreenCaptureFrame` is a stub — do not use | stub | stub |

macOS modules are real `NSPanel` / ScreenCaptureKit / AX / `NSPasteboard` / `NSWorkspace` (cfg-gated). Windows modules are real `WS_EX_NOACTIVATE` / WGC / UIA / Win32 clipboard / `CreateProcessW` (cfg-gated). Policy tests for both OS paths run on Linux.

## 4. Performance budgets

These are P0 targets for the Linux spike, not promises about a shipped installer.

| Path | Budget | Why |
| --- | --- | --- |
| Idle applet | No GPU process, Orb mapped, overlay unmapped, no gpui window | “Never become the active app” and Omarchy laptops |
| Idle RSS | Prefer well under 40 MB | Native applet, software stack |
| `@@` → popover first paint | < 80 ms after trigger (excluding first Wayland connect) | Feels like typing, not launching |
| C1 still (grim + downscale) | < 250 ms on a 1080p–1440p output | Tile appears with the prompt |
| Dummy agent (`echo`) | < 50 ms | Preview card is mandatory even when the CLI is instant |
| Clipboard write | < 40 ms | Must complete before insert starts |
| Focus re-check (`hyprctl`) | < 15 ms | Abort beats a wrong-window insert |
| AT-SPI insert | Best-effort; timeout ~400 ms then keep the copy | Clipboard is the fallback |
| History append | < 10 ms, durable JSONL | Local only |

Reduce Motion (later) skips the snapshot “flight” animation. P0 has no animation.

## 5. Permissions

Every permission is optional. Deny one and the rest of the app keeps working; that feature stays dormant.

### Linux / Omarchy (P0)

| Need | What it unlocks | How |
| --- | --- | --- |
| `WAYLAND_DISPLAY` + layer-shell | Overlay popover | Hyprland ships `zwlr_layer_shell_v1` |
| grim allowed to capture outputs | C1 auto-still | Hyprland: grim is silent; do **not** route auto-attach through xdg-desktop-portal Screenshot (picker) |
| `hyprctl` | Active output name + window address | Hyprland instance signature socket |
| `wlr-data-control` or `wl-copy` | Clipboard-first insert | Hyprland supports data-control |
| AT-SPI bus (`org.a11y.Bus`) | Insert + secure-field probe + C10 selection | Enable accessibility; some apps need `GTK_USE_PORTAL` / toolkit a11y |
| Fcitx5 (`fcitx5-openatat`) | Product `@@` trigger | C++ module; see IME plan |
| Unix socket `$XDG_RUNTIME_DIR/openatat/trigger.sock` | Addon + demo trigger + bar-chip status / Settings / Show Orb | `fcitx5-openatat`, `openatatd trigger`, `{"cmd":"status"}`, `{"cmd":"open-ui"}`, `{"cmd":"hide-orb"}`, `{"cmd":"show-orb"}` |
| `$XDG_RUNTIME_DIR/openatat/status.json` | Omarchy Quickshell bar chip (`openatat.chip`) | Written by `openatatd` on idle / busy / error. No extra GPU surface. |

The product path needs `fcitx5-openatat` installed and Fcitx5 running. `--demo` / `openatatd trigger` stay available without the addon.

### macOS

Every grant is optional. Deny one and the rest of the app keeps working; that feature stays dormant. First-run can finish with none of them.

| Need | What it unlocks | How |
| --- | --- | --- |
| Input Monitoring | Listen-only `@@` tap | System Settings → Privacy & Security → Input Monitoring → openatatd |
| Accessibility | AX insert, @@ swallow, C10, secure probe | System Settings → Privacy & Security → Accessibility |
| Screen Recording | C1 `SCScreenshotManager` | System Settings → Privacy & Security → Screen Recording. Skip tile if denied. |
| Finder Automation | cwd + selected-file tiles | System Settings → Privacy & Security → Automation → Finder. Denied: do not parse the title bar. |

`--demo` and `$TMPDIR/openatat/trigger.sock` (or `$XDG_RUNTIME_DIR`) work with zero grants.

### Windows

Every grant is optional. Deny one and the rest of the app keeps working; that feature stays dormant. First-run can finish with none of them.

| Need | What it unlocks | How |
| --- | --- | --- |
| UI Automation | UIA insert, @@ swallow, C10, `IsPassword` / `ES_PASSWORD` every key | No extra dialog on most builds. Password fields are never read. |
| Graphics Capture consent | C1 `CreateForMonitor` | Settings → Privacy & security → Screenshots and apps. Skip tile if denied. Never `GraphicsCapturePicker`. |
| Explorer shell | cwd + selected PIDLs | `IShellWindows` → `IFolderView` when Explorer is frontmost. Denied / failed: do not parse the title bar. Context-menu DLL waits. |

`--demo` and the trigger TCP port (`%TEMP%\openatat\trigger.port`) work if the keyboard hook cannot install.

### Data

Prompts and context go to the local CLI only. History is `id`, `timestamp`, `entry`, `prompt`. OpenAtat does not run a telemetry service in P0.

## 6. IME plan (product path, not a hotkey)

A global hotkey is **not** the product. The product is “the user typed `@@` in the field.”

### Why an IME-shaped filter

On Linux there is no supported equivalent of macOS Input Monitoring that is both compositor-portable and IME-correct. Reading raw evdev or grabbing Hyprland keys:

- fires inside password fields unless we re-implement secure detection in the same path
- fires in the middle of CJK preedit
- becomes a de facto summon hotkey

An IME filter sees text the same way the client does: compose first, commit later.

### Shape

```
key / compose event
    → probe focused field (AT-SPI, no cache)
    → if secure: drop, clear buffer, stop
    → if preedit / composing: do not touch the two-char buffer
    → if committed text: push into last-two-chars (case-fold for `@`)
    → if buffer == "@@": swallow the two characters, notify openatatd
```

### Backends

| Backend | Crate / vehicle | Status |
| --- | --- | --- |
| Filter interface + detection buffer | In-process Rust (`openatatd::trigger`) and C++ (`ime/fcitx5-openatat/src/openatat_filter.*`) | **Same contract.** Do not add a second detector. |
| Dev Wayland / test path | Unix socket, `openatatd trigger`, `--demo` | Keep for CI / no-IME sessions |
| Fcitx5 addon | C++ module `ime/fcitx5-openatat`. No usable Rust addon crate. | **Product path (Omarchy / Arch).** See build/install below. |
| IBus engine | `ibus` C API; no maintained high-level Rust engine crate we will ship on | Optional later. Do not block Fcitx5. |

### Building `fcitx5-openatat`

Hook: Fcitx5 **module** (`AddonInstance`) watching `InputContextKeyEvent` at `PostInputMethod` plus `Instance::CommitFilter`. That is the thinnest pair that sees committed characters (keyboard keys that the IM did not consume, and CJK `commitString`) and can swallow them (`filterAndAccept` / rewrite the commit / `deleteSurroundingText` for a leftover `@`). Compose is ignored via `Instance::isComposing` and empty preedit. Secure fields are re-probed every key (`CapabilityFlag::PasswordOrSensitive`, plus optional AT-SPI `PasswordText`). Missing `trigger.sock` is silent.

```bash
# Arch / Omarchy
sudo pacman -S --needed fcitx5 extra-cmake-modules cmake ninja pkgconf
cd ime/fcitx5-openatat
cmake -B build -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build
./build/openatat-filter-test    # no display
sudo cmake --install build
fcitx5 -r
```

If CMake reports `Fcitx5Core not found`, only the filter unit test is built. That is **not** a successful addon link — install `fcitx5` headers and re-run CMake. Full copy-paste path: `ime/fcitx5-openatat/README.md`.

`Fcitx5Backend::start` in `openatatd` detects addon presence and logs a hint when Fcitx5 is running without the module. The daemon does not fail if Fcitx5 is absent.

The filter module is intentionally IME-shaped so the Fcitx5 addon stays an adapter rather than a second detector.

### Swallowing `@@`

The addon (not the overlay) deletes the two characters from the client — typically by not forwarding them, or by committing a deletion. The daemon must not send synthetic backspaces into an unknown window as a first choice.

## 7. Ship order

### P0 — Linux / Omarchy spike (this repo)

- `SPEC.md`, workspace, `openatatd`, `openatat-ui` (Settings + history in P1).
- IME-filter module + detection tests + dev trigger path.
- Native layer-shell popover (software `wl_shm`). No gpui overlay.
- C1 via grim, downscale, removable tile.
- Dummy CLI, mandatory preview, `Tab` = clipboard then AT-SPI, abort on `hyprctl` address change.
- `Esc` cancels. JSONL history in `~/.local/share/openatat`.
- Mac overlay/trigger/capture/insert/C10/Finder/handoff are implemented (`cfg(target_os = "macos")`).
- Windows overlay/trigger/capture/insert/C10/Explorer/handoff are implemented (`cfg(target_os = "windows")`).
- `cargo test` / `cargo build` on Linux.

### P1 — Make `@@` real on Omarchy

- Fcitx5 addon (`ime/fcitx5-openatat`) that implements the filter contract. **Done for the trigger path.**
- BYO CLI runner (template, scratch dir, no shell interpolation). **Done.** Provider pick is `~/.config/openatat/agent.toml`.
- Preview refine (`R`). **Done.**
- `openatat-ui` Settings + history browser (gpui-ce), spawn/quit. **Done.** Studio / first-run still later. The GPU dep is feature-gated on `openatat-ui` only (`--features gpui`) so applet tests stay display-free.
- Selection bar (C10) for Linux mouse selections when AT-SPI reports selected text. **Done.** Keyboard selections do not summon. Browsers/Electron that expose no selection are skipped (no clipboard save/restore). User-defined prompts and an exclude list can wait on Settings.
- Quickshell bar chip (not Waybar). **Done.** Plugin source in `omarchy/openatat/`; daemon exposes `{"cmd":"status"}` / `status.json`.
- Handoff to a terminal. **Done on Linux, macOS, and Windows.**

### P2 — Other OS + the rest of C2–C19

- Mac `NSPanel` + ScreenCaptureKit + AX + Finder. **Done for the overlay / trigger / C1 / insert / C10 / Finder / handoff path.**
- Windows `WS_EX_NOACTIVATE` + WGC + UIA. **Done for the overlay / trigger / C1 / insert / C10 / Explorer / handoff path.**
- Orb (no summon hotkey). **Done.** Click opens an empty prompt (no C1). C16 drop is live. Hide is this-launch only via socket / right-click (Mac: NSStatusItem).
- Studio / annotation in `openatat-ui`.
- Clipboard shelf, scrolling capture, OCR, recording.
- Nautilus: do not invent a D-Bus API; document a user-driven tile or a future GNOME extension.

## 8. Landmines

1. **Nonactivating is the product.** An xdg-toplevel, a focused gpui window, or layer-shell `Exclusive` keyboard at idle makes OpenAtat “the active app.”
2. **gpui-ce layer-shell ≠ NSPanel.** Fine for Settings. Illegal for the `@@` overlay.
3. **gpui `ScreenCaptureFrame` is a stub.** Capture stays in `openatatd`.
4. **Portal screenshot pickers.** Auto-attach must stay grim (or a future silent protocol). A chooser breaks the Atat moment.
5. **IME compose.** Two `@` in preedit are not a trigger. Losing compose chars is a ship blocker.
6. **Secure fields.** Probe every key. Never cache “this window is fine.”
7. **Two-character buffer.** No ring of keystrokes, no accessibility text dump for detection.
8. **Insert into the wrong window.** Compare Hyprland window **address** (not title). Abort > guess.
9. **Clipboard after insert, or insert without clipboard.** Order is clipboard first.
10. **Nautilus has no selection D-Bus API.** Do not scrape the view or guess URIs.
11. **Waybar is gone on Omarchy 4.** The presence path is the Quickshell chip (`omarchy/openatat`); no Waybar module.
12. **No bundled model.** BYO CLI on PATH; `echo` dummy only when nothing is installed. Never splice the prompt into a shell string.
13. **Synthetic backspaces** into the client are a last resort and must be gated on the same focus address.
14. **AT-SPI in browsers / Electron / games** is incomplete. Clipboard-first saves the result.
15. **Recording, scrolling, OCR, shelf, studio** are out of P0. Stubs and comments only. The Orb is implemented (not a stub).

## 9. Crate choices (P0)

Investigated and used:

| Need | Crate | Why |
| --- | --- | --- |
| Wayland + layer-shell | `wayland-client` 0.31 + `smithay-client-toolkit` 0.20 | Real `zwlr_layer_shell_v1` client, `wl_shm`, seats. Matches the Smithay `simple_layer` pattern. |
| Clipboard | `wl-clipboard-rs` 0.9 | Implements `ext-data-control` / `wlr-data-control`. `wl-copy` binary as fallback. |
| Still decode / downscale | `image` 0.25 (png only) | CPU. No GPU image pipeline. |
| Overlay glyphs | `font8x8` 0.3 | Software bitmap, no fontconfig / FreeType at idle. |
| macOS AppKit / SCK / AX | `objc2` 0.6 + `objc2-app-kit` / `objc2-screen-capture-kit` 0.3.2 + `accessibility-sys` 0.2 | NSPanel nonactivating, SCScreenshotManager, AXUIElement. |
| Windows Win32 / UIA / WGC | `windows` 0.61 (gpui-ce 0.3 era) | `WS_EX_NOACTIVATE` overlay, `WH_KEYBOARD_LL`, UIA, `CreateForMonitor`, `CreateProcessW`. |
| AT-SPI insert + C10 | `zbus` 5 calling `org.a11y.atspi.*` | Same bus the `atspi` crate (Odilia) wraps. Selection events stay on `zbus` (`RegisterEvent("mouse:b1r")` + `Event.Mouse::Button`). No second compositor client: the `@@` popover and the selection bar share the layer-shell host in `openatatd`. |
| JSON / errors | `serde`, `serde_json`, `thiserror` | IPC + history. |

No crate found (documented, not invented):

| Need | Reality |
| --- | --- |
| Fcitx5 addon in Rust | No shippable addon SDK crate. C++ module: `ime/fcitx5-openatat`. |
| IBus engine in Rust | No maintained engine crate we will depend on. |
| Nautilus selection | No D-Bus API. |
| gpui nonactivating panel | Does not exist. |
| Silent ScreenCaptureKit / WGC wrappers we need on Linux | N/A on Linux. Mac uses `objc2-screen-capture-kit` + `SCScreenshotManager`. Windows uses `windows` 0.61 WGC interop (`CreateForMonitor`). |

Forbidden UI stacks for the overlay: iced, gtk4-layer-shell-as-main-UI, AGS, astal, Waybar, gpui.

## 10. P0 success

`cargo test` and `cargo build` succeed on Linux. This file lives in the repo. The README tells a human how to run `openatatd` on Hyprland. The overlay path is native layer-shell, not gpui.
