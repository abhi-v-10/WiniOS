use std::net::SocketAddr;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Starting,
    Running,
    Connected,
    TailscaleDisconnected,
    WaitingForNetwork,
    NotConfigured,
    NetworkUnavailable,
    Error,
    Stopping,
    Stopped,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "Starting",
            Self::Running => "Running",
            Self::Connected => "Connected",
            Self::TailscaleDisconnected => "Tailscale disconnected",
            Self::WaitingForNetwork => "Waiting for network",
            Self::NotConfigured => "Not configured",
            Self::NetworkUnavailable => "Network unavailable",
            Self::Error => "Error",
            Self::Stopping => "Stopping",
            Self::Stopped => "Stopped",
        }
    }
}
pub enum Event {
    ImageOptions(crate::image::Options),
    Permissions(bool),
    ClipboardWriter(crate::writes::WriteClient),
    Running(SocketAddr),
    Connectivity(crate::tailnet::Report),
    Finished(Result<(), (Status, String)>),
}
pub struct TrayModel {
    pub sharing: crate::sharing::SharingControl,
    pub retry_pending: bool,
    pub status: Status,
    pub endpoint: Option<String>,
    pub detail: String,
    pub worker_active: bool,
    pub quitting: bool,
}
impl Default for TrayModel {
    fn default() -> Self {
        Self {
            sharing: crate::sharing::SharingControl::default(),
            retry_pending: false,
            status: Status::Stopped,
            endpoint: None,
            detail: String::new(),
            worker_active: false,
            quitting: false,
        }
    }
}
impl TrayModel {
    pub fn begin(&mut self) -> bool {
        if self.worker_active || self.quitting {
            return false;
        }
        self.worker_active = true;
        self.retry_pending = false;
        self.status = Status::Starting;
        self.endpoint = None;
        self.detail.clear();
        true
    }
    pub fn apply(&mut self, event: Event) {
        match event {
            Event::ImageOptions(_) => {}
            Event::Permissions(_) => {}
            Event::ClipboardWriter(_) => {}
            Event::Running(address) => {
                self.endpoint = Some(format!("https://{address}"));
                self.status = if self.quitting {
                    Status::Stopping
                } else {
                    Status::Running
                };
            }
            Event::Connectivity(report) => {
                self.endpoint = report.endpoint;
                if !self.quitting {
                    self.detail = match report.state {
                        crate::tailnet::State::Connected => "",
                        crate::tailnet::State::Disconnected => "Connect Tailscale on Windows and iPhone.",
                        crate::tailnet::State::WaitingForNetwork => "Waiting for Tailscale connectivity or matching tailnet DNS.",
                        crate::tailnet::State::NotConfigured => "Review Tailscale sign-in, HTTPS and Serve setup. WiniOS changed no settings.",
                        crate::tailnet::State::Error => "Could not verify local Tailscale status/configuration. Clipboard access is disabled.",
                    }.into();
                    self.status = match report.state {
                        crate::tailnet::State::Connected => Status::Connected,
                        crate::tailnet::State::Disconnected => Status::TailscaleDisconnected,
                        crate::tailnet::State::WaitingForNetwork => Status::WaitingForNetwork,
                        crate::tailnet::State::NotConfigured => Status::NotConfigured,
                        crate::tailnet::State::Error => Status::Error,
                    };
                }
            }
            Event::Finished(result) => {
                self.worker_active = false;
                match result {
                    Err((status, message)) if !self.quitting && !self.retry_pending => {
                        self.status = status;
                        self.detail = message;
                    }
                    _ => self.status = Status::Stopped,
                }
            }
        }
    }
    pub fn quit(&mut self) {
        self.retry_pending = false;
        self.quitting = true;
        self.status = if self.worker_active {
            Status::Stopping
        } else {
            Status::Stopped
        };
    }
    pub fn retry(&mut self) -> bool {
        if self.quitting || self.retry_pending {
            return false;
        }
        self.retry_pending = true;
        self.detail.clear();
        self.endpoint = None;
        if self.worker_active {
            self.status = Status::Stopping;
        }
        true
    }
    pub fn status_text(&self) -> String {
        if self.quitting {
            self.status.label().into()
        } else if self.retry_pending && self.sharing.is_paused() {
            "Paused — reconnecting".into()
        } else if self.retry_pending {
            "Retrying connection".into()
        } else if self.sharing.is_paused() {
            format!("Paused — {}", self.status.label())
        } else {
            self.status.label().into()
        }
    }
    pub fn guidance(&self) -> &'static str {
        if self.quitting {
            "Waiting for a graceful shutdown."
        } else if self.sharing.is_paused() {
            "Sharing is paused. Resume from the tray when connected; Retry does not resume sharing."
        } else if self.retry_pending {
            "Reconnecting. Wait for Connected or Running before retrying the Shortcut."
        } else {
            match self.status {
                Status::Connected | Status::Running => {
                    "Connection ready. An empty clipboard needs a new copy; HTTP 412 means rerun Smart Receive from the type check."
                }
                Status::Starting => {
                    "Starting. Wait for Connected or Running before retrying the Shortcut."
                }
                Status::TailscaleDisconnected => {
                    "Connect Tailscale on both devices, then wait for Connected. Retry does not change Tailscale settings."
                }
                Status::WaitingForNetwork => {
                    "Check connectivity and Tailscale on both devices, then wait for Connected."
                }
                Status::NotConfigured => {
                    "Review sign-in and the existing HTTPS Serve setup in iPhone Setup. No network settings were changed."
                }
                Status::Error | Status::NetworkUnavailable => {
                    "Review the connection details and test HTTPS. Retry after correcting the connection; do not bypass certificate errors."
                }
                Status::Stopped => "Choose Retry connection to start WiniOS.",
                Status::Stopping => "Waiting for a graceful shutdown.",
            }
        }
    }
}
pub async fn cancelled(receiver: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *receiver.borrow() {
            return;
        }
        if receiver.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_feedback_keeps_pause_visible_during_retry_and_prioritizes_quit() {
        let mut model = TrayModel::default();
        model.begin();
        model.sharing.set_paused(true);
        assert!(model.retry());
        assert_eq!(model.status_text(), "Paused — reconnecting");
        assert!(model.guidance().contains("Retry does not resume"));
        model.quit();
        assert_eq!(model.status_text(), "Stopping");
        assert!(model.guidance().contains("shutdown"));
        model.apply(Event::Finished(Ok(())));
        assert_eq!(model.status_text(), "Stopped");
        model = TrayModel::default();
        model.begin();
        model.apply(Event::Connectivity(crate::tailnet::Report {
            state: crate::tailnet::State::Disconnected,
            endpoint: None,
        }));
        assert!(model.guidance().contains("Connect Tailscale"));
        assert!(!model.guidance().contains("HTTP 412"));
        model.apply(Event::Connectivity(crate::tailnet::Report {
            state: crate::tailnet::State::Connected,
            endpoint: Some("https://example.example.ts.net".into()),
        }));
        assert!(model.guidance().contains("empty clipboard"));
        assert!(model.guidance().contains("HTTP 412"));
    }
    #[test]
    fn retry_waits_for_worker_exit_preserves_pause_and_cannot_override_quit() {
        let mut model = TrayModel::default();
        assert!(model.begin());
        model.sharing.set_paused(true);
        assert!(model.retry());
        assert!(!model.retry());
        assert!(!model.begin());
        model.apply(Event::Finished(Ok(())));
        assert!(model.retry_pending && !model.worker_active);
        assert!(model.begin());
        assert!(!model.retry_pending && model.worker_active && model.sharing.is_paused());
        model.apply(Event::Running("192.168.1.7:8765".parse().unwrap()));
        assert!(model.status_text().starts_with("Paused"));
        assert!(model.retry());
        model.quit();
        model.apply(Event::Finished(Ok(())));
        assert!(!model.retry_pending && !model.retry() && !model.begin());
    }
    #[test]
    fn tailnet_state_can_recover_and_quit_cannot_become_connected() {
        let mut model = TrayModel::default();
        model.begin();
        for (state, expected) in [
            (crate::tailnet::State::NotConfigured, Status::NotConfigured),
            (
                crate::tailnet::State::Disconnected,
                Status::TailscaleDisconnected,
            ),
            (
                crate::tailnet::State::WaitingForNetwork,
                Status::WaitingForNetwork,
            ),
            (crate::tailnet::State::Connected, Status::Connected),
        ] {
            model.apply(Event::Connectivity(crate::tailnet::Report {
                state,
                endpoint: None,
            }));
            assert_eq!(model.status, expected);
            assert!(model.worker_active);
        }
        model.quit();
        model.apply(Event::Connectivity(crate::tailnet::Report {
            state: crate::tailnet::State::Connected,
            endpoint: None,
        }));
        assert_eq!(model.status, Status::Stopping);
    }
    #[test]
    fn lifecycle_quit_during_startup_cannot_become_running_or_restart() {
        let mut model = TrayModel::default();
        assert!(model.begin());
        assert!(!model.begin());
        model.quit();
        model.apply(Event::Running("192.168.1.7:8765".parse().unwrap()));
        assert_eq!(model.status, Status::Stopping);
        model.apply(Event::Finished(Ok(())));
        assert_eq!(model.status, Status::Stopped);
        assert!(!model.begin());
    }
    #[test]
    fn failed_service_can_retry_without_changing_endpoint_automatically() {
        let mut model = TrayModel::default();
        model.begin();
        model.apply(Event::Finished(Err((
            Status::NetworkUnavailable,
            "Configured IP is not assigned.".into(),
        ))));
        assert_eq!(model.status, Status::NetworkUnavailable);
        assert!(model.endpoint.is_none());
        assert!(model.begin());
        model.apply(Event::Running("192.168.1.7:8765".parse().unwrap()));
        assert_eq!(model.status, Status::Running);
    }
    #[tokio::test]
    async fn shutdown_signal_is_sticky_even_if_requested_before_subscription_wait() {
        let (sender, mut receiver) = tokio::sync::watch::channel(false);
        sender.send(true).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            cancelled(&mut receiver),
        )
        .await
        .unwrap();
    }
}
