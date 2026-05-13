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

### Portal first-run

The very first time you trigger a paste through the portal backend, your
desktop pops up a dialog along the lines of "plippet wants to control your
keyboard." Approve it once; plippet stores the portal's `restore_token` at
`~/.config/plippet/portal_token` and subsequent invocations are silent.

To re-trigger the prompt later (e.g. you reset your portal permissions),
delete `~/.config/plippet/portal_token`.

## Runtime dependencies

Install via your package manager:

- `wl-clipboard` — required (provides `wl-copy`)
- One picker:
  - `fuzzel` (default)
  - `wofi`
  - `rofi-wayland`
  - `bemenu`
- For `--paste`, one of:
  - `wtype` — on wlroots compositors only
  - `xdg-desktop-portal` plus a backend (`xdg-desktop-portal-gnome`,
    `xdg-desktop-portal-kde`, or `xdg-desktop-portal-wlr`) — for the portal
    backend, which Linked binaries call into automatically

Run `plippet check` to see which optional tools are present and which
backend `auto` will resolve to on your current desktop.

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
plippet pick   [--picker fuzzel|wofi|rofi|bemenu] [--paste] [--paste-backend auto|wtype|portal]
plippet list
plippet insert <key> [--paste] [--paste-backend auto|wtype|portal]
plippet check  [--strict]
```

- **pick** — open the fuzzy picker; the selected snippet is copied (and
  pasted if `--paste`). Cancelling the picker (ESC) exits 0 silently.
- **list** — print all snippets as `key<TAB>name`.
- **insert** — resolve a snippet by key and copy it (and paste if `--paste`).
- **check** — validate the config, report which backend `auto` will use, and
  list tool availability. Exits 0 as long as the config is valid. Pass
  `--strict` to also fail when required runtime tools (currently just
  `wl-copy`) are missing — useful in CI.

## Compositor bindings

### GNOME (Settings → Keyboard → Custom Shortcuts)

Bind a shortcut (e.g. `Super+;`) to:

```
plippet pick --paste
```

`--paste-backend auto` will pick `portal` on GNOME automatically.

### KDE Plasma (System Settings → Shortcuts → Custom Shortcuts)

Same — bind to `plippet pick --paste`.

### Sway (`~/.config/sway/config`)

```
bindsym $mod+semicolon exec plippet pick --paste
```

### Hyprland (`~/.config/hypr/hyprland.conf`)

```
bind = SUPER, SEMICOLON, exec, plippet pick --paste
```

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
