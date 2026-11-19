//! Fixed health probes and read-only Raspberry Pi measurements.
use super::*;
use ring::rand::{SecureRandom, SystemRandom};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const RETEST_DELAY: Duration = Duration::from_secs(5);
// One hour of checks, so a fresh tab opens on a full graph.
const HISTORY_LEN: usize = 120;

/// One probe per device: the website over HTTPS, JetKVM over TCP to its one
/// port and by IPv4 echo, and Muse by IPv4 echo. No device needs two ping
/// methods for the same reading.
#[derive(Clone, Copy)]
pub(super) enum Target {
    Website,
    Jetkvm,
    Ping,
    Muse,
}

impl Target {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "website" => Some(Self::Website),
            "jetkvm" => Some(Self::Jetkvm),
            "ping" => Some(Self::Ping),
            "muse" => Some(Self::Muse),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, serde::Deserialize, Serialize)]
pub(super) struct Muse {
    pub(super) online: Option<bool>,
    pub(super) checked_at: u64,
}

impl Muse {
    pub(super) fn current(self, now: u64) -> Option<bool> {
        now.checked_sub(self.checked_at)
            .filter(|age| *age <= 90)
            .and(self.online)
    }
}

pub(super) fn epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_muse() -> Option<Muse> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open("/run/portfolio-muse/status.json")
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(1025).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 1024 {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Reading {
    pub(super) latency_ms: Option<u64>,
    // Only the website check records the HTTP status it saw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) code: Option<u16>,
    pub(super) checked_at: u64,
}

impl Reading {
    fn measured(start: Instant, success: bool) -> Self {
        Self {
            latency_ms: success.then(|| start.elapsed().as_millis() as u64),
            code: None,
            checked_at: epoch(),
        }
    }
}

/// One reading of every plotted series. The Pi remembers them so a fresh tab
/// has a graph without waiting for the next poll to arrive.
#[derive(Clone, Serialize)]
pub(super) struct Sample {
    pub(super) at: u64,
    pub(super) jetkvm: Option<u64>,
    pub(super) muse: Option<u64>,
    pub(super) portfolio: Option<u64>,
    pub(super) cpu: Option<f32>,
    pub(super) ram: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cores: Option<Vec<f32>>,
}

impl Sample {
    fn of(status: &Status) -> Self {
        let host = status.host.as_ref();
        Self {
            at: epoch(),
            jetkvm: status.ping_probe.and_then(|probe| probe.latency_ms),
            muse: status.muse_ping_probe.and_then(|probe| probe.latency_ms),
            portfolio: status.website_probe.and_then(|probe| probe.latency_ms),
            cpu: host.and_then(|host| host.cpu_percent),
            ram: host.and_then(|host| host.memory_used_mib),
            cores: host.and_then(|host| host.cores.clone()),
        }
    }
}

#[derive(Serialize)]
pub(super) struct History {
    pub(super) samples: Vec<Sample>,
}

#[derive(Clone, Default, Serialize)]
pub(super) struct Host {
    pub(super) cpu_percent: Option<f32>,
    // One reading per core, in /proc/stat order, once two samples exist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cores: Option<Vec<f32>>,
    pub(super) memory_used_mib: Option<u64>,
    pub(super) memory_total_mib: Option<u64>,
    pub(super) uptime_seconds: Option<u64>,
}

#[derive(Clone, Copy)]
struct Cpu {
    total: u64,
    idle: u64,
}

/// The most cores the dashboard will plot. The Pi has four.
const MAX_CORES: usize = 8;

impl Cpu {
    fn parse_line(line: &str) -> Option<Self> {
        let mut fields = line.split_whitespace();
        if !fields.next()?.starts_with("cpu") {
            return None;
        }
        // Exclude guest time, which is already included in user/nice.
        let values: Vec<u64> = fields
            .take(8)
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        let idle = values.get(3)?.checked_add(*values.get(4)?)?;
        Some(Self {
            total: values.into_iter().try_fold(0u64, u64::checked_add)?,
            idle,
        })
    }

    /// The aggregate first line and one entry per core line that follows it.
    fn parse(stat: &str) -> Option<(Self, Vec<Self>)> {
        let mut lines = stat.lines();
        let total = Self::parse_line(lines.next()?)?;
        let cores = lines.take(MAX_CORES).map_while(Self::parse_line).collect();
        Some((total, cores))
    }

    fn percent(self, old: Self) -> Option<f32> {
        let total = self.total.checked_sub(old.total)?;
        let idle = self.idle.checked_sub(old.idle)?;
        let busy = total.checked_sub(idle)?;
        (total > 0).then(|| (1000.0 * busy as f64 / total as f64).round() as f32 / 10.0)
    }
}

fn memory(info: &str) -> Option<(u64, u64)> {
    let kib = |field: &str| {
        let mut parts = info
            .lines()
            .find_map(|line| line.strip_prefix(field))?
            .split_whitespace();
        let value = parts.next()?.parse::<u64>().ok()?;
        (parts.next()? == "kB").then_some(value)
    };
    let total = kib("MemTotal:")?;
    Some((
        total.checked_sub(kib("MemAvailable:")?)? / 1024,
        total / 1024,
    ))
}

/// One tick's CPU counters, kept so the next tick can difference them.
struct Counters {
    total: Cpu,
    cores: Vec<Cpu>,
}

fn host_sample(previous: Option<Counters>) -> (Option<Counters>, Host) {
    let proc = |path| std::fs::read_to_string(path).ok();
    let cpu = proc("/proc/stat").as_deref().and_then(Cpu::parse);
    let mem = proc("/proc/meminfo").as_deref().and_then(memory);
    let uptime = proc("/proc/uptime").and_then(|text| {
        text.split_whitespace()
            .next()?
            .split('.')
            .next()?
            .parse()
            .ok()
    });
    let (total, cores) = match (&cpu, &previous) {
        (Some((new, new_cores)), Some(old)) if new_cores.len() == old.cores.len() => {
            let per_core = new_cores
                .iter()
                .zip(&old.cores)
                .map(|(new, old)| new.percent(*old))
                .collect::<Option<Vec<f32>>>();
            (new.percent(old.total), per_core)
        }
        (Some((_, _)), _) => (None, None),
        (None, _) => (None, None),
    };
    (
        cpu.map(|(total, cores)| Counters { total, cores }),
        Host {
            cpu_percent: total,
            cores: cores.filter(|cores| !cores.is_empty()),
            memory_used_mib: mem.map(|v| v.0),
            memory_total_mib: mem.map(|v| v.1),
            uptime_seconds: uptime,
        },
    )
}

async fn echo(ip: std::net::Ipv4Addr) -> io::Result<()> {
    // Linux ping sockets are ICMP datagrams controlled by ping_group_range.
    // The kernel fills in the identifier/checksum. No raw socket or capability.
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::ICMPV4),
    )?;
    socket.set_nonblocking(true)?;
    let socket = tokio::net::UdpSocket::from_std(socket.into())?;
    socket.connect((ip, 0)).await?;
    let mut packet = [0u8; 24];
    packet[0] = 8; // ICMP echo request
    SystemRandom::new()
        .fill(&mut packet[6..])
        .map_err(|_| io::Error::other("nonce unavailable"))?;
    socket.send(&packet).await?;
    let mut reply = [0u8; 64];
    loop {
        let length = socket.recv(&mut reply).await?;
        if length == packet.len() && reply[..2] == [0, 0] && reply[6..length] == packet[6..] {
            return Ok(());
        }
    }
}

pub(super) struct Probes {
    client: reqwest::Client,
    jetkvm: SocketAddr,
    muse: Option<std::net::Ipv4Addr>,
    tx: watch::Sender<Status>,
    last: [tokio::sync::Mutex<Option<Instant>>; 4],
    history: Mutex<VecDeque<Sample>>,
}

impl Probes {
    pub(super) fn new(
        client: reqwest::Client,
        jetkvm: SocketAddr,
        muse: Option<std::net::Ipv4Addr>,
        tx: watch::Sender<Status>,
    ) -> Self {
        Self {
            client,
            jetkvm,
            muse,
            tx,
            last: Default::default(),
            history: Mutex::new(VecDeque::with_capacity(HISTORY_LEN)),
        }
    }

    /// Append the whole current picture, whether a timer or a person asked for it.
    pub(super) fn record(&self) {
        let sample = Sample::of(&self.tx.borrow());
        let mut history = self
            .history
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if history.len() == HISTORY_LEN {
            history.pop_front();
        }
        history.push_back(sample);
    }

    pub(super) fn history(&self) -> History {
        History {
            samples: self
                .history
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .cloned()
                .collect(),
        }
    }

    pub(super) fn jetkvm_ip(&self) -> String {
        self.jetkvm.ip().to_string()
    }

    pub(super) fn muse_ip(&self) -> Option<String> {
        self.muse.map(|ip| ip.to_string())
    }

    pub(super) async fn run(&self, target: Target) -> &'static str {
        if matches!(target, Target::Muse) && self.muse.is_none() {
            return "Muse ping is not configured";
        }
        let mut last = self.last[target as usize].lock().await;
        if last.is_some_and(|t| t.elapsed() < RETEST_DELAY) {
            return "Recent result reused";
        }
        let start = Instant::now();
        let tcp = |addr| async move {
            matches!(
                time::timeout(CHECK_TIMEOUT, TcpStream::connect(addr)).await,
                Ok(Ok(_))
            )
        };
        let (ok, code) = match target {
            Target::Website => website_check(&self.client).await,
            Target::Jetkvm => (tcp(self.jetkvm).await, None),
            Target::Ping => match self.jetkvm.ip() {
                IpAddr::V4(ip) => (
                    matches!(time::timeout(CHECK_TIMEOUT, echo(ip)).await, Ok(Ok(()))),
                    None,
                ),
                IpAddr::V6(_) => return "IPv4 ping is not configured",
            },
            Target::Muse => (
                match self.muse {
                    Some(ip) => {
                        matches!(time::timeout(CHECK_TIMEOUT, echo(ip)).await, Ok(Ok(())))
                    }
                    None => false,
                },
                None,
            ),
        };
        let mut r = Reading::measured(start, ok);
        r.code = code;
        self.tx.send_modify(|s| match target {
            Target::Website => {
                s.website = if ok {
                    Website::Healthy
                } else {
                    Website::Unavailable
                };
                s.website_probe = Some(r);
            }
            Target::Jetkvm => {
                s.jetkvm = if ok {
                    JetKvm::Reachable
                } else {
                    JetKvm::Unavailable
                };
                s.jetkvm_probe = Some(r);
            }
            Target::Ping => s.ping_probe = Some(r),
            Target::Muse => s.muse_ping_probe = Some(r),
        });
        *last = Some(Instant::now());
        "Check complete"
    }

    pub(super) async fn monitor(self: Arc<Self>) {
        let mut previous = None;
        let mut timer = time::interval(PROBE_INTERVAL);
        timer.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        loop {
            timer.tick().await;
            let prior = previous.take();
            let host = tokio::task::spawn_blocking(move || {
                let (counters, host) = host_sample(prior);
                (counters, host, read_muse())
            });
            let (_, _, _, _, sample) = tokio::join!(
                self.run(Target::Website),
                self.run(Target::Jetkvm),
                self.run(Target::Ping),
                self.run(Target::Muse),
                host
            );
            if let Ok((counters, host, muse)) = sample {
                previous = counters;
                self.tx.send_modify(|s| {
                    s.host = Some(host);
                    s.muse = muse;
                });
            }
            self.record();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn simultaneous_fixed_socket_checks_share_one_fresh_result() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (tx, rx) = watch::channel(Status::initial());
        let probes = Probes::new(
            reqwest::Client::new(),
            listener.local_addr().unwrap(),
            None,
            tx,
        );
        let (a, b) = tokio::join!(probes.run(Target::Jetkvm), probes.run(Target::Jetkvm));
        assert!(matches!(
            (a, b),
            ("Check complete", "Recent result reused") | ("Recent result reused", "Check complete")
        ));
        let _ = listener.accept().await.unwrap();
        assert!(
            time::timeout(Duration::from_millis(15), listener.accept())
                .await
                .is_err()
        );
        assert!(rx.borrow().jetkvm_probe.unwrap().latency_ms.is_some());
        assert!(Target::parse("100.64.0.9:9999").is_none());
    }

    #[tokio::test]
    async fn muse_ping_is_one_probe_that_needs_a_configured_address() {
        assert!(matches!(Target::parse("muse"), Some(Target::Muse)));
        for name in ["Muse", "muse ", "muse.ping", ""] {
            assert!(Target::parse(name).is_none(), "{name}");
        }
        let (tx, rx) = watch::channel(Status::initial());
        let probes = Probes::new(
            reqwest::Client::new(),
            "127.0.0.1:9".parse().unwrap(),
            None,
            tx,
        );
        assert_eq!(
            probes.run(Target::Muse).await,
            "Muse ping is not configured"
        );
        assert!(rx.borrow().muse_ping_probe.is_none());
    }
    #[test]
    fn history_keeps_one_row_per_check_up_to_its_cap() {
        let (tx, _rx) = watch::channel(Status::initial());
        let probes = Probes::new(
            reqwest::Client::new(),
            "127.0.0.1:9".parse().unwrap(),
            None,
            tx.clone(),
        );
        tx.send_modify(|s| {
            s.ping_probe = Some(Reading {
                latency_ms: Some(4),
                code: None,
                checked_at: 1,
            });
            s.host = Some(Host {
                cpu_percent: Some(7.5),
                cores: Some(vec![1.0, 2.0, 3.0, 4.0]),
                memory_used_mib: Some(250),
                memory_total_mib: Some(415),
                uptime_seconds: None,
            });
        });
        probes.record();
        let rows = probes.history().samples;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].jetkvm, Some(4));
        assert_eq!(rows[0].cpu, Some(7.5));
        assert_eq!(rows[0].ram, Some(250));
        assert_eq!(rows[0].cores.as_deref(), Some(&[1.0, 2.0, 3.0, 4.0][..]));
        assert_eq!(rows[0].muse, None);
        assert_eq!(rows[0].portfolio, None);
        for _ in 0..HISTORY_LEN + 5 {
            probes.record();
        }
        assert_eq!(probes.history().samples.len(), HISTORY_LEN);
        assert_eq!(probes.jetkvm_ip(), "127.0.0.1");
        assert_eq!(probes.muse_ip(), None);
    }

    #[test]
    fn linux_host_measurements() {
        let (a, a_cores) = Cpu::parse("cpu  20 0 10 30 40 0 0 0 987 0\ncpu0 0 0 0 0 0").unwrap();
        let (b, _) = Cpu::parse("cpu  30 0 20 80 70 0 0 0 999 0").unwrap();
        let (aggregate, cores) = Cpu::parse(
            "cpu  20 0 10 30 40 0 0 0 987 0\ncpu0 5 0 5 10 10 0 0 0 0 0\ncpu1 5 0 5 10 10 0 0 0 0 0\nintr 1",
        )
        .unwrap();
        assert_eq!(a_cores.len(), 1);
        let (next, next_cores) = Cpu::parse(
            "cpu  30 0 20 80 70 0 0 0 999 0\ncpu0 15 0 5 10 10 0 0 0 0 0\ncpu1 5 0 5 30 10 0 0 0 0 0\nintr 1",
        )
        .unwrap();
        let per_core: Vec<Option<f32>> = next_cores
            .iter()
            .zip(&cores)
            .map(|(new, old)| new.percent(*old))
            .collect();
        assert_eq!(per_core, [Some(100.0), Some(0.0)]);
        assert_eq!(next.percent(aggregate), Some(20.0));
        assert_eq!(b.percent(a), Some(20.0));
        assert_eq!(b.percent(b), None);
        assert_eq!(a.percent(b), None);
        assert_eq!(
            memory("MemTotal: 4096 kB\nMemFree: 10 kB\nMemAvailable: 1024 kB"),
            Some((3, 4))
        );
        assert_eq!(memory("MemTotal: 512 kB\nMemAvailable: 800 kB"), None);
    }
    #[test]
    fn muse_status_expires_and_rejects_future_timestamps() {
        let status = Muse {
            online: Some(false),
            checked_at: 120,
        };
        assert_eq!(status.current(120), Some(false));
        assert_eq!(status.current(210), Some(false));
        assert_eq!(status.current(211), None);
        assert_eq!(status.current(119), None);
    }
}
