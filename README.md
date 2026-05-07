# proc-dash

Process dashboard TUI with dedicated GPU and NPU views.

## Features

- Three tabs: Processes, GPU, NPU
- Live CPU, RAM, and GPU utilization with sparkline history (60s window)
- Process tree view
- Sort by PID, user, name, CPU%, memory%, RSS, state, or runtime
- Process filter with live search
- Kill, signal (TERM/KILL/STOP/CONT/HUP), and renice from the TUI
- Phosphor-green terminal aesthetic
- 500ms refresh interval, reads directly from `/proc`

## Install

```
cargo build --release
```

Binary lands at `target/release/proc-dash`.

## Usage

```
proc-dash
```

## Keybindings

| Key             | Action                          |
|-----------------|---------------------------------|
| `Tab` / `S-Tab` | Cycle tabs                     |
| `F1` `F2` `F3` | Jump to Processes / GPU / NPU   |
| `j` / `k`      | Move selection down / up        |
| `PgUp` / `PgDn`| Page scroll                     |
| `1`-`8`         | Sort by column (toggles asc/desc) |
| `/`             | Filter processes by name        |
| `t`             | Toggle tree view                |
| `g`             | Toggle sparkline graphs         |
| `K`             | Kill selected process (confirm) |
| `s`             | Signal menu                     |
| `r`             | Renice selected process         |
| `Esc`           | Cancel / clear filter           |
| `q` / `Ctrl-C`  | Quit                           |

---

Built with Rust + ratatui.
