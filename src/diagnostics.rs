//! No disk log. GUI mode disables stdout entirely; console mode is explicit.
use std::sync::atomic::{AtomicBool, Ordering};
static CONSOLE: AtomicBool = AtomicBool::new(true);
pub fn set_console(enabled: bool) {
    CONSOLE.store(enabled, Ordering::Relaxed);
}
pub fn enabled() -> bool {
    CONSOLE.load(Ordering::Relaxed)
}
macro_rules! info {
    ($($arg:tt)*) => { if $crate::diagnostics::enabled() {
        use std::io::Write;
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    } };
}
macro_rules! error {
    ($($arg:tt)*) => { if $crate::diagnostics::enabled() {
        use std::io::Write;
        let _ = writeln!(std::io::stderr().lock(), $($arg)*);
    } };
}
pub(crate) use {error, info};
