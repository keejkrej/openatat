# OpenAtat

Open clone of [Atat](https://atatapp.com): type `@@` in any text field. OpenAtat gathers what you are looking at, runs the CLI agent you already have, and puts the answer back in the line you were typing. It never becomes the active app.

This repository is the P0 Linux/Omarchy spike. Product contract, capture inventory C1–C19, OS API matrix, budgets, permissions, ship order, and landmines: **[SPEC.md](SPEC.md)**. Product facts follow Atat’s [manual](https://atatapp.com/manual) and [FAQ](https://atatapp.com/faq).

Apache-2.0. No bundled LLM.

## Native applet (locked)

| Process | Role |
| --- | --- |
| `openatatd` | Always-on native applet. Owns the `@@` overlay, trigger, insert, capture, and (later) Orb + selection bar. Idle maps **no** surface and starts **no** GPU window. |
| `openatat-ui` | gpui-ce, **on demand**. Settings, studio/annotation, history, first-run. Quit when idle. P0 is a placeholder binary so we do not pull a GPU stack into `cargo test`. |

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

Type a prompt, `Return` runs the dummy CLI (`echo`), review the **preview card**, `Tab` copies then tries AT-SPI insert, `Esc` cancels. Click **remove** to drop the C1 still tile.

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

The daemon listens on `$XDG_RUNTIME_DIR/openatat/trigger.sock`. The Fcitx5/IBus addon (P1) will send the same JSON. A Hyprland bind is not the product path — see SPEC.md §6.

Optional:

| Variable | Meaning |
| --- | --- |
| `OPENATAT_AGENT` | BYO CLI. Prompt is written to stdin (never spliced into a shell string). |
| `OPENATAT_PROMPT` | Headless / `--demo` default prompt. |
| `OPENATAT_INSERT` | Headless path also runs clipboard + insert. |
| `OPENATAT_DEMO` | Same as `--demo`. |

`openatat-ui` prints the P0 placeholder and exits.

## Permissions (Linux)

Optional; deny one and the rest still works.

- **grim** — C1 auto-still of the focused Hyprland output. Silent. Do not route auto-attach through the xdg-desktop-portal screenshot picker.
- **hyprctl** — active output name + `activewindow` address (insert abort).
- **wlr-data-control** or **wl-copy** — clipboard-first insert.
- **AT-SPI** (`org.a11y.Bus`) — insert into a focused text field; password-role probe every key.
- **Fcitx5 or IBus addon** — product `@@` trigger (stub in P0).
- **layer-shell** — the popover. Hyprland provides it.

History is local: `~/.local/share/openatat/history.jsonl` (`id`, `timestamp`, `entry`, `prompt` only). Prompts never go through our servers.

## What is stubbed in P0

- Product IME backends (Fcitx5 / IBus): interface + tests + socket. No C++ addon yet.
- Mac overlay (`NSPanel` nonactivating), ScreenCaptureKit, AX insert.
- Windows overlay (`WS_EX_NOACTIVATE`), WGC, UI Automation.
- `openatat-ui` Settings / studio / first-run (gpui-ce).
- Orb, selection bar, clipboard shelf, Finder/Nautilus (Nautilus has **no** selection D-Bus API).
- Recording, scrolling capture, OCR, annotation studio (C6–C19 except comments).
- Real BYO agent templates / handoff.

C1 (auto-still via grim, long-edge ~1760, removable tile) is implemented.

## Crate layout

```
crates/openatat-ipc     shared JSON protocol
crates/openatatd        native applet
crates/openatat-ui      gpui-ce placeholder
```

Overlay crates: `wayland-client` + `smithay-client-toolkit` (layer-shell), `wl-clipboard-rs` (data-control), `zbus` (AT-SPI), `image` (downscale). No iced, gtk4-layer-shell, AGS, astal, or Waybar.
