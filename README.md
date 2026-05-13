# plippet

A small Wayland-friendly hotkey snippet picker.

You bind a compositor shortcut to `plippet pick --paste`. It opens a fuzzy
picker, you choose a snippet, the snippet body is copied to the Wayland
clipboard, and (with `--paste`) `Ctrl+V` is synthesised into the focused
window.

## What this is NOT

- **Not a global text-expansion daemon.** plippet never observes typed input.
- **Not a Wayland clipboard manager.** It hands off to `wl-copy`.
- **Not a keylogger.** The compositor owns the hotkey binding.

## Paste backends

`--paste` works on every major Wayland desktop, but the mechanism differs:

| Backend  | Works on                          | How                                           |
|----------|-----------------------------------|-----------------------------------------------|
| `wtype`  | wlroots (Sway, Hyprland, river…)  | `virtual-keyboard-unstable-v1` protocol       |
| `portal` | GNOME, KDE Plasma, wlroots        | XDG Desktop Portal `RemoteDesktop` interface  |
| `auto`   | both — picks based on environment | reads `$XDG_CURRENT_DESKTOP`                  |

`--paste-backend` defaults to `auto`. The heuristic picks `portal` when
`XDG_CURRENT_DESKTOP` contains `GNOME`, `KDE`, `Plasma`, or `KWin`, and
falls back to `wtype` otherwise. Override with `--paste-backend wtype` or
`--paste-backend portal` if the heuristic guesses wrong.

On X11, plippet captures the focused window before opening the picker and
re-activates that window immediately before pasting. That is what makes
`plippet pick --paste` behave like a real snippet inserter on XFCE/Xorg.

### Portal first-run

The very first time you trigger a paste through the portal backend, your
desktop pops up a dialog along the lines of "plippet wants to control your
keyboard." Approve it once; plippet stores the portal's `restore_token` at
`~/.config/plippet/portal_token` and subsequent invocations are silent.

To re-trigger the prompt later (e.g. you reset your portal permissions),
delete `~/.config/plippet/portal_token`.

## Runtime dependencies

Plippet auto-detects whether you're on a Wayland or X11 session
(`XDG_SESSION_TYPE` + `WAYLAND_DISPLAY`) and picks the appropriate clipboard
and paste tools. Install what matches your session:

**Wayland sessions** (Sway, Hyprland, GNOME, KDE Plasma, river, labwc, …):

- `wl-clipboard` — required (provides `wl-copy`)
- For `--paste`, one of:
  - `wtype` — wlroots compositors only (Sway/Hyprland/river/labwc)
  - `xdg-desktop-portal` plus a backend (`xdg-desktop-portal-gnome`,
    `xdg-desktop-portal-kde`, or `xdg-desktop-portal-wlr`) — used by the
    portal backend, called via D-Bus automatically

**X11 sessions** (XFCE, classic LXQt/Cinnamon, GNOME-on-Xorg, …):

- `xclip` — required (clipboard)
- `xdotool` — required for X11 snippet insertion via `--paste`; plippet
  uses it both to re-focus the original X11 window and to synthesize the
  paste/type event

**Picker** — plippet ships a built-in fuzzy picker (egui-based) that works
on every compositor without any external dependency, so you don't *have*
to install anything. External pickers are still supported if you prefer
their look or speed:

- `builtin` — bundled with plippet, requires the `gui` feature (default-on);
  works on every Wayland compositor (GNOME, KDE, wlroots) and on X11.
- `fuzzel` — wlroots only (uses `wlr-layer-shell`); will NOT run on GNOME or
  KDE Plasma
- `wofi` — wlroots only (same reason as fuzzel)
- `bemenu` — wlroots Wayland mode; has X11/curses backends too
- `rofi` — plain X11 rofi works via XWayland everywhere; **`rofi-wayland`
  (the lbonn fork) also needs `wlr-layer-shell` and so doesn't work on
  GNOME/KDE**

`--picker auto` (the default) picks per-environment:

- **wlroots Wayland** (Sway, Hyprland, river, labwc): `fuzzel` → `wofi` →
  `bemenu` → `rofi` → `builtin`. Whatever's installed first wins.
- **GNOME / KDE Wayland**: `builtin` first (categorically works) → `rofi`
  (which may be the broken rofi-wayland fork on your distro). The built-in
  is preferred because we can't tell plain rofi from rofi-wayland by binary
  name alone.
- **X11**: `rofi` → `builtin`. Plain X11 rofi is fast and reliable here.

Pass an explicit `--picker <name>` to override. `--picker builtin` always
works.

Run `plippet check` to see which session was detected, which paste backend
`auto` will resolve to, which picker `auto` will resolve to, and which
external tools are present on your system.

## Build & install

```sh
cargo build --release
cp target/release/plippet ~/.local/bin/
```

Make sure `~/.local/bin` is on your `$PATH`.

## Configuration

Path: `~/.config/plippet/snippets.toml`

```toml
[[snippet]]
key = "sig"
name = "Email signoff"
body = """
Best,
J
"""

[[snippet]]
key = "addr"
name = "Address"
body = """
123 Example Street
Berlin
"""

[[snippet]]
key = "today"
name = "Today's date"
command = "date +%Y-%m-%d"
```

Each snippet must have:

- a non-empty `key` (unique across the file)
- a non-empty `name` (shown in the picker)
- **exactly one** of `body` or `command`

`body` is copied verbatim. `command` is run via `sh -c`; its stdout is copied,
with **trailing newlines** trimmed (so `date +%Y-%m-%d` pastes cleanly inline).
Other trailing whitespace is preserved.

A starter file is included at `examples/snippets.toml`:

```sh
mkdir -p ~/.config/plippet
cp examples/snippets.toml ~/.config/plippet/snippets.toml
```

## Commands

```
plippet pick   [--picker auto|builtin|fuzzel|wofi|rofi|bemenu] [--paste] [--paste-backend auto|wtype|portal|xdotool] [--paste-keys ctrl-v|ctrl-shift-v] [--paste-mode chord|type]
plippet list
plippet insert <key> [--paste] [--paste-backend auto|wtype|portal|xdotool] [--paste-keys ctrl-v|ctrl-shift-v] [--paste-mode chord|type]
plippet check  [--strict]
plippet edit                          # requires the `gui` feature (default-on)
```

- **pick** — open the fuzzy picker; the selected snippet is copied (and
  pasted if `--paste`). Cancelling the picker (ESC) exits 0 silently.
- **list** — print all snippets as `key<TAB>name`.
- **insert** — resolve a snippet by key and copy it (and paste if `--paste`).
- **`--paste-keys`** (on `pick` and `insert`) — choose the synthesized
  keystroke. Default `ctrl-v` works in browsers, editors, chat apps, and
  most GUI text inputs. Pass `ctrl-shift-v` if your binding is meant to
  paste into a terminal (GNOME Terminal, Foot, Kitty, Alacritty all use
  Ctrl+Shift+V for paste). **Note:** mutter on GNOME drops the Shift
  modifier from synthesized two-modifier chords, so `ctrl-shift-v` won't
  actually reach the focused window there — use `--paste-mode type` for
  GNOME terminals instead.
- **`--paste-mode`** (on `pick` and `insert`) — how the snippet reaches
  the focused window. Default `chord` synthesizes a paste shortcut and is
  fast. `type` types each character of the snippet directly via key
  synthesis — slower for long snippets and ASCII-only, but works in any
  focused text input regardless of paste bindings (terminals, password
  fields, search boxes) and sidesteps mutter's chord-dropping bug.
- **check** — validate the config, report which backend `auto` will use, and
  list tool availability. Exits 0 as long as the config is valid. Pass
  `--strict` to also fail when required runtime tools (currently just
  `wl-copy`) are missing — useful in CI.
- **edit** — open the snippet-manager GUI (see next section).

## Snippet manager GUI

`plippet edit` opens a small window for adding, editing, renaming, and
deleting snippets without hand-editing TOML.

- Click **➕ Add snippet** to create a new row. It's pre-filled with a fresh
  unique key (`new`, `new-2`, `new-3`, …) so the row is valid as-is — just
  retype the key/name to taste.
- The **✕** button on each row asks for confirmation before deleting.
- **↻ Revert** reloads from disk; if you have unsaved changes it asks first.
- **💾 Save** (or `Ctrl+S`) writes `~/.config/plippet/snippets.toml`
  atomically (write to a temp file, then rename).
- Closing the window with unsaved changes pops a Save / Discard & quit /
  Cancel modal — you won't lose work by missing the dirty indicator.
- The bottom status bar shows live validation; Save is disabled while
  validation is failing.

If you don't want the GUI in your build, compile with
`cargo build --release --no-default-features`. The `edit` subcommand will be
absent but everything else works unchanged.

## Compositor bindings

### GNOME (Settings → Keyboard → Custom Shortcuts)

GNOME's custom shortcuts run the command in a non-interactive, non-login
shell — `~/.bashrc` is not sourced, so `~/.local/bin/` may not be on
`$PATH`. **Use the absolute path to the binary** in your binding:

```
/home/<you>/.local/bin/plippet pick --paste
```

`--picker auto` (default) detects GNOME Wayland's lack of `wlr-layer-shell`
and uses the built-in egui picker — no external picker to install.
`--paste-backend auto` picks the portal backend (you'll see a one-time
"plippet wants to control your keyboard" dialog the first time — approve
it; the grant persists via a restore token).

**If you want to paste into a terminal on GNOME**, the chord approach
won't work — mutter's portal silently drops the Shift modifier from
synthesized two-modifier chords, and terminals don't accept plain Ctrl+V
as paste. Use `--paste-mode type` instead, which types the snippet
character by character:

```
/home/<you>/.local/bin/plippet pick --paste --paste-mode type
```

You can bind two different hotkeys — one for GUI inputs (default chord
mode, fast) and one for terminals (`--paste-mode type`, slower but works
everywhere).

### KDE Plasma (System Settings → Shortcuts → Custom Shortcuts)

Same as GNOME — Plasma's Wayland session also lacks `wlr-layer-shell`, and
`--picker auto` falls back to the built-in picker:

```
plippet pick --paste
```

### Sway (`~/.config/sway/config`)

```
bindsym $mod+semicolon exec plippet pick --paste
```

### Hyprland (`~/.config/hypr/hyprland.conf`)

```
bind = SUPER, SEMICOLON, exec, plippet pick --paste
```

### XFCE (Settings → Keyboard → Application Shortcuts)

Add a new shortcut and set the command to:

```
plippet pick --paste
```

Then press the key combination you want to bind (e.g. `Super+;`).

Equivalent from the command line:

```sh
xfconf-query -c xfce4-keyboard-shortcuts \
  -p '/commands/custom/<Super>semicolon' \
  -n -t string -s 'plippet pick --paste'
```

XFCE typically runs on X11, and plippet detects this via
`XDG_SESSION_TYPE` / absent `WAYLAND_DISPLAY`. With `xclip` and `xdotool`
installed, `plippet pick --paste` works end-to-end on X11 — clipboard via
`xclip`, then focus is restored to the original window and paste happens via
`xdotool`. `rofi` is X11-native here, so the picker also works without any
tricks.

If you're running XFCE on a Wayland session (e.g. via `labwc`), the Wayland
tools (`wl-clipboard` + `wtype` / portal) are used instead, automatically.

Reload your compositor/session config, then press the binding inside a text
field. On GNOME/KDE, the first press will prompt for portal permission;
approve it and subsequent presses are silent.

## Security notes

- **Command snippets execute shell commands.** Only put commands in your
  config that you would otherwise run yourself.
- plippet does **not** read global keystrokes; the compositor owns the
  hotkey.
- The portal backend asks the system once for permission to inject
  keystrokes; that grant is stored as a restore-token in
  `~/.config/plippet/portal_token`. Delete the file to revoke. Some portal
  implementations also surface the grant in their own settings UI.
- Snippet contents pass through the Wayland clipboard. Clipboard managers
  (e.g. `cliphist`, `clipman`, KDE's Klipper) may record them — don't store
  secrets in snippets unless that's acceptable in your setup.

## Tests

```sh
cargo test
```

Covers config validation, body-snippet resolution, and command-snippet
trimming/error propagation. Picker / clipboard / paste paths are not unit
tested — verify them interactively with `plippet check` and a sample
binding.
