# Auto-type of the selected item (fork)

KeePass-style "Perform Auto-Type" for the Bitwarden desktop app on Windows. This exists only in
the fork `red-one1/bitwarden-clients` (branch `keepass-autotype`), not in upstream Bitwarden.

## Use

1. Click into the target field (e.g. the user name box) of the app or web page to log in to.
2. Switch to Bitwarden and select the item.
3. Press **Ctrl+Shift+V** (or Edit > "Auto-type into previous window").

Bitwarden gives the focus back to the window below it (like KeePass), waits until you have let go
of Ctrl and Shift, and types the item's sequence. It does not check where the focus is, so make
sure the right field is focused.

If the item asks for the master password again ("Master password re-prompt"), Bitwarden asks
first.

## Sequence

An item's sequence is its custom field named `autotype` (any case; text or hidden). Logins without
it use `{USERNAME}{TAB}{PASSWORD}{ENTER}`; other item types need the field.

The syntax is KeePass 2.x (https://keepass.info/help/base/autotype.html), codes are
case-insensitive:

| Kind        | Codes                                                                                                                                                                                                                                                                                                                                                                            |
| ----------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Item values | `{USERNAME}` `{PASSWORD}` `{TITLE}` `{URL}` (first URI) `{NOTES}` `{TOTP}` (also `{TIMEOTP}`) `{S:field name}` (custom field)                                                                                                                                                                                                                                                    |
| Keys        | `{TAB}` `{ENTER}` or `~` `{UP}` `{DOWN}` `{LEFT}` `{RIGHT}` `{HOME}` `{END}` `{PGUP}` `{PGDN}` `{INSERT}`/`{INS}` `{DELETE}`/`{DEL}` `{SPACE}` `{BACKSPACE}`/`{BS}`/`{BKSP}` `{ESC}` `{BREAK}` `{CAPSLOCK}` `{WIN}`/`{LWIN}` `{RWIN}` `{APPS}` `{HELP}` `{NUMLOCK}` `{PRTSC}` `{SCROLLLOCK}` `{F1}`-`{F16}` `{ADD}` `{SUBTRACT}` `{MULTIPLY}` `{DIVIDE}` `{NUMPAD0}`-`{NUMPAD9}` |
| Repeat      | `{TAB 3}`, `{+ 2}`                                                                                                                                                                                                                                                                                                                                                               |
| Modifiers   | `+` Shift, `^` Ctrl, `%` Alt, for the next key, character or `( )` group: `^a`, `%{F4}`, `+(abc)`                                                                                                                                                                                                                                                                                |
| Literal     | `{+}` `{^}` `{%}` `{~}` `{(}` `{)}` `{[}` `{]}` `{{}` `{}}`                                                                                                                                                                                                                                                                                                                      |
| Commands    | `{DELAY n}` (pause n ms), `{DELAY=n}` (n ms between keys from here, default 10), `{CLEARFIELD}`, `{VKEY n}` (virtual key, decimal or `0x..`)                                                                                                                                                                                                                                     |

Item values are typed literally (a password containing `{TAB}` or `+` is typed as is). Text is sent
as Unicode key packets, so Caps Lock and the keyboard layout don't change it.

Not supported (the item is not typed at all, an error is shown): `{APPACTIVATE}`, `{BEEP}`,
`{VKEY n D|U}`, other KeePass placeholders, and the global auto-type hotkey / window matching.

## Limits

- Windows only. Windows blocks typing into apps that run as administrator unless Bitwarden does too.
- Remote desktops and VMs must use the same keyboard layout for characters typed with modifiers.

## Code

| Part                                                                    | What it does                                                                |
| ----------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `autotype-sequence.ts` (+ spec)                                         | Parses the sequence and resolves placeholders into steps                    |
| `selected-item-autotype.ts`                                             | Picks the item's sequence and values (view-password, linked fields)         |
| `vault-v3/vault.component.ts` (`autoType`)                              | Handles the menu command: re-prompt, TOTP, sends the steps over IPC         |
| `main/menu/menu.edit.ts`                                                | Edit menu entry and the Ctrl+Shift+V accelerator (Windows only)             |
| `autofill/main/main-selected-item-autotype.service.ts`                  | Main process: calls the native module                                       |
| `desktop_native/napi/src/autotype_sequence.rs`                          | N-API binding                                                               |
| `desktop_native/autotype/src/sequence/` (`mod.rs` tested, `windows.rs`) | Key strokes, focusing the previous window, releasing modifiers, `SendInput` |

`main/updater.main.ts` is patched so this build never updates itself to the official release.

## Build

`.github/workflows/fork-build-desktop-autotype.yml` runs on every push to `keepass-autotype` and
uploads an unsigned NSIS installer and a portable exe (artifact
`bitwarden-desktop-autotype-windows-x64`). Windows SmartScreen warns about the unsigned files.

Keeping up with upstream: `git fetch upstream && git rebase upstream/main` on this branch, then push.
