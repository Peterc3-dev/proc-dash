use std::collections::VecDeque;

use crate::proc_info::{
    self, GpuProcessInfo, NpuProcessInfo, ProcCollector, ProcessInfo, SystemSummary,
};

pub const HISTORY_LEN: usize = 120; // 60 seconds at 500ms

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Processes,
    Gpu,
    Npu,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortColumn {
    Pid,
    User,
    Name,
    Cpu,
    Mem,
    Rss,
    State,
    Runtime,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Filter,
    KillConfirm,
    SignalMenu,
    ReniceInput,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Term,
    Kill,
    Stop,
    Cont,
    Hup,
}

impl Signal {
    pub fn all() -> &'static [Signal] {
        &[Signal::Term, Signal::Kill, Signal::Stop, Signal::Cont, Signal::Hup]
    }

    pub fn label(self) -> &'static str {
        match self {
            Signal::Term => "SIGTERM (15)",
            Signal::Kill => "SIGKILL (9)",
            Signal::Stop => "SIGSTOP (19)",
            Signal::Cont => "SIGCONT (18)",
            Signal::Hup  => "SIGHUP (1)",
        }
    }

    pub fn to_nix(self) -> nix::sys::signal::Signal {
        match self {
            Signal::Term => nix::sys::signal::Signal::SIGTERM,
            Signal::Kill => nix::sys::signal::Signal::SIGKILL,
            Signal::Stop => nix::sys::signal::Signal::SIGSTOP,
            Signal::Cont => nix::sys::signal::Signal::SIGCONT,
            Signal::Hup  => nix::sys::signal::Signal::SIGHUP,
        }
    }
}

pub struct App {
    pub collector: ProcCollector,
    pub processes: Vec<ProcessInfo>,
    pub gpu_processes: Vec<GpuProcessInfo>,
    pub npu_processes: Vec<NpuProcessInfo>,
    pub summary: SystemSummary,

    pub tab: Tab,
    pub sort_col: SortColumn,
    pub sort_dir: SortDir,
    pub selected: usize,
    pub scroll_offset: usize,
    pub tree_view: bool,
    pub show_graphs: bool,

    pub input_mode: InputMode,
    pub filter_text: String,
    pub renice_text: String,
    pub signal_menu_idx: usize,
    pub status_msg: Option<String>,

    pub cpu_history: VecDeque<f64>,
    pub ram_history: VecDeque<f64>,
    pub gpu_history: VecDeque<f64>,

    pub quit: bool,
    pub my_uid: u32,
}

impl App {
    pub fn new() -> Self {
        let uid = nix::unistd::getuid().as_raw();
        Self {
            collector: ProcCollector::new(),
            processes: Vec::new(),
            gpu_processes: Vec::new(),
            npu_processes: Vec::new(),
            summary: SystemSummary::default(),
            tab: Tab::Processes,
            sort_col: SortColumn::Cpu,
            sort_dir: SortDir::Desc,
            selected: 0,
            scroll_offset: 0,
            tree_view: false,
            show_graphs: false,
            input_mode: InputMode::Normal,
            filter_text: String::new(),
            renice_text: String::new(),
            signal_menu_idx: 0,
            status_msg: None,
            cpu_history: VecDeque::with_capacity(HISTORY_LEN),
            ram_history: VecDeque::with_capacity(HISTORY_LEN),
            gpu_history: VecDeque::with_capacity(HISTORY_LEN),
            quit: false,
            my_uid: uid,
        }
    }

    pub fn tick(&mut self) {
        self.processes = self.collector.collect_processes();
        self.summary = self.collector.collect_system_summary();
        self.gpu_processes = proc_info::collect_gpu_processes();
        self.npu_processes = proc_info::collect_npu_processes();

        // Update history
        push_history(&mut self.cpu_history, self.summary.cpu_percent);
        push_history(&mut self.ram_history, self.summary.mem_percent);
        push_history(&mut self.gpu_history, self.summary.gpu_busy_percent as f64);

        // Apply filter
        if !self.filter_text.is_empty() {
            let f = self.filter_text.to_lowercase();
            self.processes.retain(|p| p.name.to_lowercase().contains(&f));
        }

        // Sort
        self.sort_processes();

        // Build tree if needed
        if self.tree_view {
            proc_info::build_tree(&mut self.processes);
        }

        // Clamp selection
        let len = self.visible_len();
        if len == 0 {
            self.selected = 0;
        } else if self.selected >= len {
            self.selected = len - 1;
        }
    }

    fn sort_processes(&mut self) {
        let dir = self.sort_dir;
        match self.sort_col {
            SortColumn::Pid => self.processes.sort_by(|a, b| cmp_val(a.pid, b.pid, dir)),
            SortColumn::User => {
                self.processes.sort_by(|a, b| cmp_str(&a.user, &b.user, dir))
            }
            SortColumn::Name => {
                self.processes.sort_by(|a, b| cmp_str(&a.name, &b.name, dir))
            }
            SortColumn::Cpu => self.processes.sort_by(|a, b| {
                cmp_f64(a.cpu_percent, b.cpu_percent, dir)
            }),
            SortColumn::Mem => self.processes.sort_by(|a, b| {
                cmp_f64(a.mem_percent, b.mem_percent, dir)
            }),
            SortColumn::Rss => {
                self.processes.sort_by(|a, b| cmp_val(a.rss_kb, b.rss_kb, dir))
            }
            SortColumn::State => self.processes.sort_by(|a, b| cmp_val(a.state, b.state, dir)),
            SortColumn::Runtime => self
                .processes
                .sort_by(|a, b| cmp_val(a.runtime, b.runtime, dir)),
        }
    }

    pub fn visible_len(&self) -> usize {
        match self.tab {
            Tab::Processes => self.processes.len(),
            Tab::Gpu => self.gpu_processes.len(),
            Tab::Npu => self.npu_processes.len(),
        }
    }

    pub fn selected_pid(&self) -> Option<u32> {
        match self.tab {
            Tab::Processes => self.processes.get(self.selected).map(|p| p.pid),
            Tab::Gpu => self.gpu_processes.get(self.selected).map(|p| p.pid),
            Tab::Npu => self.npu_processes.get(self.selected).map(|p| p.pid),
        }
    }

    #[allow(dead_code)]
    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        if self.tab == Tab::Processes {
            self.processes.get(self.selected)
        } else {
            None
        }
    }

    pub fn can_signal(&self, pid: u32) -> bool {
        if self.my_uid == 0 {
            return true; // root
        }
        // Check if process belongs to current user
        match self.tab {
            Tab::Processes => self
                .processes
                .get(self.selected)
                .map(|p| p.uid == self.my_uid)
                .unwrap_or(false),
            _ => {
                // Look up the process
                self.processes.iter().any(|p| p.pid == pid && p.uid == self.my_uid)
            }
        }
    }

    /// Get the expected process name for the currently selected entry.
    fn selected_name(&self) -> Option<String> {
        match self.tab {
            Tab::Processes => self.processes.get(self.selected).map(|p| p.name.clone()),
            Tab::Gpu => self.gpu_processes.get(self.selected).map(|p| p.name.clone()),
            Tab::Npu => self.npu_processes.get(self.selected).map(|p| p.name.clone()),
        }
    }

    pub fn send_signal(&mut self, sig: Signal) {
        if let Some(pid) = self.selected_pid() {
            if !self.can_signal(pid) {
                self.status_msg = Some(format!(
                    "Cannot signal PID {} — not owned by you",
                    pid
                ));
                return;
            }

            // TOCTOU mitigation: re-read /proc/{pid}/comm and verify the
            // process name still matches what the UI showed the user.  If the
            // PID was recycled between the user pressing "kill" and now, the
            // comm will differ and we abort.
            if let Some(expected_name) = self.selected_name() {
                let comm_path = format!("/proc/{}/comm", pid);
                match std::fs::read_to_string(&comm_path) {
                    Ok(current_comm) => {
                        let current_comm = current_comm.trim();
                        if current_comm != expected_name {
                            self.status_msg = Some("Process changed, signal not sent.".to_string());
                            self.input_mode = InputMode::Normal;
                            return;
                        }
                    }
                    Err(_) => {
                        // Process vanished — nothing to signal.
                        self.status_msg =
                            Some(format!("PID {} no longer exists, signal not sent.", pid));
                        self.input_mode = InputMode::Normal;
                        return;
                    }
                }
            }

            let nix_pid = nix::unistd::Pid::from_raw(pid as i32);
            match nix::sys::signal::kill(nix_pid, sig.to_nix()) {
                Ok(()) => {
                    self.status_msg = Some(format!(
                        "Sent {} to PID {}",
                        sig.label(),
                        pid
                    ));
                }
                Err(e) => {
                    self.status_msg = Some(format!(
                        "Failed to signal PID {}: {}",
                        pid, e
                    ));
                }
            }
        }
        self.input_mode = InputMode::Normal;
    }

    pub fn do_renice(&mut self) {
        if let Some(pid) = self.selected_pid() {
            if !self.can_signal(pid) {
                self.status_msg = Some(format!(
                    "Cannot renice PID {} — not owned by you",
                    pid
                ));
                self.input_mode = InputMode::Normal;
                return;
            }
            match self.renice_text.trim().parse::<i32>() {
                Ok(nice) => {
                    // Use libc setpriority
                    let ret =
                        unsafe { libc::setpriority(libc::PRIO_PROCESS, pid as libc::id_t, nice) };
                    if ret == 0 {
                        self.status_msg =
                            Some(format!("Set nice {} for PID {}", nice, pid));
                    } else {
                        self.status_msg = Some(format!(
                            "Failed to renice PID {}: errno {}",
                            pid,
                            std::io::Error::last_os_error()
                        ));
                    }
                }
                Err(_) => {
                    self.status_msg = Some("Invalid nice value".to_string());
                }
            }
        }
        self.renice_text.clear();
        self.input_mode = InputMode::Normal;
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        let len = self.visible_len();
        if len > 0 && self.selected < len - 1 {
            self.selected += 1;
        }
    }

    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(20);
    }

    pub fn page_down(&mut self) {
        let len = self.visible_len();
        if len > 0 {
            self.selected = (self.selected + 20).min(len - 1);
        }
    }

    pub fn next_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Processes => Tab::Gpu,
            Tab::Gpu => Tab::Npu,
            Tab::Npu => Tab::Processes,
        };
        self.selected = 0;
        self.scroll_offset = 0;
    }

    pub fn prev_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Processes => Tab::Npu,
            Tab::Gpu => Tab::Processes,
            Tab::Npu => Tab::Gpu,
        };
        self.selected = 0;
        self.scroll_offset = 0;
    }
}

fn push_history(hist: &mut VecDeque<f64>, val: f64) {
    if hist.len() >= HISTORY_LEN {
        hist.pop_front();
    }
    hist.push_back(val);
}

fn cmp_val<T: Ord>(a: T, b: T, dir: SortDir) -> std::cmp::Ordering {
    match dir {
        SortDir::Asc => a.cmp(&b),
        SortDir::Desc => b.cmp(&a),
    }
}

fn cmp_f64(a: f64, b: f64, dir: SortDir) -> std::cmp::Ordering {
    match dir {
        SortDir::Asc => a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal),
        SortDir::Desc => b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal),
    }
}

fn cmp_str(a: &str, b: &str, dir: SortDir) -> std::cmp::Ordering {
    match dir {
        SortDir::Asc => a.cmp(b),
        SortDir::Desc => b.cmp(a),
    }
}

extern crate libc;
