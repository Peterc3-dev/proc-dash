use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use std::time::Duration;

use crate::app::{App, InputMode, Signal, SortColumn, SortDir, Tab};

/// Poll for input events. Returns true if an event was handled.
pub fn handle_events(app: &mut App) -> std::io::Result<bool> {
    if event::poll(Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            // Clear status message on any keypress
            app.status_msg = None;
            handle_key(app, key);
            return Ok(true);
        }
    }
    Ok(false)
}

fn handle_key(app: &mut App, key: KeyEvent) {
    match app.input_mode {
        InputMode::Normal => handle_normal(app, key),
        InputMode::Filter => handle_filter(app, key),
        InputMode::KillConfirm => handle_kill_confirm(app, key),
        InputMode::SignalMenu => handle_signal_menu(app, key),
        InputMode::ReniceInput => handle_renice(app, key),
    }
}

fn handle_normal(app: &mut App, key: KeyEvent) {
    // Ctrl-C always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        app.quit = true;
        return;
    }

    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('Q') => app.quit = true,

        // Navigation
        KeyCode::Up | KeyCode::Char('k') => app.move_up(),
        KeyCode::Down | KeyCode::Char('j') => app.move_down(),
        KeyCode::PageUp => app.page_up(),
        KeyCode::PageDown => app.page_down(),
        KeyCode::Home => app.selected = 0,
        KeyCode::End => {
            let len = app.visible_len();
            if len > 0 {
                app.selected = len - 1;
            }
        }

        // Tabs
        KeyCode::Tab => app.next_tab(),
        KeyCode::BackTab => app.prev_tab(),
        KeyCode::F(1) => {
            app.tab = Tab::Processes;
            app.selected = 0;
        }
        KeyCode::F(2) => {
            app.tab = Tab::Gpu;
            app.selected = 0;
        }
        KeyCode::F(3) => {
            app.tab = Tab::Npu;
            app.selected = 0;
        }

        // Sort (1-8 keys)
        KeyCode::Char('1') => toggle_sort(app, SortColumn::Pid),
        KeyCode::Char('2') => toggle_sort(app, SortColumn::User),
        KeyCode::Char('3') => toggle_sort(app, SortColumn::Name),
        KeyCode::Char('4') => toggle_sort(app, SortColumn::Cpu),
        KeyCode::Char('5') => toggle_sort(app, SortColumn::Mem),
        KeyCode::Char('6') => toggle_sort(app, SortColumn::Rss),
        KeyCode::Char('7') => toggle_sort(app, SortColumn::State),
        KeyCode::Char('8') => toggle_sort(app, SortColumn::Runtime),

        // Filter
        KeyCode::Char('/') => {
            app.input_mode = InputMode::Filter;
            app.filter_text.clear();
        }

        // Tree view
        KeyCode::Char('t') => app.tree_view = !app.tree_view,

        // Graph toggle
        KeyCode::Char('g') => app.show_graphs = !app.show_graphs,

        // Kill
        KeyCode::Char('K') => {
            if app.selected_pid().is_some() {
                app.input_mode = InputMode::KillConfirm;
            }
        }

        // Signal menu
        KeyCode::Char('s') => {
            if app.selected_pid().is_some() {
                app.input_mode = InputMode::SignalMenu;
                app.signal_menu_idx = 0;
            }
        }

        // Renice
        KeyCode::Char('r') => {
            if app.selected_pid().is_some() {
                app.input_mode = InputMode::ReniceInput;
                app.renice_text.clear();
            }
        }

        _ => {}
    }
}

fn handle_filter(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.filter_text.clear();
        }
        KeyCode::Enter => {
            app.input_mode = InputMode::Normal;
            // filter stays applied
        }
        KeyCode::Backspace => {
            app.filter_text.pop();
        }
        KeyCode::Char(c) => {
            app.filter_text.push(c);
        }
        _ => {}
    }
}

fn handle_kill_confirm(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            app.send_signal(Signal::Kill);
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        _ => {}
    }
}

fn handle_signal_menu(app: &mut App, key: KeyEvent) {
    let signals = Signal::all();
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.signal_menu_idx > 0 {
                app.signal_menu_idx -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.signal_menu_idx < signals.len() - 1 {
                app.signal_menu_idx += 1;
            }
        }
        KeyCode::Enter => {
            let sig = signals[app.signal_menu_idx];
            app.send_signal(sig);
        }
        _ => {}
    }
}

fn handle_renice(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.renice_text.clear();
        }
        KeyCode::Enter => {
            app.do_renice();
        }
        KeyCode::Backspace => {
            app.renice_text.pop();
        }
        KeyCode::Char(c) if c.is_ascii_digit() || c == '-' => {
            app.renice_text.push(c);
        }
        _ => {}
    }
}

fn toggle_sort(app: &mut App, col: SortColumn) {
    if app.sort_col == col {
        app.sort_dir = match app.sort_dir {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        };
    } else {
        app.sort_col = col;
        // Default to desc for numeric, asc for text
        app.sort_dir = match col {
            SortColumn::User | SortColumn::Name | SortColumn::State | SortColumn::Pid => {
                SortDir::Asc
            }
            _ => SortDir::Desc,
        };
    }
}
