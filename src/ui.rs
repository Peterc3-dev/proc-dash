use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, Gauge, Paragraph, Row, Sparkline, Table, Tabs, Wrap,
    },
    Frame,
};

use crate::app::{App, InputMode, Signal, SortColumn, Tab};

// Phosphor green theme
const GREEN: Color = Color::Rgb(0, 255, 200);
const DIM_GREEN: Color = Color::Rgb(0, 200, 156);
const DARK_GREEN: Color = Color::Rgb(0, 50, 40);
const YELLOW: Color = Color::Rgb(255, 230, 50);
const RED: Color = Color::Rgb(255, 60, 60);
const CYAN: Color = Color::Rgb(100, 255, 240);
const WHITE: Color = Color::Rgb(220, 255, 245);

pub fn draw(f: &mut Frame, app: &App) {
    let size = f.area();

    // Main layout: summary | tabs | table | [graphs] | status
    let mut constraints = vec![
        Constraint::Length(4),  // summary bar
        Constraint::Length(1),  // tab bar
        Constraint::Min(10),   // main table
    ];
    if app.show_graphs {
        // Steal some space from the table for graphs
        constraints[2] = Constraint::Min(6);
        constraints.push(Constraint::Length(8)); // graph panel
    }
    constraints.push(Constraint::Length(1)); // status bar

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(size);

    draw_summary(f, app, chunks[0]);
    draw_tabs(f, app, chunks[1]);

    let table_area = chunks[2];
    match app.tab {
        Tab::Processes => draw_process_table(f, app, table_area),
        Tab::Gpu => draw_gpu_table(f, app, table_area),
        Tab::Npu => draw_npu_table(f, app, table_area),
    }

    let status_idx = if app.show_graphs {
        draw_graphs(f, app, chunks[3]);
        4
    } else {
        3
    };

    draw_status(f, app, chunks[status_idx]);

    // Overlays
    match app.input_mode {
        InputMode::Filter => draw_filter_overlay(f, app, size),
        InputMode::KillConfirm => draw_kill_confirm(f, app, size),
        InputMode::SignalMenu => draw_signal_menu(f, app, size),
        InputMode::ReniceInput => draw_renice_overlay(f, app, size),
        InputMode::Normal => {}
    }
}

fn draw_summary(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM_GREEN))
        .title(Span::styled(" proc-dash ", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(inner);

    // CPU gauge
    draw_gauge(
        f,
        cols[0],
        "CPU",
        app.summary.cpu_percent,
        usage_color(app.summary.cpu_percent),
    );

    // RAM gauge
    draw_gauge(
        f,
        cols[1],
        "RAM",
        app.summary.mem_percent,
        usage_color(app.summary.mem_percent),
    );

    // VRAM gauge
    let vram_pct = if app.summary.vram_total_kb > 0 {
        (app.summary.vram_used_kb as f64 / app.summary.vram_total_kb as f64) * 100.0
    } else {
        0.0
    };
    draw_gauge(f, cols[2], "VRAM", vram_pct, usage_color(vram_pct));

    // GTT gauge
    let gtt_pct = if app.summary.gtt_total_kb > 0 {
        (app.summary.gtt_used_kb as f64 / app.summary.gtt_total_kb as f64) * 100.0
    } else {
        0.0
    };
    draw_gauge(f, cols[3], "GTT", gtt_pct, usage_color(gtt_pct));

    // Load + uptime
    let load_text = format!(
        "Load: {:.2} {:.2} {:.2}\nUp: {}",
        app.summary.load_avg_1,
        app.summary.load_avg_5,
        app.summary.load_avg_15,
        format_duration_short(app.summary.uptime_secs),
    );
    let load_para = Paragraph::new(load_text)
        .style(Style::default().fg(DIM_GREEN));
    f.render_widget(load_para, cols[4]);
}

fn draw_gauge(f: &mut Frame, area: Rect, label: &str, percent: f64, color: Color) {
    let pct = percent.clamp(0.0, 100.0) as u16;
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(color).bg(DARK_GREEN))
        .percent(pct)
        .label(format!("{} {:.1}%", label, percent));
    f.render_widget(gauge, area);
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = vec![
        Line::from(" [1] Processes "),
        Line::from(" [2] GPU "),
        Line::from(" [3] NPU "),
    ];
    let selected = match app.tab {
        Tab::Processes => 0,
        Tab::Gpu => 1,
        Tab::Npu => 2,
    };
    let tabs = Tabs::new(titles)
        .select(selected)
        .style(Style::default().fg(DIM_GREEN))
        .highlight_style(Style::default().fg(GREEN).add_modifier(Modifier::BOLD))
        .divider("|");
    f.render_widget(tabs, area);
}

fn draw_process_table(f: &mut Frame, app: &App, area: Rect) {
    let header_labels = [
        ("PID", SortColumn::Pid),
        ("USER", SortColumn::User),
        ("NAME", SortColumn::Name),
        ("CPU%", SortColumn::Cpu),
        ("MEM%", SortColumn::Mem),
        ("RSS", SortColumn::Rss),
        ("S", SortColumn::State),
        ("TIME", SortColumn::Runtime),
    ];

    let header_cells: Vec<Span> = header_labels
        .iter()
        .map(|(label, col)| {
            let arrow = if *col == app.sort_col {
                match app.sort_dir {
                    crate::app::SortDir::Asc => " ^",
                    crate::app::SortDir::Desc => " v",
                }
            } else {
                ""
            };
            Span::styled(
                format!("{}{}", label, arrow),
                if *col == app.sort_col {
                    Style::default().fg(CYAN).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(GREEN)
                },
            )
        })
        .collect();

    let header = Row::new(header_cells)
        .style(Style::default().fg(GREEN).add_modifier(Modifier::BOLD))
        .height(1);

    // Determine visible rows
    let table_height = area.height.saturating_sub(3) as usize; // header + borders
    let total = app.processes.len();

    // Adjust scroll offset to keep selected visible
    let scroll = compute_scroll(app.selected, app.scroll_offset, table_height, total);

    let rows: Vec<Row> = app
        .processes
        .iter()
        .enumerate()
        .skip(scroll)
        .take(table_height)
        .map(|(i, p)| {
            let prefix = if app.tree_view {
                tree_prefix(p, &app.processes)
            } else {
                String::new()
            };

            let cpu_color = usage_color(p.cpu_percent);
            let mem_color = usage_color(p.mem_percent);

            let cells = vec![
                Span::styled(format!("{:>7}", p.pid), Style::default().fg(DIM_GREEN)),
                Span::styled(
                    format!("{:<8}", truncate(&p.user, 8)),
                    Style::default().fg(DIM_GREEN),
                ),
                Span::styled(
                    format!("{}{:<20}", prefix, truncate(&p.name, 20 - prefix.len().min(20))),
                    Style::default().fg(WHITE),
                ),
                Span::styled(
                    format!("{:>6.1}", p.cpu_percent),
                    Style::default().fg(cpu_color),
                ),
                Span::styled(
                    format!("{:>6.1}", p.mem_percent),
                    Style::default().fg(mem_color),
                ),
                Span::styled(
                    format!("{:>8}", format_kb(p.rss_kb)),
                    Style::default().fg(DIM_GREEN),
                ),
                Span::styled(
                    format!(" {} ", p.state),
                    Style::default().fg(state_color(p.state)),
                ),
                Span::styled(
                    format_duration(p.runtime),
                    Style::default().fg(DIM_GREEN),
                ),
            ];

            let style = if i == app.selected {
                Style::default().bg(DARK_GREEN).fg(GREEN)
            } else {
                Style::default()
            };

            Row::new(cells).style(style)
        })
        .collect();

    let widths = [
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Min(20),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(3),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM_GREEN))
                .title(Span::styled(
                    format!(
                        " Processes ({}) {} ",
                        total,
                        if !app.filter_text.is_empty() {
                            format!("[filter: {}]", app.filter_text)
                        } else {
                            String::new()
                        }
                    ),
                    Style::default().fg(GREEN),
                )),
        )
        .row_highlight_style(Style::default().bg(DARK_GREEN));

    f.render_widget(table, area);
}

fn draw_gpu_table(f: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec![
        Span::styled("PID", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(
            "NAME",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "VRAM",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "GTT",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
    ])
    .height(1);

    let rows: Vec<Row> = app
        .gpu_processes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == app.selected {
                Style::default().bg(DARK_GREEN)
            } else {
                Style::default()
            };
            Row::new(vec![
                Span::styled(format!("{:>7}", p.pid), Style::default().fg(DIM_GREEN)),
                Span::styled(
                    format!("{:<24}", truncate(&p.name, 24)),
                    Style::default().fg(WHITE),
                ),
                Span::styled(
                    format!("{:>10}", format_kb(p.vram_kb)),
                    Style::default().fg(CYAN),
                ),
                Span::styled(
                    format!("{:>10}", format_kb(p.gtt_kb)),
                    Style::default().fg(DIM_GREEN),
                ),
            ])
            .style(style)
        })
        .collect();

    let summary_text = format!(
        " GPU Processes ({}) | VRAM: {}/{} | GTT: {}/{} | Busy: {}% ",
        app.gpu_processes.len(),
        format_kb(app.summary.vram_used_kb),
        format_kb(app.summary.vram_total_kb),
        format_kb(app.summary.gtt_used_kb),
        format_kb(app.summary.gtt_total_kb),
        app.summary.gpu_busy_percent,
    );

    let widths = [
        Constraint::Length(8),
        Constraint::Min(24),
        Constraint::Length(11),
        Constraint::Length(11),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM_GREEN))
                .title(Span::styled(summary_text, Style::default().fg(GREEN))),
        );

    f.render_widget(table, area);
}

fn draw_npu_table(f: &mut Frame, app: &App, area: Rect) {
    let header = Row::new(vec![
        Span::styled("PID", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(
            "NAME",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "DEVICE",
            Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        ),
    ])
    .height(1);

    let rows: Vec<Row> = app
        .npu_processes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == app.selected {
                Style::default().bg(DARK_GREEN)
            } else {
                Style::default()
            };
            Row::new(vec![
                Span::styled(format!("{:>7}", p.pid), Style::default().fg(DIM_GREEN)),
                Span::styled(
                    format!("{:<24}", truncate(&p.name, 24)),
                    Style::default().fg(WHITE),
                ),
                Span::styled(
                    p.fd_path.clone(),
                    Style::default().fg(CYAN),
                ),
            ])
            .style(style)
        })
        .collect();

    let npu_status = if app.summary.npu_module_loaded {
        "amdxdna LOADED"
    } else {
        "amdxdna NOT LOADED"
    };

    let widths = [
        Constraint::Length(8),
        Constraint::Min(24),
        Constraint::Min(20),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(DIM_GREEN))
                .title(Span::styled(
                    format!(" NPU ({}) | {} ", app.npu_processes.len(), npu_status),
                    Style::default().fg(GREEN),
                )),
        );

    f.render_widget(table, area);
}

fn draw_graphs(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(area);

    draw_sparkline(f, cols[0], "CPU %", &app.cpu_history, GREEN);
    draw_sparkline(f, cols[1], "RAM %", &app.ram_history, CYAN);
    draw_sparkline(f, cols[2], "GPU %", &app.gpu_history, YELLOW);
}

fn draw_sparkline(f: &mut Frame, area: Rect, title: &str, data: &std::collections::VecDeque<f64>, color: Color) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(DIM_GREEN))
        .title(Span::styled(
            format!(" {} ", title),
            Style::default().fg(color),
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Convert f64 to u64 for sparkline (scale 0-100)
    let values: Vec<u64> = data.iter().map(|v| v.clamp(0.0, 100.0) as u64).collect();
    // Only take what fits in the width
    let width = inner.width as usize;
    let start = if values.len() > width {
        values.len() - width
    } else {
        0
    };

    let sparkline = Sparkline::default()
        .data(&values[start..])
        .max(100)
        .style(Style::default().fg(color));

    f.render_widget(sparkline, inner);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let msg = if let Some(ref m) = app.status_msg {
        m.clone()
    } else {
        match app.input_mode {
            InputMode::Normal => {
                "q:quit | Tab:switch | /:filter | k:kill | s:signal | r:renice | t:tree | g:graph | 1-8:sort"
                    .to_string()
            }
            InputMode::Filter => format!("Filter: {}_ (Enter to apply, Esc to cancel)", app.filter_text),
            InputMode::KillConfirm => "Kill selected process? (y/n)".to_string(),
            InputMode::SignalMenu => "Select signal: j/k to move, Enter to send, Esc to cancel".to_string(),
            InputMode::ReniceInput => format!("Nice value (-20..19): {}_ (Enter to apply)", app.renice_text),
        }
    };

    let style = if app.status_msg.is_some() {
        Style::default().fg(YELLOW)
    } else {
        Style::default().fg(DIM_GREEN)
    };

    let para = Paragraph::new(msg).style(style);
    f.render_widget(para, area);
}

fn draw_filter_overlay(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(40, 3, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(GREEN))
        .title(Span::styled(" Filter ", Style::default().fg(GREEN)));
    let text = Paragraph::new(format!("{}|", app.filter_text))
        .style(Style::default().fg(WHITE))
        .block(block);
    f.render_widget(text, popup);
}

fn draw_kill_confirm(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(50, 5, area);
    f.render_widget(Clear, popup);
    let pid_text = app
        .selected_pid()
        .map(|p| format!("PID {}", p))
        .unwrap_or_default();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(RED))
        .title(Span::styled(
            " Kill Confirmation ",
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ));
    let text = Paragraph::new(format!(
        "Send SIGKILL to {}?\n\nPress y to confirm, n or Esc to cancel",
        pid_text
    ))
    .style(Style::default().fg(WHITE))
    .wrap(Wrap { trim: true })
    .block(block);
    f.render_widget(text, popup);
}

fn draw_signal_menu(f: &mut Frame, app: &App, area: Rect) {
    let signals = Signal::all();
    let height = signals.len() as u16 + 2;
    let popup = centered_rect(30, height, area);
    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(CYAN))
        .title(Span::styled(
            " Send Signal ",
            Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines: Vec<Line> = signals
        .iter()
        .enumerate()
        .map(|(i, sig)| {
            let style = if i == app.signal_menu_idx {
                Style::default()
                    .fg(GREEN)
                    .bg(DARK_GREEN)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(DIM_GREEN)
            };
            Line::styled(format!(" {} ", sig.label()), style)
        })
        .collect();

    let para = Paragraph::new(lines);
    f.render_widget(para, inner);
}

fn draw_renice_overlay(f: &mut Frame, app: &App, area: Rect) {
    let popup = centered_rect(40, 3, area);
    f.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(CYAN))
        .title(Span::styled(" Renice ", Style::default().fg(CYAN)));
    let text = Paragraph::new(format!("Nice (-20..19): {}|", app.renice_text))
        .style(Style::default().fg(WHITE))
        .block(block);
    f.render_widget(text, popup);
}

// === Helpers ===

fn usage_color(pct: f64) -> Color {
    if pct >= 80.0 {
        RED
    } else if pct >= 40.0 {
        YELLOW
    } else {
        GREEN
    }
}

fn state_color(state: char) -> Color {
    match state {
        'R' => GREEN,
        'S' | 'I' => DIM_GREEN,
        'D' => YELLOW,
        'Z' => RED,
        'T' | 't' => CYAN,
        _ => DIM_GREEN,
    }
}

fn format_kb(kb: u64) -> String {
    if kb >= 1_048_576 {
        format!("{:.1}G", kb as f64 / 1_048_576.0)
    } else if kb >= 1024 {
        format!("{:.1}M", kb as f64 / 1024.0)
    } else {
        format!("{}K", kb)
    }
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{:>3}h{:02}m{:02}s", h, m, s)
    } else {
        format!("    {:02}m{:02}s", m, s)
    }
}

fn format_duration_short(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{}d {}h", d, h)
    } else {
        format!("{}h {}m", h, m)
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else if max > 1 {
        let truncated: String = s.chars().take(max - 1).collect();
        format!("{}~", truncated)
    } else if max == 1 {
        s.chars().next().map(|c| c.to_string()).unwrap_or_default()
    } else {
        String::new()
    }
}

fn tree_prefix(proc: &crate::proc_info::ProcessInfo, _all: &[crate::proc_info::ProcessInfo]) -> String {
    // Simple indent based on whether it has a parent that's also in the list
    if proc.ppid <= 1 {
        String::new()
    } else {
        "  ".to_string() // Could be made recursive for deep trees
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(
        x,
        y,
        width.min(area.width),
        height.min(area.height),
    )
}

fn compute_scroll(selected: usize, current_scroll: usize, page_size: usize, _total: usize) -> usize {
    if page_size == 0 {
        return 0;
    }
    if selected < current_scroll {
        selected
    } else if selected >= current_scroll + page_size {
        selected - page_size + 1
    } else {
        current_scroll
    }
}
