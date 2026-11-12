//! Native address-removal notifications; no interface polling or automatic rebind.
use std::{
    ffi::c_void,
    net::Ipv4Addr,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::sync::watch;
use windows::Win32::{
    Foundation::HANDLE,
    NetworkManagement::IpHelper::{
        CancelMibChangeNotify2, MIB_NOTIFICATION_TYPE, MIB_UNICASTIPADDRESS_ROW, MibDeleteInstance,
        NotifyUnicastIpAddressChange,
    },
    Networking::WinSock::AF_INET,
};
struct Context {
    ip: Ipv4Addr,
    removed: AtomicBool,
    stop: watch::Sender<bool>,
}
impl Context {
    fn address_removed(&self, ip: Ipv4Addr) {
        if self.ip == ip {
            self.removed.store(true, Ordering::Release);
            let _ = self.stop.send(true);
        }
    }
}
unsafe extern "system" fn changed(
    context: *const c_void,
    row: *const MIB_UNICASTIPADDRESS_ROW,
    kind: MIB_NOTIFICATION_TYPE,
) {
    if context.is_null() || row.is_null() || kind != MibDeleteInstance {
        return;
    }
    // Native callback owns the row only for this call. The context remains
    // allocated until CancelMibChangeNotify2 completes outside the callback.
    unsafe {
        if (*row).Address.si_family != AF_INET {
            return;
        }
        let ip = Ipv4Addr::from((*row).Address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes());
        (&*context.cast::<Context>()).address_removed(ip);
    }
}
pub struct AddressMonitor {
    handle: HANDLE,
    context: Option<Box<Context>>,
}
impl AddressMonitor {
    pub fn new(ip: Ipv4Addr, stop: watch::Sender<bool>) -> windows::core::Result<Self> {
        let context = Box::new(Context {
            ip,
            stop,
            removed: AtomicBool::new(false),
        });
        let mut handle = HANDLE::default();
        unsafe {
            NotifyUnicastIpAddressChange(
                AF_INET,
                Some(changed),
                Some((&*context as *const Context).cast()),
                false,
                &mut handle,
            )
            .ok()?;
        }
        Ok(Self {
            handle,
            context: Some(context),
        })
    }
    pub fn removed(&self) -> bool {
        self.context
            .as_ref()
            .is_some_and(|context| context.removed.load(Ordering::Acquire))
    }
}
impl Drop for AddressMonitor {
    fn drop(&mut self) {
        if unsafe { CancelMibChangeNotify2(self.handle) }.ok().is_err() {
            // Exceptional cancellation failure: retain callback storage rather
            // than risk a use-after-free. No keys/text are held in this context.
            if let Some(context) = self.context.take() {
                let _ = Box::into_raw(context);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_configured_address_removal_requests_shutdown() {
        let (sender, receiver) = watch::channel(false);
        let context = Context {
            ip: "192.168.1.7".parse().unwrap(),
            removed: AtomicBool::new(false),
            stop: sender,
        };
        context.address_removed("192.168.1.8".parse().unwrap());
        assert!(!*receiver.borrow());
        context.address_removed("192.168.1.7".parse().unwrap());
        assert!(*receiver.borrow());
        assert!(context.removed.load(Ordering::Acquire));
    }
    #[test]
    fn notification_registration_and_cleanup_need_no_adapter_changes() {
        let (sender, receiver) = watch::channel(false);
        let monitor = AddressMonitor::new("127.0.0.1".parse().unwrap(), sender).unwrap();
        assert!(!monitor.removed());
        drop(monitor);
        assert!(!*receiver.borrow());
    }
}
