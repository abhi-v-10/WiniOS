mod clipboard;
#[cfg(windows)]
mod watcher;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut debug = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--debug-clipboard" => debug = true,
            "--help" | "-h" => {
                println!(
                    "winios-agent [--debug-clipboard]\nWarning: --debug-clipboard logs sensitive clipboard text."
                );
                return Ok(());
            }
            _ => return Err(format!("Unknown argument: {arg}").into()),
        }
    }
    #[cfg(windows)]
    watcher::run(clipboard::ClipboardStore::default(), debug)?;
    #[cfg(not(windows))]
    {
        let _ = debug;
        return Err("The clipboard watcher requires Windows.".into());
    }
    #[cfg(windows)]
    Ok(())
}
