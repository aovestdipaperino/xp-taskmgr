//! System sampling: CPU, memory, processes, network adapters and users.
//!
//! macOS does not have Windows concepts like a page file or a commit charge,
//! so this module maps the closest Darwin equivalents onto the XP vocabulary:
//!
//! * Commit charge is resident memory in use plus swap in use.
//! * The commit limit is physical memory plus the swap file size.
//! * System cache is the file-backed page count.
//! * Nonpaged kernel memory is wired memory, paged is the compressor pool.
//! * Handles are open file descriptors.

use std::collections::HashMap;
use std::ffi::CStr;
use std::time::Instant;

use sysinfo::{
    CpuRefreshKind, MemoryRefreshKind, Networks, ProcessRefreshKind, ProcessesToUpdate,
    RefreshKind, System, Users,
};

/// One row of the Processes tab.
#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent: Option<u32>,
    pub name: String,
    pub user: String,
    pub uid: u32,
    /// CPU usage normalized to the whole machine, 0..=100.
    pub cpu: f32,
    /// Resident memory in bytes.
    pub memory: u64,
}

/// One row of the Networking tab.
#[derive(Debug, Clone)]
pub struct AdapterInfo {
    pub device: String,
    pub name: String,
    /// Link speed in bits per second, 0 when unknown.
    pub link_speed: u64,
    pub connected: bool,
    /// Bits per second over the last sample interval.
    pub bits_per_sec: f64,
}

impl AdapterInfo {
    /// Utilization of the link in percent, 0..=100.
    pub fn utilization(&self) -> f64 {
        if self.link_speed == 0 {
            0.0
        } else {
            (self.bits_per_sec / self.link_speed as f64 * 100.0).min(100.0)
        }
    }
}

/// One row of the Users tab.
#[derive(Debug, Clone)]
pub struct UserSession {
    pub user: String,
    pub id: u32,
    pub client: String,
    pub session: String,
}

/// A full picture of the system at one instant, all memory figures in KiB.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub cpu_total: f32,
    pub cpu_kernel: f32,
    pub per_cpu: Vec<f32>,
    pub processes: Vec<ProcessInfo>,
    pub handles: u64,
    pub threads: u64,
    pub phys_total: u64,
    pub phys_available: u64,
    pub system_cache: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub commit_peak: u64,
    pub kernel_paged: u64,
    pub kernel_nonpaged: u64,
    pub adapters: Vec<AdapterInfo>,
    pub sessions: Vec<UserSession>,
}

/// Owns the sysinfo state and the deltas needed between samples.
pub struct Sampler {
    system: System,
    networks: Networks,
    users: Users,
    last_ticks: Option<[u32; 4]>,
    last_net: Instant,
    commit_peak: u64,
    port_names: HashMap<String, String>,
    page_size: u64,
}

impl std::fmt::Debug for Sampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sampler").finish_non_exhaustive()
    }
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new()
    }
}

impl Sampler {
    /// Creates a sampler and primes the CPU counters.
    pub fn new() -> Self {
        let system = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(MemoryRefreshKind::everything()),
        );
        // SAFETY: sysconf has no preconditions.
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(4096) as u64;
        Self {
            system,
            networks: Networks::new_with_refreshed_list(),
            users: Users::new_with_refreshed_list(),
            last_ticks: cpu_load_ticks(),
            last_net: Instant::now(),
            commit_peak: 0,
            port_names: hardware_port_names(),
            page_size,
        }
    }

    /// Takes a new sample. Intended to run on a background thread.
    pub fn sample(&mut self) -> Snapshot {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_user(sysinfo::UpdateKind::OnlyIfNotSet),
        );

        let (cpu_total, cpu_kernel) = self.cpu_load();
        let per_cpu = self
            .system
            .cpus()
            .iter()
            .map(sysinfo::Cpu::cpu_usage)
            .collect();
        let cpu_count = self.system.cpus().len().max(1) as f32;

        let mut processes = Vec::with_capacity(self.system.processes().len());
        let (mut handles, mut threads) = (0u64, 0u64);
        for (pid, process) in self.system.processes() {
            if process.thread_kind().is_some() {
                continue;
            }
            let pid = pid.as_u32();
            let (t, h) = task_counts(pid);
            threads += u64::from(t);
            handles += u64::from(h);
            let uid = process.user_id().map_or(u32::MAX, |u| **u);
            let user = process
                .user_id()
                .and_then(|u| self.users.get_user_by_id(u))
                .map_or_else(|| uid_name(uid), |u| u.name().to_owned());
            processes.push(ProcessInfo {
                pid,
                parent: process.parent().map(sysinfo::Pid::as_u32),
                name: process.name().to_string_lossy().into_owned(),
                user,
                uid,
                cpu: (process.cpu_usage() / cpu_count).clamp(0.0, 100.0),
                memory: process.memory(),
            });
        }

        let vm = vm_statistics(self.page_size);
        let kib = |b: u64| b / 1024;
        let phys_total = kib(self.system.total_memory());
        let used = kib(self.system.used_memory());
        let swap_total = kib(self.system.total_swap());
        let swap_used = kib(self.system.used_swap());
        let commit_total = used + swap_used;
        self.commit_peak = self.commit_peak.max(commit_total);

        Snapshot {
            cpu_total,
            cpu_kernel,
            per_cpu,
            processes,
            handles,
            threads,
            phys_total,
            phys_available: kib(self.system.available_memory()),
            system_cache: kib(vm.file_backed),
            commit_total,
            commit_limit: phys_total + swap_total,
            commit_peak: self.commit_peak,
            kernel_paged: kib(vm.compressed),
            kernel_nonpaged: kib(vm.wired),
            adapters: self.adapters(),
            sessions: user_sessions(),
        }
    }

    /// Returns total and kernel CPU load in percent since the previous call.
    fn cpu_load(&mut self) -> (f32, f32) {
        let Some(now) = cpu_load_ticks() else {
            return (self.system.global_cpu_usage(), 0.0);
        };
        let prev = self.last_ticks.replace(now).unwrap_or(now);
        let delta: [u32; 4] = std::array::from_fn(|i| now[i].wrapping_sub(prev[i]));
        let total: u64 = delta.iter().copied().map(u64::from).sum();
        if total == 0 {
            return (self.system.global_cpu_usage(), 0.0);
        }
        // Tick order is user, system, idle, nice.
        let busy = total - u64::from(delta[2]);
        (
            busy as f32 * 100.0 / total as f32,
            delta[1] as f32 * 100.0 / total as f32,
        )
    }

    fn adapters(&mut self) -> Vec<AdapterInfo> {
        let elapsed = self.last_net.elapsed().as_secs_f64().max(0.001);
        self.last_net = Instant::now();
        self.networks.refresh(true);
        let links = link_states();

        let mut adapters: Vec<AdapterInfo> = self
            .networks
            .iter()
            .filter_map(|(device, data)| {
                let (connected, link_speed) = links.get(device.as_str()).copied()?;
                let name = self.port_names.get(device)?.clone();
                let bytes = data.received() + data.transmitted();
                Some(AdapterInfo {
                    device: device.clone(),
                    name,
                    link_speed,
                    connected,
                    bits_per_sec: bytes as f64 * 8.0 / elapsed,
                })
            })
            .collect();
        adapters.sort_by(|a, b| a.device.cmp(&b.device));
        adapters
    }
}

/// Aggregated host CPU ticks in user, system, idle, nice order.
fn cpu_load_ticks() -> Option<[u32; 4]> {
    const HOST_CPU_LOAD_INFO: libc::c_int = 3;
    let mut ticks = [0u32; 4];
    let mut count: libc::mach_msg_type_number_t = 4;
    // SAFETY: the buffer holds `count` integers, which is what the
    // HOST_CPU_LOAD_INFO flavor writes.
    #[allow(deprecated)]
    let kr = unsafe {
        libc::host_statistics(
            libc::mach_host_self(),
            HOST_CPU_LOAD_INFO,
            ticks.as_mut_ptr().cast(),
            &raw mut count,
        )
    };
    (kr == libc::KERN_SUCCESS).then_some(ticks)
}

#[derive(Debug, Default)]
struct VmBytes {
    file_backed: u64,
    compressed: u64,
    wired: u64,
}

fn vm_statistics(page_size: u64) -> VmBytes {
    // SAFETY: vm_statistics64 is plain old data.
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;
    // SAFETY: the out-pointer and count describe a vm_statistics64.
    #[allow(deprecated)]
    let kr = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            (&raw mut stats).cast(),
            &raw mut count,
        )
    };
    if kr != libc::KERN_SUCCESS {
        return VmBytes::default();
    }
    VmBytes {
        file_backed: u64::from(stats.external_page_count) * page_size,
        compressed: u64::from(stats.compressor_page_count) * page_size,
        wired: u64::from(stats.wire_count) * page_size,
    }
}

/// Thread and file descriptor counts for a process, zero when not permitted.
fn task_counts(pid: u32) -> (u32, u32) {
    let pid = pid as libc::c_int;
    // SAFETY: proc_taskinfo is plain old data.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
    // SAFETY: the buffer is exactly `size` bytes.
    let written =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&raw mut info).cast(), size) };
    let threads = if written == size {
        info.pti_threadnum.max(0) as u32
    } else {
        0
    };
    // SAFETY: a null buffer asks for the size of the descriptor list.
    let bytes =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let handles = (bytes.max(0) as usize / std::mem::size_of::<libc::proc_fdinfo>()) as u32;
    (threads, handles)
}

fn uid_name(uid: u32) -> String {
    match uid {
        0 => "root".into(),
        u32::MAX => String::new(),
        _ => uid.to_string(),
    }
}

/// Link state and baud rate of every interface, keyed by BSD name.
fn link_states() -> HashMap<String, (bool, u64)> {
    let mut map = HashMap::new();
    let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills a linked list that freeifaddrs releases.
    if unsafe { libc::getifaddrs(&raw mut addrs) } != 0 {
        return map;
    }
    let mut cursor = addrs;
    while !cursor.is_null() {
        // SAFETY: cursor walks the list returned by getifaddrs.
        let ifa = unsafe { &*cursor };
        cursor = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_data.is_null() {
            continue;
        }
        // SAFETY: ifa_addr is non-null and points to a sockaddr.
        if i32::from(unsafe { (*ifa.ifa_addr).sa_family }) != libc::AF_LINK {
            continue;
        }
        let flags = ifa.ifa_flags;
        if flags & libc::IFF_LOOPBACK as u32 != 0 {
            continue;
        }
        // SAFETY: for AF_LINK entries ifa_data points to an if_data.
        let data = unsafe { &*ifa.ifa_data.cast::<libc::if_data>() };
        // SAFETY: ifa_name is a valid C string.
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let up = flags & libc::IFF_UP as u32 != 0 && flags & libc::IFF_RUNNING as u32 != 0;
        map.insert(name, (up, u64::from(data.ifi_baudrate)));
    }
    // SAFETY: addrs came from getifaddrs.
    unsafe { libc::freeifaddrs(addrs) };
    map
}

/// Maps BSD device names (en0) to hardware port names (Wi-Fi).
fn hardware_port_names() -> HashMap<String, String> {
    let Ok(out) = std::process::Command::new("/usr/sbin/networksetup")
        .arg("-listallhardwareports")
        .output()
    else {
        return HashMap::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut map = HashMap::new();
    let mut port = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("Hardware Port: ") {
            port = Some(p.to_owned());
        } else if let Some(d) = line.strip_prefix("Device: ")
            && let Some(p) = port.take()
        {
            map.insert(d.to_owned(), p);
        }
    }
    map
}

/// Logged in users, one row per user, from the utmpx database.
fn user_sessions() -> Vec<UserSession> {
    let mut sessions: Vec<UserSession> = Vec::new();
    // SAFETY: setutxent/getutxent/endutxent are used single-threaded here;
    // the sampler is only ever driven from one task at a time.
    unsafe {
        libc::setutxent();
        loop {
            let entry = libc::getutxent();
            if entry.is_null() {
                break;
            }
            let entry = &*entry;
            if entry.ut_type != libc::USER_PROCESS {
                continue;
            }
            let user = CStr::from_ptr(entry.ut_user.as_ptr())
                .to_string_lossy()
                .into_owned();
            let line = CStr::from_ptr(entry.ut_line.as_ptr())
                .to_string_lossy()
                .into_owned();
            let host = CStr::from_ptr(entry.ut_host.as_ptr())
                .to_string_lossy()
                .into_owned();
            if let Some(existing) = sessions.iter_mut().find(|s| s.user == user) {
                if line == "console" {
                    existing.session = "Console".into();
                }
                continue;
            }
            let id = sessions.len() as u32;
            sessions.push(UserSession {
                user,
                id,
                client: host,
                session: if line == "console" {
                    "Console".into()
                } else {
                    line
                },
            });
        }
        libc::endutxent();
    }
    sessions
}

#[cfg(test)]
mod tests {
    use super::Sampler;

    #[test]
    fn samples_the_system() {
        let t = std::time::Instant::now();
        let mut sampler = Sampler::new();
        eprintln!("new: {:?}", t.elapsed());
        for _ in 0..3 {
            let t = std::time::Instant::now();
            let snap = sampler.sample();
            eprintln!(
                "sample: {:?} ({} processes)",
                t.elapsed(),
                snap.processes.len()
            );
            assert!(!snap.processes.is_empty());
            assert!(snap.phys_total > 0);
            assert!(snap.commit_limit >= snap.phys_total);
        }
    }
}
