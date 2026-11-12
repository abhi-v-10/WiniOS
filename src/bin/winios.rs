#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    if let Err(error) = winios_agent::integration::identify() {
        winios_agent::tray::show_error(&error.to_string());
        return std::process::ExitCode::FAILURE;
    }
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = if args.as_slice() == ["--quit"] {
        winios_agent::tray::request_quit()
    } else if args.as_slice() == ["--install-shortcut"] {
        winios_agent::integration::install_start_menu_shortcut()
    } else if args.first().is_some_and(|arg| arg == "--console") {
        args.remove(0);
        winios_agent::tray::run_console(args)
    } else {
        let native_menu = args.first().is_some_and(|arg| arg == "--native-menu");
        if native_menu {
            args.remove(0);
        }
        let startup = args.as_slice() == ["--startup"];
        if startup {
            args.clear();
        }
        let setup = args.first().is_some_and(|arg| arg == "--setup");
        if setup {
            args.remove(0);
        }
        let settings = args.first().is_some_and(|arg| arg == "--settings");
        if settings {
            args.remove(0);
        }
        let parsed = match args.as_slice() {
            [] => Ok(None),
            [tray] if tray == "--tray" => Ok(None),
            [flag, path] if flag == "--config" => Ok(Some(std::path::PathBuf::from(path))),
            [tray, flag, path] if tray == "--tray" && flag == "--config" => Ok(Some(std::path::PathBuf::from(path))),
            _ => Err("Use winios.exe [--native-menu] [--setup | --settings] [--tray] [--config <absolute-settings.json>], --quit, --install-shortcut, or --console <CLI options>.".into()),
        };
        parsed.and_then(|path| {
            winios_agent::tray::run_with_ui(path, native_menu, startup, setup, settings)
        })
    };
    if let Err(error) = result {
        winios_agent::tray::show_error(&error.to_string());
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("WiniOS desktop requires Windows.");
}
