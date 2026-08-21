# fcitx5-openatat

Fcitx5 module that implements the product `@@` trigger. It watches **committed**
text (not preedit), skips secure fields every key, swallows the two `@`
characters, and sends `{"cmd":"trigger","source":"ime",...}` to
`$XDG_RUNTIME_DIR/openatat/trigger.sock`.

The native applet (`openatatd`) still owns the overlay. This addon only detects
and swallows `@@`. A Hyprland global bind is not the product path.

The last-two-char / compose / secure state machine is the same contract as
`ImeFilter` in `crates/openatatd` (`src/openatat_filter.*` here). The addon is
the Fcitx5 adapter, not a second detector.

## Omarchy / Arch install

```bash
sudo pacman -S --needed fcitx5 fcitx5-gtk fcitx5-qt extra-cmake-modules cmake ninja pkgconf gcc

# From the OpenAtat repo root
cd ime/fcitx5-openatat
cmake -B build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX=/usr \
  -DCMAKE_CXX_COMPILER=g++
cmake --build build

# State-machine tests: no display, no Fcitx5 session
./build/openatat-filter-test

# Link step only happens when Fcitx5 headers were found. If CMake warned that
# Fcitx5Core is missing, do not install — that is not a successful addon build.
sudo cmake --install build

# Load the module
fcitx5 -r
```

User-local prefix (no root):

```bash
cmake -B build -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_INSTALL_PREFIX="$HOME/.local"
cmake --build build
cmake --install build
fcitx5 -r
```

`~/.local/lib/fcitx5/libopenatat.so` and `~/.local/share/fcitx5/addon/openatat.conf`
are on Fcitx5's XDG search path.

### Packages

| Distro | Build | Runtime (so GTK/Qt fields use Fcitx5) |
| --- | --- | --- |
| Arch / Omarchy | `fcitx5` (headers ship in the same package), `extra-cmake-modules`, `cmake`, `ninja`, `pkgconf`, `gcc` | `fcitx5`, `fcitx5-gtk`, `fcitx5-qt` |
| Fedora | `fcitx5-devel`, `extra-cmake-modules` | `fcitx5`, `fcitx5-gtk`, `fcitx5-qt` |
| Debian / Ubuntu | `libfcitx5core-dev` / `fcitx5-dev`, `extra-cmake-modules` | `fcitx5`, `fcitx5-frontend-gtk4`, `fcitx5-frontend-qt6` |

`dbus-1` (`libdbus-1-dev` / `dbus`) is optional. When present, the addon also
walks AT-SPI `PasswordText` every key (never cached). The IME-native equivalent
is Fcitx5 `CapabilityFlag::Password \| Sensitive`, which is always probed.

## Enable

`OnDemand=False`, so Fcitx5 loads OpenAtat at startup unless you disabled it.

- `fcitx5-configtool` → **Addons** → **OpenAtat** → on
- Or confirm `~/.config/fcitx5/conf/` has not disabled `openatat`

Restart after install: `fcitx5 -r`.

## Verify

```bash
# terminal A — native applet
openatatd

# Type @@ in any text field (kitty, firefox, a GTK entry, …).
# The two @ characters should disappear and the OpenAtat popover should open.
# Compose (CJK preedit containing @@) must not fire.
# A password field must not fire.
```

If `openatatd` is not running, the addon still swallows `@@` and fails quietly
(no crash, no dialog). Start the daemon later and type `@@` again.

Dev paths `--demo` and `openatatd trigger` keep working without this addon.

## CMake without Fcitx5 headers

On a headless/CI machine that lacks Fcitx5:

```bash
cmake -B build -S ime/fcitx5-openatat
cmake --build build --target openatat-filter-test
ctest --test-dir build --output-on-failure
```

The filter test target always builds. The shared-library addon is skipped with
a CMake warning — that is not a successful `fcitx5` link.
