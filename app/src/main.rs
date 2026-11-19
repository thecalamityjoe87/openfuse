use openfuse::autostart::Settings;
use openfuse::backend::detect::create_backend;
use openfuse::single_instance::{self, Acquire};
use openfuse::tray::TrayLink;
use openfuse::ui::{self, App, External, Message, ThemeMode};
use openfuse::cli;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Mutex;

fn main() -> ExitCode {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("openfuse: {e}\n\n{}", cli::usage());
            return ExitCode::from(2);
        }
    };
    if args.help {
        print!("{}", cli::usage());
        return ExitCode::SUCCESS;
    }
    if args.version {
        println!("openfuse {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.verbose {
        // SAFETY: single-threaded at this point.
        unsafe { std::env::set_var("OPENFUSE_DEBUG", "1") };
    }
    if args.list_controls {
        return ExitCode::from(cli::list_controls(args.card) as u8);
    }

    let tx = ui::external_channel();
    let activate_tx = tx.clone();
    let _guard = match single_instance::acquire(&single_instance::default_path(), move || {
        let _ = activate_tx.unbounded_send(External::Activate);
    }) {
        Ok(Acquire::Primary(g)) => Some(g),
        Ok(Acquire::Secondary) => {
            println!("OpenFuse is already running; raised its window.");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("openfuse: single-instance check failed ({e}); continuing");
            None
        }
    };

    let backend = create_backend(args.mock, args.model.as_deref(), args.card, Path::new("/proc/asound"), true);
    if args.verbose {
        eprintln!("openfuse: backend: {} ({})", backend.description(), backend.status().text());
    }

    let boot_data = Mutex::new(Some((backend, args.clone(), tx)));
    let boot = move || {
        let (backend, args, tx) = boot_data.lock().unwrap().take().expect("boot runs once");
        let mut app = App::new(backend, args.theme, Settings::open());
        if !args.mock {
            app.watch_for_device(args.card, Path::new("/proc/asound").to_path_buf());
        }
        app.system_dark = args.theme == ThemeMode::Auto && ui::prefers_dark();
        app.desktop_notifications = true;
        app.start();
        if !args.no_tray {
            match TrayLink::spawn(app.tray_snapshot(), tx) {
                Ok(t) => app.tray = Some(t),
                Err(e) => {
                    if args.verbose || args.tray {
                        eprintln!("openfuse: no system tray ({e})");
                    }
                }
            }
        }
        let mut tasks = vec![];
        if args.theme == ThemeMode::Auto {
            tasks.push(iced::system::theme().map(Message::SystemTheme));
        }
        if args.tray && app.tray.is_none() {
            eprintln!("openfuse: no system tray available; showing the window instead of starting hidden.");
        }
        if !(args.tray && app.tray.is_some()) {
            tasks.push(app.open_window());
        }
        (app, iced::Task::batch(tasks))
    };

    let result = iced::daemon(boot, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .style(App::style)
        .font(ui::theme::FONT_REGULAR)
        .font(ui::theme::FONT_MEDIUM)
        .font(ui::theme::FONT_SEMIBOLD)
        .font(ui::theme::FONT_BOLD)
        .default_font(ui::theme::INTER)
        .antialiasing(true)
        .run();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("openfuse: {e}");
            ExitCode::FAILURE
        }
    }
}
