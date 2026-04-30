// mDNS / Bonjour service publishing for daemon discovery.
// Publishes `_xbark._tcp.local.` so clients can find the running daemon
// without hardcoding a port.
//
// IMPORTANT: we intentionally use a *private* hostname (`xbark-<pid>.local.`)
// instead of the system hostname. Using the real hostname causes macOS to
// think there's a name conflict and append a "-2" suffix. Since this service
// only listens on 127.0.0.1, the host record doesn't matter for connectivity
// — clients read the port file directly, mDNS is just a "daemon is alive"
// beacon.

use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};

pub struct Discovery {
    daemon: ServiceDaemon,
    fullname: String,
}

impl Discovery {
    pub fn publish(port: u16) -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let service_type = "_xbark._tcp.local.";
        let pid = std::process::id();
        let instance = format!("xbark-{}", pid);
        // Private hostname — won't conflict with the OS hostname
        let private_host = format!("xbark-{}.local.", pid);
        let info = ServiceInfo::new(
            service_type,
            &instance,
            &private_host,
            "127.0.0.1",
            port,
            None,
        )?;
        let fullname = info.get_fullname().to_string();
        daemon.register(info)?;
        tracing::info!("mDNS registered: {} on :{}", fullname, port);
        Ok(Self { daemon, fullname })
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        if let Err(e) = self.daemon.unregister(&self.fullname) {
            tracing::warn!("mDNS unregister failed: {}", e);
        }
    }
}
