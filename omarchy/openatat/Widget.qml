import QtQuick
import Quickshell
import Quickshell.Io

// Omarchy 4 bar-widget (kinds: bar-widget). Host injects `bar`, `moduleName`,
// `settings`. Presence only — not a second @@ overlay, not the Orb.
// Click opens Settings via the daemon socket; it never sends `trigger`.
Item {
  id: root

  property var bar
  property string moduleName
  property var settings

  property string presence: "idle"
  property string presenceMessage: ""
  property bool daemonSeen: false
  property string pendingLine: ""

  readonly property string runtimeDir: {
    var xdg = ""
    try { xdg = Quickshell.env("XDG_RUNTIME_DIR") } catch (e) { xdg = "" }
    return xdg && xdg.length ? xdg : "/tmp"
  }
  readonly property string statusPath: runtimeDir + "/openatat/status.json"
  readonly property string socketPath: runtimeDir + "/openatat/trigger.sock"

  readonly property color fg: (bar && bar.foreground) ? bar.foreground : "#e6e6e6"
  readonly property color urgent: (bar && bar.urgent) ? bar.urgent : "#e05c5c"
  readonly property color chipColor: (root.presence === "error" && root.daemonSeen) ? root.urgent : root.fg
  readonly property real chipOpacity: {
    if (!root.daemonSeen)
      return 0.45
    if (root.presence === "busy" || root.presence === "error")
      return 1.0
    return 0.75
  }
  readonly property string chipText: {
    if (!root.daemonSeen)
      return "@@"
    if (root.presence === "busy")
      return "@@ ·"
    if (root.presence === "error")
      return "@@ !"
    return "@@"
  }
  readonly property string chipTooltip: {
    if (!root.daemonSeen)
      return "OpenAtat: daemon not running"
    if (root.presence === "busy")
      return "OpenAtat: agent running"
    if (root.presence === "error")
      return root.presenceMessage && root.presenceMessage.length
        ? ("OpenAtat: " + root.presenceMessage)
        : "OpenAtat: error"
    return "OpenAtat: idle — click opens Settings"
  }

  implicitWidth: label.implicitWidth + 12
  implicitHeight: bar && bar.barSize ? bar.barSize : 26

  function applyStatusText(raw) {
    if (!raw || !String(raw).trim().length)
      return
    try {
      var obj = JSON.parse(String(raw).trim())
      var s = obj.status || obj.state || "idle"
      if (s === "ok")
        s = "idle"
      if (s === "idle" || s === "busy" || s === "error") {
        root.presence = s
        root.presenceMessage = obj.message || ""
        root.daemonSeen = true
      }
    } catch (e) {
    }
  }

  function fileText(view) {
    if (!view)
      return ""
    if (typeof view.text === "function")
      return view.text()
    return view.text || ""
  }

  function sendLine(line) {
    root.pendingLine = line
    if (ipc.connected)
      ipc.connected = false
    Qt.callLater(function() { ipc.connected = true })
  }

  function openSettings() {
    // Settings may activate. The @@ overlay stays in openatatd and must
    // not become the active app because of this click.
    root.sendLine("{\"cmd\":\"open-ui\",\"page\":\"settings\"}\n")
  }

  function queryStatus() {
    root.sendLine("{\"cmd\":\"status\"}\n")
  }

  // Status file written by openatatd on state change. Watched — no GPU
  // layer-shell, no Waybar JSON module, no polling the compositor.
  FileView {
    id: statusFile
    path: root.statusPath
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
  }

  Connections {
    target: statusFile
    function onTextChanged() {
      root.applyStatusText(root.fileText(statusFile))
    }
  }

  // One line in, one JSON line out. Never `{"cmd":"trigger"}`.
  Socket {
    id: ipc
    path: root.socketPath
    connected: false
    onConnectedChanged: {
      if (!connected)
        return
      if (root.pendingLine && root.pendingLine.length) {
        write(root.pendingLine)
        flush()
        root.pendingLine = ""
      }
    }
    parser: SplitParser {
      onRead: function(line) {
        root.applyStatusText(line)
      }
    }
  }

  Timer {
    interval: 4000
    running: !root.daemonSeen
    repeat: true
    onTriggered: root.queryStatus()
  }

  Component.onCompleted: queryStatus()

  Text {
    id: label
    anchors.centerIn: parent
    text: root.chipText
    color: root.chipColor
    opacity: root.chipOpacity
    font.family: bar && bar.fontFamily ? bar.fontFamily : "monospace"
    font.pixelSize: 12
    renderType: Text.NativeRendering
  }

  MouseArea {
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    acceptedButtons: Qt.LeftButton
    onEntered: {
      if (root.bar && root.bar.showTooltip)
        root.bar.showTooltip(root, root.chipTooltip)
    }
    onExited: {
      if (root.bar && root.bar.hideTooltip)
        root.bar.hideTooltip(root)
    }
    onClicked: {
      if (root.bar && root.bar.hideTooltip)
        root.bar.hideTooltip(root)
      root.openSettings()
    }
  }
}
