# OpenAtat Omarchy bar chip

Quickshell **bar-widget** for Omarchy 4 (Hyprland + Quickshell). It is the
Omarchy presence path: a small chip that shows whether `@@` is idle, an
agent is running, or the last run failed.

It is **not** Waybar, AGS, gpui, a second overlay, or the Orb. The Orb
is a native layer-shell surface inside `openatatd`. The `@@` popover,
trigger, insert, capture, selection bar, and handoff stay in `openatatd`.
Clicking the chip opens Settings (`openatat-ui`) over the existing unix
socket. If the Settings binary was not built with `--features gpui`, the
daemon returns an error and the click is a no-op. The click never sends
`{"cmd":"trigger"}` and does not map a layer-shell surface.

Plugin id: `openatat.chip` (the `omarchy.*` namespace is reserved for
first-party Omarchy plugins).

## Status protocol

`openatatd` publishes presence without a GPU window:

1. **Unix socket** `$XDG_RUNTIME_DIR/openatat/trigger.sock`
   - request: `{"cmd":"status"}` or `{"cmd":"ping"}` (or the bare word `status`)
   - reply: `{"status":"idle"}` | `{"status":"busy"}` | `{"status":"error","message":"…"}`
2. **Status file** `$XDG_RUNTIME_DIR/openatat/status.json` — same JSON, written
   when the daemon starts and whenever a session begins or ends. The widget
   watches this file (`FileView`); it does not poll the compositor.

Click:

```json
{"cmd":"open-ui","page":"settings"}
```

CLI (same socket):

```bash
openatatd status
```

## Enable on Omarchy 4

Omarchy discovers third-party plugins from
`~/.config/omarchy/plugins/<id>/` (a directory with `manifest.json` + QML).
`omarchy plugin add <git-url>` expects `manifest.json` at the **git root**, so
do **not** point it at this whole repository. Copy or symlink this folder.

From a clone of OpenAtat:

```bash
# 1. Install the plugin directory (copy — Omarchy rejects symlinks *inside* a plugin)
mkdir -p ~/.config/omarchy/plugins
rm -rf ~/.config/omarchy/plugins/openatat.chip
cp -a /path/to/openatat/omarchy/openatat ~/.config/omarchy/plugins/openatat.chip

# 2. Discover it
omarchy-shell shell rescanPlugins
# or: omarchy plugin list

# 3. Put it on the bar (default section: right)
omarchy plugin enable openatat.chip
# equivalent hand edit of ~/.config/omarchy/shell.json — add to bar.layout.right:
#   { "id": "openatat.chip" }

# 4. Restart the shell (not Waybar — Waybar is gone on Omarchy 4)
omarchy-restart-shell
```

Move it later with `omarchy bar move openatat.chip --section right`.

`openatatd` must be running for the chip to leave the “daemon not running”
state. Build and start the applet as in the repo README (`cargo run -p openatatd`).
Settings still need `cargo build -p openatat-ui --features gpui`.

## Validate

On a machine with Omarchy installed:

```bash
omarchy plugin validate /path/to/openatat/omarchy/openatat
```

This repo’s `cargo test --workspace` checks the manifest schema and a dry
JSON status roundtrip. It does not start Quickshell or a compositor.
