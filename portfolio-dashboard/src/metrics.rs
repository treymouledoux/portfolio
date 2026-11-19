//! Fixed health probes and read-only Raspberry Pi measurements.
use super::*;
use ring::rand::{SecureRandom, SystemRandom};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const RETEST_DELAY: Duration = Duration::from_secs(5);

#[derive(Clone, Copy)]
pub(super) enum Target {
    Website,
    Jetkvm,
    Ping,
}

impl Target {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "website" => Some(Self::Website),
            "jetkvm" => Some(Self::Jetkvm),
            "ping" => Some(Self::Ping),
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
    pub(super) checked_at: u64,
}

impl Reading {
    fn measured(start: Instant, success: bool) -> Self {
        Self {
            latency_ms: success.then(|| start.elapsed().as_millis() as u64),
            checked_at: epoch(),
        }
    }
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Host {
    pub(super) cpu_percent: Option<f32>,
    pub(super) memory_used_mib: Option<u64>,
    pub(super) memory_total_mib: Option<u64>,
    pub(super) uptime_seconds: Option<u64>,
}

#[derive(Clone, Copy)]
struct Cpu {
    total: u64,
    idle: u64,
}

impl Cpu {
    fn parse(stat: &str) -> Option<Self> {
        let mut fields = stat.lines().next()?.split_whitespace();
        if fields.next()? != "cpu" {
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

fn host_sample(previous: Option<Cpu>) -> (Option<Cpu>, Host) {
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
    (
        cpu,
        Host {
            cpu_percent: cpu.zip(previous).and_then(|(new, old)| new.percent(old)),
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
    tx: watch::Sender<Status>,
    last: [Mutex<Option<Instant>>; 3],
}

impl Probes {
    pub(super) fn new(
        client: reqwest::Client,
        jetkvm: SocketAddr,
        tx: watch::Sender<Status>,
    ) -> Self {
        Self {
            client,
            jetkvm,
            tx,
            last: Default::default(),
        }
    }

    pub(super) async fn run(&self, target: Target) -> &'static str {
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
        let ok = match target {
            Target::Website => website_healthy(&self.client).await,
            Target::Jetkvm => tcp(self.jetkvm).await,
            Target::Ping => match self.jetkvm.ip() {
                IpAddr::V4(ip) => {
                    matches!(time::timeout(CHECK_TIMEOUT, echo(ip)).await, Ok(Ok(())))
                }
                IpAddr::V6(_) => return "IPv4 ping is not configured",
            },
        };
        let r = Reading::measured(start, ok);
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
            let host = tokio::task::spawn_blocking(move || (host_sample(previous), read_muse()));
            let (_, _, _, sample) = tokio::join!(
                self.run(Target::Website),
                self.run(Target::Jetkvm),
                self.run(Target::Ping),
                host
            );
            if let Ok(((cpu, host), muse)) = sample {
                previous = cpu;
                self.tx.send_modify(|s| {
                    s.host = Some(host);
                    s.muse = muse;
                });
            }
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
        let probes = Probes::new(reqwest::Client::new(), listener.local_addr().unwrap(), tx);
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
    #[test]
    fn linux_host_measurements() {
        let a = Cpu::parse("cpu  20 0 10 30 40 0 0 0 987 0\ncpu0 0").unwrap();
        let b = Cpu::parse("cpu  30 0 20 80 70 0 0 0 999 0").unwrap();
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
