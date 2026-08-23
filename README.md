# OpenAtat

Open clone of [Atat](https://atatapp.com): type `@@` in any text field. OpenAtat gathers what you are looking at, runs the CLI agent you already have, and puts the answer back in the line you were typing. It never becomes the active app.

This repository is the P0 Linux/Omarchy spike. Product contract, capture inventory C1–C19, OS API matrix, budgets, permissions, ship order, and landmines: **[SPEC.md](SPEC.md)**. Product facts follow Atat’s [manual](https://atatapp.com/manual) and [FAQ](https://atatapp.com/faq).

Apache-2.0. No bundled LLM.

## Native applet (locked)

| Process | Role |
| --- | --- |
| `openatatd` | Always-on native applet. Owns the `@@` overlay, trigger, insert, capture, and (later) Orb + selection bar. Idle maps **no** surface and starts **no** GPU window. |
| `openatat-ui` | gpui-ce, **on demand**. Settings + history browser (studio / first-run later). Quit when the last window closes. The GPU crate is feature-gated (`--features gpui`) so `cargo test --workspace` does not pull a GPU stack. |

P0 does **not** use gpui for the overlay. gpui-ce 0.3 has LayerShell / PopUp / Transparent / `focus: false`, but that is not a nonactivating panel.

- **Omarchy / Hyprland:** native `zwlr_layer_shell_v1` surface inside `openatatd`, software `wl_shm`. Keyboard `OnDemand` only while the prompt is up.
- **Mac (later):** `NSPanel` + `NSWindowStyleMaskNonactivatingPanel`.
- **Windows (later):** `WS_EX_NOACTIVATE`.

Omarchy 4 is Hyprland + Quickshell (Waybar is gone). Do not add Waybar modules. A Quickshell plugin is only the bar chip, later.

## How to run the Linux spike (Hyprland)

Build:

```bash
cargo build
```

One-shot demo (opens the native popover if `WAYLAND_DISPLAY` is set):

```bash
cargo run -p openatatd -- --demo
```

Type a prompt, `Return` runs the BYO CLI (first provider on `PATH`, or `echo` if none), review the **preview card**, `Tab` copies then tries AT-SPI insert, `R` refines with one more sentence, `Esc` cancels. Click **remove** to drop the C1 still tile.

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
```

`OPENATAT_AGENT` remains an escape hatch: that binary is exec’d directly and the prompt is written to stdin (still not interpolated into a shell).

Every run uses a **scratch workspace** under `~/.cache/openatat/scratch/<id>/`. cwd is set there. Quick answers never run in the folder you happened to have focused — file-manager tiles are not implemented, so it is always scratch. If launch fails, the prompt is copied to the clipboard before the error is shown.

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

### Settings + History (`openatat-ui`)

`openatat-ui` is a **separate** gpui-ce process. Activating it is fine — this is not the `@@` overlay. It exits when the last window closes.

```bash
# Real window (needs gpui-ce + Linux GPU/Wayland headers; see below)
cargo run -p openatat-ui --features gpui -- --settings
cargo run -p openatat-ui --features gpui -- --history

# From a running build: daemon just execs the sibling binary
cargo build -p openatat-ui --features gpui
cargo run -p openatatd -- --settings    # or --history
# IPC (newline JSON on $XDG_RUNTIME_DIR/openatat/trigger.sock):
# {"cmd":"open-ui","page":"settings"}
# {"cmd":"open-ui","page":"history"}
```

Settings writes `~/.config/openatat/agent.toml` (provider + argv list). Unknown keys and comments are kept. History reads `~/.local/share/openatat/history.jsonl` newest first; **Reuse** copies/prints the prompt only. Clear History asks for a second click. Screenshots and agent output are not in the file and are never shown.

The Permissions tab names what each optional Linux grant unlocks (grim, hyprctl, clipboard, AT-SPI, Fcitx5). It does **not** request OS permissions.

`cargo test --workspace` builds `openatat-ui` **without** gpui-ce. A binary built that way exits 2 and tells you to rebuild with `--features gpui`.

gpui-ce 0.3 on Linux needs compile-time headers that this cloud VM often lacks: `libxkbcommon-dev`, `libwayland-dev`, `libvulkan-dev`, and usually `libfontconfig-dev`. Runtime still needs Vulkan + a Wayland or X11 display. Missing those libs is a real build gap — do not treat a header-less compile as success.

## Permissions (Linux)

Optional; deny one and the rest still works.

- **grim** — C1 auto-still of the focused Hyprland output. Silent. Do not route auto-attach through the xdg-desktop-portal screenshot picker.
- **hyprctl** — active output name + `activewindow` address (insert abort).
- **wlr-data-control** or **wl-copy** — clipboard-first insert.
- **AT-SPI** (`org.a11y.Bus`) — insert into a focused text field; password-role probe every key.
- **Fcitx5 addon (`fcitx5-openatat`)** — product `@@` trigger. See above.
- **layer-shell** — the popover. Hyprland provides it.

History is local: `~/.local/share/openatat/history.jsonl` (`id`, `timestamp`, `entry`, `prompt` only). Prompts never go through our servers.

## What is stubbed in P0

- IBus engine (optional later). Fcitx5 product trigger is `ime/fcitx5-openatat`.
- Mac overlay (`NSPanel` nonactivating), ScreenCaptureKit, AX insert.
- Windows overlay (`WS_EX_NOACTIVATE`), WGC, UI Automation.
- `openatat-ui` studio / first-run (Settings + history are implemented).
- Orb, selection bar, clipboard shelf, Finder/Nautilus (Nautilus has **no** selection D-Bus API).
- Recording, scrolling capture, OCR, annotation studio (C6–C19 except comments).
- Handoff to a terminal.

C1 (auto-still via grim, long-edge ~1760, removable tile) is implemented.

## Crate layout

```
crates/openatat-ipc     shared JSON protocol
crates/openatatd        native applet
crates/openatat-ui      gpui-ce Settings + history (`--features gpui`)
ime/fcitx5-openatat     Fcitx5 module (product @@ trigger)
```

Overlay crates: `wayland-client` + `smithay-client-toolkit` (layer-shell), `wl-clipboard-rs` (data-control), `zbus` (AT-SPI), `image` (downscale). No iced, gtk4-layer-shell, AGS, astal, or Waybar.
