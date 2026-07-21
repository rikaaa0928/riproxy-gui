use proxy_observe::{
    ConnectionSnapshot, ObserveRegistry, OverviewSnapshot, RouteSnapshot, SiteSnapshot,
};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const MAX_HISTORY: usize = 120;

#[derive(Clone, Copy)]
pub struct BandwidthPoint {
    pub rx_bps: u64,
    pub tx_bps: u64,
}

pub struct ObserveState {
    pub overview: Option<OverviewSnapshot>,
    pub sites: Vec<SiteSnapshot>,
    pub routes: Vec<RouteSnapshot>,
    pub connections: Vec<ConnectionSnapshot>,
    pub history: VecDeque<BandwidthPoint>,
    last_poll: Instant,
}

impl ObserveState {
    pub fn new() -> Self {
        Self {
            overview: None,
            sites: Vec::new(),
            routes: Vec::new(),
            connections: Vec::new(),
            history: VecDeque::new(),
            last_poll: Instant::now() - Duration::from_secs(5),
        }
    }

    pub fn update(&mut self, registry: Option<ObserveRegistry>) {
        if self.last_poll.elapsed() < Duration::from_millis(900) {
            return;
        }
        self.last_poll = Instant::now();

        let Some(registry) = registry else {
            self.overview = None;
            self.sites.clear();
            self.routes.clear();
            self.connections.clear();
            self.history.clear();
            return;
        };

        let overview = registry.overview();
        self.history.push_back(BandwidthPoint {
            rx_bps: overview.rx_bps,
            tx_bps: overview.tx_bps,
        });
        while self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
        self.overview = Some(overview);
        self.sites = registry.sites();
        self.routes = registry.routes();
        self.connections = registry.connections();
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_bps(bytes_per_second: u64) -> String {
    format!("{}/s", format_bytes(bytes_per_second))
}

pub fn format_uptime(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}
