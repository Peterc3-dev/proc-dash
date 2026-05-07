use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// Information about a single process
#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub name: String,
    pub state: char,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub rss_kb: u64,
    #[allow(dead_code)]
    pub start_time: u64,
    #[allow(dead_code)]
    pub utime: u64,
    #[allow(dead_code)]
    pub stime: u64,
    pub runtime: Duration,
    pub uid: u32,
    /// Children PIDs (populated during tree building)
    pub children: Vec<u32>,
}

/// GPU process info from fdinfo
#[derive(Clone, Debug)]
pub struct GpuProcessInfo {
    pub pid: u32,
    pub name: String,
    pub vram_kb: u64,
    pub gtt_kb: u64,
}

/// NPU process info
#[derive(Clone, Debug)]
pub struct NpuProcessInfo {
    pub pid: u32,
    pub name: String,
    pub fd_path: String,
}

/// System-level summary
#[derive(Clone, Debug, Default)]
pub struct SystemSummary {
    pub cpu_percent: f64,
    pub mem_total_kb: u64,
    pub mem_used_kb: u64,
    pub mem_percent: f64,
    pub vram_total_kb: u64,
    pub vram_used_kb: u64,
    pub gtt_total_kb: u64,
    pub gtt_used_kb: u64,
    pub gpu_busy_percent: u8,
    pub load_avg_1: f64,
    pub load_avg_5: f64,
    pub load_avg_15: f64,
    pub uptime_secs: u64,
    pub npu_module_loaded: bool,
}

struct CpuTimes {
    total: u64,
    idle: u64,
}

pub struct ProcCollector {
    prev_cpu: Option<CpuTimes>,
    prev_proc_times: HashMap<u32, (u64, u64)>, // pid -> (utime+stime, cpu_total)
    hz: u64,
    boot_time: u64,
    uid_cache: HashMap<u32, String>,
}

impl ProcCollector {
    pub fn new() -> Self {
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u64;
        let boot_time = read_boot_time();
        Self {
            prev_cpu: None,
            prev_proc_times: HashMap::new(),
            hz,
            boot_time,
            uid_cache: HashMap::new(),
        }
    }

    pub fn collect_processes(&mut self) -> Vec<ProcessInfo> {
        let cur_cpu = read_cpu_times();
        let cpu_delta_total = if let Some(ref prev) = self.prev_cpu {
            let dt = cur_cpu.total.saturating_sub(prev.total);
            if dt == 0 { 1 } else { dt }
        } else {
            1
        };

        let now_ticks = {
            let uptime = read_uptime_secs();
            uptime * self.hz
        };

        let mut procs = Vec::new();
        let mem_total = read_mem_total_kb();

        if let Ok(entries) = fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let fname = entry.file_name();
                let name_str = fname.to_string_lossy();
                if let Ok(pid) = name_str.parse::<u32>() {
                    if let Some(info) = self.read_process(pid, cpu_delta_total, now_ticks, mem_total) {
                        procs.push(info);
                    }
                }
            }
        }

        self.prev_cpu = Some(cur_cpu);
        // Clean up stale entries
        let live_pids: std::collections::HashSet<u32> = procs.iter().map(|p| p.pid).collect();
        self.prev_proc_times.retain(|pid, _| live_pids.contains(pid));

        procs
    }

    fn read_process(
        &mut self,
        pid: u32,
        cpu_delta_total: u64,
        now_ticks: u64,
        mem_total: u64,
    ) -> Option<ProcessInfo> {
        let stat_path = format!("/proc/{}/stat", pid);
        let stat_content = fs::read_to_string(&stat_path).ok()?;

        // Parse stat - name is in parens, may contain spaces
        let open = stat_content.find('(')?;
        let close = stat_content.rfind(')')?;
        let name = stat_content[open + 1..close].to_string();
        let rest = &stat_content[close + 2..];
        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() < 20 {
            return None;
        }

        let state = fields[0].chars().next().unwrap_or('?');
        let ppid: u32 = fields[1].parse().unwrap_or(0);
        let utime: u64 = fields[11].parse().unwrap_or(0);
        let stime: u64 = fields[12].parse().unwrap_or(0);
        let starttime: u64 = fields[19].parse().unwrap_or(0);

        // RSS in pages
        let rss_pages: u64 = fields[21].parse().unwrap_or(0);
        let page_size = 4; // KB
        let rss_kb = rss_pages * page_size;

        // CPU%
        let total_time = utime + stime;
        let cpu_percent = if let Some(&(prev_total, _prev_cpu_total)) = self.prev_proc_times.get(&pid)
        {
            let proc_delta = total_time.saturating_sub(prev_total);
            (proc_delta as f64 / cpu_delta_total as f64) * 100.0
        } else {
            0.0
        };
        self.prev_proc_times.insert(pid, (total_time, 0));

        // MEM%
        let mem_percent = if mem_total > 0 {
            (rss_kb as f64 / mem_total as f64) * 100.0
        } else {
            0.0
        };

        // Runtime
        let start_secs = self.boot_time + starttime / self.hz;
        let now_secs = now_ticks / self.hz;
        let runtime_secs = now_secs.saturating_sub(start_secs - self.boot_time);
        let runtime = Duration::from_secs(runtime_secs);

        // UID
        let uid = read_proc_uid(pid);
        let user = self.resolve_uid(uid);

        Some(ProcessInfo {
            pid,
            ppid,
            user,
            name,
            state,
            cpu_percent,
            mem_percent,
            rss_kb,
            start_time: starttime,
            utime,
            stime,
            runtime,
            uid,
            children: Vec::new(),
        })
    }

    fn resolve_uid(&mut self, uid: u32) -> String {
        if let Some(name) = self.uid_cache.get(&uid) {
            return name.clone();
        }
        let name = resolve_uid_to_name(uid);
        self.uid_cache.insert(uid, name.clone());
        name
    }

    pub fn collect_system_summary(&self) -> SystemSummary {
        let mut summary = SystemSummary::default();

        // CPU
        if let Some(ref prev) = self.prev_cpu {
            let cur = read_cpu_times();
            let total_delta = cur.total.saturating_sub(prev.total);
            let idle_delta = cur.idle.saturating_sub(prev.idle);
            if total_delta > 0 {
                summary.cpu_percent =
                    ((total_delta - idle_delta) as f64 / total_delta as f64) * 100.0;
            }
        }

        // Memory
        if let Ok(content) = fs::read_to_string("/proc/meminfo") {
            for line in content.lines() {
                if let Some(val) = line.strip_prefix("MemTotal:") {
                    summary.mem_total_kb = parse_meminfo_val(val);
                } else if let Some(val) = line.strip_prefix("MemAvailable:") {
                    let avail = parse_meminfo_val(val);
                    summary.mem_used_kb = summary.mem_total_kb.saturating_sub(avail);
                }
            }
            if summary.mem_total_kb > 0 {
                summary.mem_percent =
                    (summary.mem_used_kb as f64 / summary.mem_total_kb as f64) * 100.0;
            }
        }

        // GPU VRAM
        summary.vram_total_kb = read_sysfs_bytes("/sys/class/drm/card1/device/mem_info_vram_total")
            .unwrap_or(0)
            / 1024;
        summary.vram_used_kb = read_sysfs_bytes("/sys/class/drm/card1/device/mem_info_vram_used")
            .unwrap_or(0)
            / 1024;
        // Also try card0 if card1 doesn't exist
        if summary.vram_total_kb == 0 {
            summary.vram_total_kb =
                read_sysfs_bytes("/sys/class/drm/card0/device/mem_info_vram_total")
                    .unwrap_or(0)
                    / 1024;
            summary.vram_used_kb =
                read_sysfs_bytes("/sys/class/drm/card0/device/mem_info_vram_used")
                    .unwrap_or(0)
                    / 1024;
        }

        // GTT
        summary.gtt_total_kb = read_sysfs_bytes("/sys/class/drm/card1/device/mem_info_gtt_total")
            .unwrap_or(0)
            / 1024;
        summary.gtt_used_kb = read_sysfs_bytes("/sys/class/drm/card1/device/mem_info_gtt_used")
            .unwrap_or(0)
            / 1024;
        if summary.gtt_total_kb == 0 {
            summary.gtt_total_kb =
                read_sysfs_bytes("/sys/class/drm/card0/device/mem_info_gtt_total")
                    .unwrap_or(0)
                    / 1024;
            summary.gtt_used_kb =
                read_sysfs_bytes("/sys/class/drm/card0/device/mem_info_gtt_used")
                    .unwrap_or(0)
                    / 1024;
        }

        // GPU busy %
        summary.gpu_busy_percent = read_gpu_busy_percent();

        // Load avg
        if let Ok(content) = fs::read_to_string("/proc/loadavg") {
            let parts: Vec<&str> = content.split_whitespace().collect();
            if parts.len() >= 3 {
                summary.load_avg_1 = parts[0].parse().unwrap_or(0.0);
                summary.load_avg_5 = parts[1].parse().unwrap_or(0.0);
                summary.load_avg_15 = parts[2].parse().unwrap_or(0.0);
            }
        }

        // Uptime
        summary.uptime_secs = read_uptime_secs();

        // NPU module
        summary.npu_module_loaded = Path::new("/dev/accel/accel0").exists()
            || check_module_loaded("amdxdna");

        summary
    }
}

pub fn collect_gpu_processes() -> Vec<GpuProcessInfo> {
    let mut result = Vec::new();
    let entries = match fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return result,
    };

    for entry in entries.flatten() {
        let fname = entry.file_name();
        let name_str = fname.to_string_lossy();
        if let Ok(pid) = name_str.parse::<u32>() {
            let fdinfo_dir = format!("/proc/{}/fdinfo", pid);
            let mut vram_total: u64 = 0;
            let mut gtt_total: u64 = 0;

            if let Ok(fds) = fs::read_dir(&fdinfo_dir) {
                for fd_entry in fds.flatten() {
                    if let Ok(content) = fs::read_to_string(fd_entry.path()) {
                        // Look for drm-memory lines
                        let mut has_drm = false;
                        for line in content.lines() {
                            if let Some(rest) = line.strip_prefix("drm-memory-vram:") {
                                if let Some(kb) = parse_drm_memory_val(rest) {
                                    vram_total += kb;
                                    has_drm = true;
                                }
                            } else if let Some(rest) = line.strip_prefix("drm-memory-gtt:") {
                                if let Some(kb) = parse_drm_memory_val(rest) {
                                    gtt_total += kb;
                                    has_drm = true;
                                }
                            }
                            // Also handle older amdgpu format
                            if line.starts_with("amd-memory-visible-vram:") || line.starts_with("amd-memory-vram:") {
                                if let Some(rest) = line.split(':').nth(1) {
                                    if let Some(kb) = parse_drm_memory_val(rest) {
                                        vram_total += kb;
                                        has_drm = true;
                                    }
                                }
                            }
                            if line.starts_with("amd-memory-gtt:") {
                                if let Some(rest) = line.split(':').nth(1) {
                                    if let Some(kb) = parse_drm_memory_val(rest) {
                                        gtt_total += kb;
                                        has_drm = true;
                                    }
                                }
                            }
                            let _ = has_drm;
                        }
                    }
                }
            }

            if vram_total > 0 || gtt_total > 0 {
                let proc_name = read_proc_comm(pid).unwrap_or_else(|| format!("[{}]", pid));
                result.push(GpuProcessInfo {
                    pid,
                    name: proc_name,
                    vram_kb: vram_total,
                    gtt_kb: gtt_total,
                });
            }
        }
    }

    result.sort_by(|a, b| (b.vram_kb + b.gtt_kb).cmp(&(a.vram_kb + a.gtt_kb)));
    result
}

pub fn collect_npu_processes() -> Vec<NpuProcessInfo> {
    let mut result = Vec::new();
    let entries = match fs::read_dir("/proc") {
        Ok(e) => e,
        Err(_) => return result,
    };

    for entry in entries.flatten() {
        let fname = entry.file_name();
        let name_str = fname.to_string_lossy();
        if let Ok(pid) = name_str.parse::<u32>() {
            let fd_dir = format!("/proc/{}/fd", pid);
            if let Ok(fds) = fs::read_dir(&fd_dir) {
                for fd_entry in fds.flatten() {
                    if let Ok(link) = fs::read_link(fd_entry.path()) {
                        let link_str = link.to_string_lossy().to_string();
                        if link_str.contains("/dev/accel") {
                            let proc_name =
                                read_proc_comm(pid).unwrap_or_else(|| format!("[{}]", pid));
                            result.push(NpuProcessInfo {
                                pid,
                                name: proc_name,
                                fd_path: link_str,
                            });
                            break; // one entry per process
                        }
                    }
                }
            }
        }
    }

    result.sort_by_key(|p| p.pid);
    result
}

pub fn build_tree(procs: &mut [ProcessInfo]) {
    // Build parent->children map
    let pid_to_idx: HashMap<u32, usize> = procs.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();

    // Collect children for each process
    let children_map: HashMap<u32, Vec<u32>> = {
        let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
        for p in procs.iter() {
            map.entry(p.ppid).or_default().push(p.pid);
        }
        map
    };

    for p in procs.iter_mut() {
        if let Some(kids) = children_map.get(&p.pid) {
            p.children = kids.clone();
        }
    }
    let _ = pid_to_idx;
}

// === Helper functions ===

fn read_cpu_times() -> CpuTimes {
    if let Ok(content) = fs::read_to_string("/proc/stat") {
        if let Some(line) = content.lines().next() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 8 && parts[0] == "cpu" {
                let vals: Vec<u64> = parts[1..].iter().filter_map(|s| s.parse().ok()).collect();
                let total: u64 = vals.iter().sum();
                let idle = if vals.len() > 3 { vals[3] } else { 0 };
                return CpuTimes { total, idle };
            }
        }
    }
    CpuTimes { total: 0, idle: 0 }
}

fn read_boot_time() -> u64 {
    if let Ok(content) = fs::read_to_string("/proc/stat") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("btime ") {
                return rest.trim().parse().unwrap_or(0);
            }
        }
    }
    0
}

fn read_uptime_secs() -> u64 {
    if let Ok(content) = fs::read_to_string("/proc/uptime") {
        if let Some(first) = content.split_whitespace().next() {
            return first.parse::<f64>().unwrap_or(0.0) as u64;
        }
    }
    0
}

fn read_mem_total_kb() -> u64 {
    if let Ok(content) = fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if let Some(val) = line.strip_prefix("MemTotal:") {
                return parse_meminfo_val(val);
            }
        }
    }
    0
}

fn parse_meminfo_val(s: &str) -> u64 {
    let s = s.trim();
    // "12345 kB"
    s.split_whitespace()
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn read_sysfs_bytes(path: &str) -> Option<u64> {
    fs::read_to_string(path)
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn read_gpu_busy_percent() -> u8 {
    // Try card1 first, then card0
    for card in &["card1", "card0"] {
        let path = format!("/sys/class/drm/{}/device/gpu_busy_percent", card);
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(val) = content.trim().parse::<u8>() {
                return val;
            }
        }
    }
    0
}

fn read_proc_uid(pid: u32) -> u32 {
    let status_path = format!("/proc/{}/status", pid);
    if let Ok(content) = fs::read_to_string(status_path) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("Uid:") {
                // Real UID is the first field
                if let Some(val) = rest.split_whitespace().next() {
                    return val.parse().unwrap_or(0);
                }
            }
        }
    }
    0
}

fn resolve_uid_to_name(uid: u32) -> String {
    if let Ok(content) = fs::read_to_string("/etc/passwd") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() >= 3 {
                if let Ok(line_uid) = parts[2].parse::<u32>() {
                    if line_uid == uid {
                        return parts[0].to_string();
                    }
                }
            }
        }
    }
    uid.to_string()
}

fn read_proc_comm(pid: u32) -> Option<String> {
    let path = format!("/proc/{}/comm", pid);
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn parse_drm_memory_val(s: &str) -> Option<u64> {
    // Format: " 1234 KiB" or " 1234 kB"
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let val: u64 = parts[0].parse().ok()?;
    // Value is in KiB typically
    Some(val)
}

fn check_module_loaded(module: &str) -> bool {
    if let Ok(content) = fs::read_to_string("/proc/modules") {
        for line in content.lines() {
            if line.starts_with(module) {
                return true;
            }
        }
    }
    false
}

// Needed for sysconf
extern crate libc;
