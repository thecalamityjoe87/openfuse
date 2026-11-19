//! The installer window.

use std::path::PathBuf;
use std::time::Duration;

use iced::futures::{SinkExt, Stream};
use iced::widget::{
    button, center, checkbox, column, container, horizontal_space, mouse_area, progress_bar, row, scrollable, svg, text,
    text_input, vertical_space, Column, Space,
};
use iced::{window, Alignment, Element, Length, Padding, Subscription, Task, Theme};

use crate::engine::{self, Diagnosis, Event, Failure, InstallOutcome, INSTALL_STEPS, UNINSTALL_STEPS};
use crate::secureboot;
use crate::sysinfo::{self, Status, SystemInfo};
use crate::theme::{self as th, Marker, BOLD, MEDIUM, SEMIBOLD};

const DEVICE_RENDER: &[u8] = include_bytes!("../../assets/render/minifuse2_front.svg");
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub simulate: bool,
    pub resume: bool,
    pub payload: Option<PathBuf>,
    /// Development: render a page, save a PNG and exit.
    pub screenshot: Option<(String, PathBuf)>,
    pub dark: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StepState {
    Pending,
    Running,
    Done,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    Install,
    Uninstall,
    Enroll,
    Restore,
}

#[derive(Debug, Clone)]
enum Page {
    Welcome,
    Working(Job),
    Key,
    Restart { key: bool },
    KeyLater,
    Uninstalled,
    Restored,
    Finish(Diagnosis),
    Failed { failure: Failure, retry: Option<Job> },
}

#[derive(Debug, Clone)]
pub enum Message {
    Install,
    Uninstall,
    Close,
    Worker(Worker),
    EnrollKey,
    KeyLater,
    RestartNow,
    RestartLater,
    CheckAgain,
    OpenApp,
    Autostart(bool),
    ToggleDetails,
    Retry,
    RestoreStock,
    Tick,
    Shot(window::Screenshot),
    TakeShot,
    Drag,
    /// Periodic check for the MiniFuse being plugged in or out.
    Rescan,
    /// "Use my own password" for the Secure Boot key
    OwnPassword(bool),
    Password(String),
    Minimize,
}

#[derive(Debug, Clone)]
pub enum Worker {
    Ev(Event),
    Installed(Result<InstallOutcome, Failure>),
    Enrolled(Result<(), Failure>),
    Removed(Result<(), Failure>),
    Restored(Result<(), Failure>),
}

pub struct App {
    opts: Options,
    info: SystemInfo,
    page: Page,
    steps: Vec<(&'static str, StepState)>,
    status: String,
    progress: Option<u8>,
    log: Vec<String>,
    show_details: bool,
    cert: Option<String>,
    autostart: bool,
    notice: Option<String>,
    /// Animation phase for indeterminate progress, 0.0..1.0.
    phase: f32,
    /// the app version this installer carries
    bundled: Option<String>,
    /// the user chose their own Secure Boot key password (else `secureboot::MOK_PASSWORD`)
    own_password: bool,
    password: String,
}

pub fn run(opts: Options) -> iced::Result {
    let dark = opts.dark.unwrap_or_else(prefers_dark);
    let theme = th::iced_theme(if dark { th::DARK } else { th::LIGHT });
    iced::application("OpenFuse Installer", App::update, App::view)
        .theme(move |_| theme.clone())
        .subscription(App::subscription)
        .font(th::FONT_REGULAR)
        .font(th::FONT_MEDIUM)
        .font(th::FONT_SEMIBOLD)
        .font(th::FONT_BOLD)
        .default_font(th::INTER)
        .antialiasing(true)
        // Our own title controls; transparent so the rounded corners show the desktop.
        .style(|_, theme| iced::application::Appearance {
            background_color: if rounded_window() { iced::Color::TRANSPARENT } else { th::tokens(theme).bg },
            text_color: th::tokens(theme).text,
        })
        .window(window::Settings {
            size: iced::Size::new(920.0, 680.0),
            resizable: false,
            decorations: false,
            transparent: rounded_window(),
            position: window::Position::Centered,
            ..Default::default()
        })
        .run_with(move || App::new(opts))
}

/// Rounded, transparent corners need a compositor; Wayland always has one.
/// On X11 the corners stay square rather than risk black corners.
fn rounded_window() -> bool {
    std::env::var("XDG_SESSION_TYPE").map(|s| s == "wayland").unwrap_or(false)
        || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

const CORNER: f32 = 14.0;

/// The desktop's light/dark preference (XDG desktop portal), light if unknown.
fn prefers_dark() -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else { return false };
    conn.call_method(
        Some("org.freedesktop.portal.Desktop"),
        "/org/freedesktop/portal/desktop",
        Some("org.freedesktop.portal.Settings"),
        "ReadOne",
        &("org.freedesktop.appearance", "color-scheme"),
    )
    .ok()
    .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
    .and_then(|v| u32::try_from(v).ok())
        == Some(1)
}

impl App {
    fn new(opts: Options) -> (Self, Task<Message>) {
        let info = if opts.simulate { simulated_info() } else { sysinfo::gather() };
        let mut app = App {
            info,
            page: Page::Welcome,
            steps: Vec::new(),
            status: String::new(),
            progress: None,
            log: Vec::new(),
            show_details: false,
            cert: None,
            autostart: true,
            notice: None,
            phase: 0.0,
            bundled: None,
            own_password: false,
            password: String::new(),
            opts,
        };
        app.bundled = if app.opts.simulate { Some("0.1.0".into()) } else { engine::bundled_version(app.opts.payload.as_deref()) };
        if app.opts.resume {
            app.show_finish(engine::diagnose(app.opts.simulate));
        }
        let mut task = Task::none();
        if let Some((page, _)) = app.opts.screenshot.clone() {
            app.stage_for_screenshot(&page);
            task = Task::perform(tokio::time::sleep(Duration::from_millis(900)), |_| Message::TakeShot);
        }
        (app, task)
    }

    fn stage_for_screenshot(&mut self, page: &str) {
        self.steps = INSTALL_STEPS.iter().map(|s| (*s, StepState::Pending)).collect();
        self.page = match page {
            "working" => {
                self.steps[0].1 = StepState::Skipped;
                self.steps[1].1 = StepState::Running;
                self.status = "Building the driver".into();
                self.progress = Some(70);
                self.log = vec!["dkms".into(), "openfuse-dkms".into()];
                Page::Working(Job::Install)
            }
            "key" => {
                Page::Key
            }
            "restart" => Page::Restart { key: true },
            "done" => Page::Finish(Diagnosis::Ready),
            "notconfirmed" => Page::Finish(Diagnosis::KeyNotConfirmed),
            "failed" => {
                self.show_details = true;
                Page::Failed {
                    failure: Failure {
                        message: "The driver could not be built for your kernel".into(),
                        details: "dkms status: openfuse/0.1.0: added\nThe build log is in /var/lib/dkms/openfuse/.".into(),
                    },
                    retry: Some(Job::Install),
                }
            }
            "installed" => {
                self.info.installed_app = Some("0.1.0".into());
                self.info.installed_dkms = Some("0.1.0".into());
                Page::Welcome
            }
            "update" => {
                self.info.installed_app = Some("0.1.0".into());
                self.info.installed_dkms = Some("0.1.0".into());
                self.bundled = Some("0.2.0".into());
                Page::Welcome
            }
            _ => Page::Welcome,
        };
    }

    fn subscription(&self) -> Subscription<Message> {
        let animate = if matches!(self.page, Page::Working(_)) && self.progress.is_none() {
            iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        // Hotplug: re-reading /sys/bus/usb/devices once a second is cheap and needs no udev.
        let rescan = if self.opts.screenshot.is_none() {
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Rescan)
        } else {
            Subscription::none()
        };
        Subscription::batch([animate, rescan])
    }

    /// The MiniFuse was plugged in or out: refresh everything that depends on it.
    fn on_rescan(&mut self) -> Task<Message> {
        let devices = sysinfo::usb_devices();
        if devices == self.info.devices {
            return Task::none();
        }
        let plugged_in = self.info.devices.is_empty() && !devices.is_empty();
        self.info.devices = devices;
        let Page::Finish(d) = &self.page else { return Task::none() };
        if self.info.devices.is_empty() {
            // Unplugged: say so straight away.
            self.show_finish(Diagnosis::NotPluggedIn);
            Task::none()
        } else if plugged_in && !matches!(d, Diagnosis::KeyNotConfirmed) {
            // Give the driver a moment to set up the device, then check it.
            Task::perform(tokio::time::sleep(Duration::from_millis(1200)), |_| Message::CheckAgain)
        } else {
            Task::none()
        }
    }

    fn start(&mut self, job: Job) -> Task<Message> {
        self.log.clear();
        self.status = "Starting".into();
        self.progress = None;
        self.show_details = false;
        self.steps = match job {
            Job::Install => INSTALL_STEPS,
            Job::Uninstall => UNINSTALL_STEPS,
            Job::Enroll => &["Queue the key and build the driver"][..],
            Job::Restore => &["Switch to the standard driver"][..],
        }
        .iter()
        .map(|s| (*s, StepState::Pending))
        .collect();
        self.page = Page::Working(job);
        let sim = self.opts.simulate;
        let payload = self.opts.payload.clone();
        match job {
            Job::Install => run_worker(move |emit| Worker::Installed(engine::install(payload, sim, emit))),
            Job::Uninstall => run_worker(move |emit| Worker::Removed(engine::uninstall(sim, emit))),
            Job::Enroll => {
                self.steps[0].1 = StepState::Running;
                self.status = "Waiting for permission".into();
                let password = self.key_password().to_string();
                run_worker(move |_| Worker::Enrolled(engine::enroll_key(&password, sim)))
            }
            Job::Restore => {
                self.steps[0].1 = StepState::Running;
                self.status = "Waiting for permission".into();
                run_worker(move |_| Worker::Restored(engine::restore_stock(sim)))
            }
        }
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Install => return self.start(Job::Install),
            Message::Uninstall => return self.start(Job::Uninstall),
            Message::RestoreStock => return self.start(Job::Restore),
            Message::Retry => match self.page.clone() {
                Page::Failed { retry: Some(Job::Enroll), .. } | Page::Finish(Diagnosis::KeyNotConfirmed) | Page::Restored => {
                    if self.cert.is_none() {
                        self.cert = sysinfo::mok_cert();
                    }
                    self.page = Page::Key;
                }
                Page::Failed { retry: Some(job), .. } => return self.start(job),
                _ => {}
            },
            Message::Close => {
                self.finish_setup();
                return iced::exit();
            }
            Message::Worker(w) => return self.on_worker(w),
            Message::EnrollKey => return self.start(Job::Enroll),
            Message::KeyLater => self.page = Page::KeyLater,
            Message::RestartNow | Message::RestartLater => {
                if matches!(self.page, Page::Restart { .. })
                    && let Err(f) = engine::schedule_resume(self.opts.simulate)
                {
                    self.page = Page::Failed { failure: f, retry: None };
                    return Task::none();
                }
                if matches!(msg, Message::RestartNow) {
                    if let Err(f) = engine::restart(self.opts.simulate) {
                        self.page = Page::Failed { failure: f, retry: None };
                        return Task::none();
                    }
                    if self.opts.simulate {
                        // Pretend the computer restarted and the installer reopened.
                        self.page = Page::Finish(Diagnosis::Ready);
                        return Task::none();
                    }
                }
                return iced::exit();
            }
            Message::CheckAgain => {
                self.info = if self.opts.simulate { simulated_info() } else { sysinfo::gather() };
                self.show_finish(engine::diagnose(self.opts.simulate));
            }
            Message::Autostart(on) => self.autostart = on,
            Message::OwnPassword(on) => self.own_password = on,
            Message::Password(p) => self.password = p,
            Message::OpenApp => {
                self.finish_setup();
                if let Err(f) = engine::launch_app(self.opts.simulate) {
                    self.page = Page::Failed { failure: f, retry: None };
                    return Task::none();
                }
                return iced::exit();
            }
            Message::ToggleDetails => self.show_details = !self.show_details,
            Message::Drag => return window::get_latest().and_then(window::drag),
            Message::Rescan => return self.on_rescan(),
            Message::Minimize => return window::get_latest().and_then(|id| window::minimize(id, true)),
            Message::Tick => self.phase = (self.phase + 0.012) % 1.0,
            Message::TakeShot => return window::get_latest().and_then(window::screenshot).map(Message::Shot),
            Message::Shot(shot) => {
                if let Some((_, path)) = &self.opts.screenshot
                    && let Err(e) = save_png(path, &shot)
                {
                    eprintln!("screenshot failed: {e}");
                }
                return iced::exit();
            }
        }
        Task::none()
    }

    /// Show the result after a restart. "Ready" and "Plug in your MiniFuse" both
    /// mean the install is complete (the driver binds whenever the device shows
    /// up), so the resume hook goes now: otherwise the installer reopens at every
    /// login until the MiniFuse happens to be plugged in.
    fn show_finish(&mut self, d: Diagnosis) {
        if matches!(d, Diagnosis::Ready | Diagnosis::NotPluggedIn) && !self.opts.simulate {
            engine::clear_resume();
        }
        self.page = Page::Finish(d);
    }

    /// Leaving the "ready" page: apply the autostart choice and remove the resume hook.
    fn finish_setup(&mut self) {
        if matches!(self.page, Page::Finish(Diagnosis::Ready)) && !self.opts.simulate {
            if let Err(e) = engine::set_app_autostart(self.autostart) {
                self.notice = Some(format!("Could not change the login setting: {e}"));
            }
            engine::clear_resume();
        }
    }

    fn on_worker(&mut self, w: Worker) -> Task<Message> {
        match w {
            Worker::Ev(ev) => match ev {
                Event::StepStarted(i) => self.set_step(i, StepState::Running),
                Event::StepDone(i) => self.set_step(i, StepState::Done),
                Event::StepSkipped(i) => self.set_step(i, StepState::Skipped),
                Event::Status(s) => self.status = s,
                Event::Progress(p) => self.progress = p,
                Event::Log(l) => {
                    if self.log.last() != Some(&l) {
                        self.log.push(l);
                    }
                }
            },
            Worker::Installed(Ok(out)) => {
                self.cert = out.cert;
                self.page = if out.needs_key && self.cert.is_some() { Page::Key } else { Page::Restart { key: false } };
            }
            Worker::Installed(Err(f)) => self.page = Page::Failed { failure: f, retry: Some(Job::Install) },
            Worker::Enrolled(Ok(())) => {
                self.page = Page::Restart { key: true };
            }
            Worker::Enrolled(Err(f)) => self.page = Page::Failed { failure: f, retry: Some(Job::Enroll) },
            Worker::Removed(Ok(())) => self.page = Page::Uninstalled,
            Worker::Removed(Err(f)) => self.page = Page::Failed { failure: f, retry: Some(Job::Uninstall) },
            Worker::Restored(Ok(())) => self.page = Page::Restored,
            Worker::Restored(Err(f)) => self.page = Page::Failed { failure: f, retry: Some(Job::Restore) },
        }
        Task::none()
    }

    fn set_step(&mut self, i: usize, state: StepState) {
        if let Some(s) = self.steps.get_mut(i) {
            s.1 = state;
        }
        if state == StepState::Running {
            self.progress = None;
        }
    }

    // ================================================================ view

    fn view(&self) -> Element<'_, Message> {
        let content = match &self.page {
            Page::Welcome if self.info.installed() => self.view_installed(),
            Page::Welcome => self.view_welcome(),
            Page::Working(job) => self.view_working(*job),
            Page::Key => self.view_key(),
            Page::KeyLater => Screen::new("Finish the setup later")
                .subtitle("With Secure Boot on, the OpenFuse driver can only start after its key is confirmed.")
                .body(
                    card(
                        column![
                            body_text("Run the installer again whenever you are ready. It picks up where you left off."),
                            body_text("Until then, your MiniFuse keeps working as a normal audio interface."),
                        ]
                        .spacing(10),
                    )
                    .padding(22),
                )
                .button(primary("Close", Message::Close)),
            Page::Restart { key } => self.view_restart(*key),
            Page::Restored => Screen::new("Standard driver restored")
                .hero(badge_icon(Icon::Check, th::LIGHT.good, 56.0))
                .subtitle("Your audio devices work with the standard Linux driver again. OpenFuse stays installed; set up the key whenever you're ready to use its controls.")
                .button(secondary("Close", Message::Close))
                .button(primary("Set up the key", Message::Retry)),
            Page::Uninstalled => Screen::new("OpenFuse was removed")
                .hero(badge_icon(Icon::Check, th::LIGHT.good, 56.0))
                .subtitle("Restart to switch back to the standard Linux driver. Your MiniFuse keeps working as a normal audio interface.")
                .body(muted_text("The Secure Boot signing key stays on this computer. It is harmless, and other drivers may use it."))
                .button(secondary("Restart later", Message::Close))
                .button(primary("Restart now", Message::RestartNow)),
            Page::Finish(d) => self.view_finish(d),
            Page::Failed { failure, retry } => self.view_failed(failure, *retry),
        };

        let r = if rounded_window() { CORNER } else { 0.0 };
        let window = container(row![self.sidebar(), self.main(content)].height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |t: &Theme| {
                let tk = th::tokens(t);
                container::Style {
                    border: iced::Border {
                        color: if tk.dark { th::alpha(iced::Color::WHITE, 0.08) } else { th::alpha(iced::Color::BLACK, 0.12) },
                        width: 1.0,
                        radius: r.into(),
                    },
                    ..Default::default()
                }
            });
        // Press on any empty area to move the window.
        mouse_area(window).on_press(Message::Drag).into()
    }

    fn stage(&self) -> usize {
        match &self.page {
            Page::Welcome => 0,
            Page::Working(Job::Install) | Page::Working(Job::Uninstall) => 1,
            Page::Failed { retry: Some(Job::Enroll), .. } => 2,
            Page::Failed { .. } => 1,
            Page::Key | Page::Working(Job::Enroll) | Page::KeyLater => 2,
            Page::Working(Job::Restore) => 3,
            Page::Restart { .. } | Page::Uninstalled | Page::Restored | Page::Finish(_) => 3,
        }
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let stage = self.stage();
        let finished = matches!(self.page, Page::Finish(Diagnosis::Ready) | Page::Uninstalled);
        let names = ["Welcome", "Install", "Secure Boot", "Finish"];
        let mut timeline = Column::new();
        for (i, name) in names.iter().enumerate() {
            let state = if i < stage || (finished && i == stage) {
                Marker::Done
            } else if i == stage {
                Marker::Current
            } else {
                Marker::Upcoming
            };
            let inner: Element<'_, Message> = match state {
                Marker::Done => icon(Icon::Check, th::DARK.on_accent, 13.0),
                Marker::Current => container(Space::new(8, 8)).style(th::dot(th::DARK.accent)).into(),
                Marker::Upcoming => Space::new(0, 0).into(),
            };
            let marker = container(inner).width(24).height(24).center_x(24).center_y(24).style(th::marker(state));
            let label = match state {
                Marker::Current => text(*name).size(15).font(SEMIBOLD),
                Marker::Done => text(*name).size(15).font(MEDIUM),
                Marker::Upcoming => text(*name).size(15).style(th::side_muted),
            };
            timeline = timeline.push(row![marker, label].spacing(14).align_y(Alignment::Center));
            if i + 1 < names.len() {
                timeline = timeline.push(
                    container(container(Space::new(2, 26)).style(th::line(i < stage)))
                        .padding(Padding { left: 11.0, ..Padding::ZERO }),
                );
            }
        }

        let (dot_color, device_label) = match self.info.devices.first() {
            Some((_, name)) => (th::DARK.good, format!("{name} connected")),
            None => (th::DARK.side_muted, "No MiniFuse connected".to_string()),
        };
        let device = row![container(Space::new(8, 8)).style(th::dot(dot_color)), text(device_label).size(13).style(th::side_muted)]
            .spacing(8)
            .align_y(Alignment::Center);

        let mut bottom = column![device].spacing(10);
        if self.opts.simulate {
            bottom = bottom.push(
                container(text("SIMULATION").size(11).font(BOLD))
                    .padding(Padding::from([4, 10]))
                    .style(th::pill(th::DARK.warn)),
            );
        }
        bottom = bottom.push(text(format!("Version {VERSION}")).size(12).style(th::side_muted));

        container(
            column![
                column![
                    text("OpenFuse").size(22).font(BOLD),
                    text("Setup for Arturia MiniFuse").size(13).style(th::side_muted)
                ]
                .spacing(4),
                vertical_space().height(44),
                timeline,
                vertical_space(),
                bottom,
            ]
            .height(Length::Fill),
        )
        .width(260)
        .height(Length::Fill)
        .padding(Padding { top: 36.0, right: 28.0, bottom: 28.0, left: 32.0 })
        .style(|t: &Theme| {
            let r = if rounded_window() { CORNER } else { 0.0 };
            container::Style {
                border: iced::Border {
                    radius: iced::border::Radius { top_left: r, bottom_left: r, top_right: 0.0, bottom_right: 0.0 },
                    ..Default::default()
                },
                ..th::sidebar(t)
            }
        })
        .into()
    }

    /// Minimize and close, top right. No maximize: the installer has a fixed size.
    fn window_controls(&self) -> Element<'_, Message> {
        let ctl = |kind: Icon, msg: Message, close: bool| {
            button(chrome_icon(kind))
                .width(30)
                .height(30)
                .padding(0)
                .on_press(msg)
                .style(move |t: &Theme, status| {
                    let tk = th::tokens(t);
                    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
                    button::Style {
                        background: hovered.then_some(iced::Background::Color(if close { tk.bad } else { tk.surface_hover })),
                        text_color: tk.text,
                        border: iced::border::rounded(999),
                        shadow: iced::Shadow::default(),
                    }
                })
        };
        row![ctl(Icon::Minimize, Message::Minimize, false), ctl(Icon::Close, Message::Close, true)]
            .spacing(6)
            .into()
    }

    fn main<'a>(&'a self, s: Screen<'a>) -> Element<'a, Message> {
        let mut head = Column::new().spacing(10);
        if let Some(h) = s.hero {
            head = head.push(h).push(vertical_space().height(8));
        }
        head = head.push(text(s.title).size(28).font(BOLD).line_height(1.15));
        if let Some(sub) = s.subtitle {
            head = head.push(text(sub).size(16).line_height(1.45).style(th::muted));
        }
        let mut body_col = column![head].spacing(22);
        if let Some(b) = s.body {
            body_col = body_col.push(b);
        }

        let mut footer = row![].spacing(12).align_y(Alignment::Center);
        if let Some(n) = &self.notice {
            footer = footer.push(text(n).size(13).style(th::muted));
        }
        footer = footer.push(horizontal_space());
        for b in s.buttons {
            footer = footer.push(b);
        }

        container(
            column![
                row![horizontal_space(), self.window_controls()],
                container(
                    column![
                        scrollable(container(body_col).padding(Padding { right: 14.0, ..Padding::ZERO }))
                            .direction(scrollable::Direction::Vertical(
                                scrollable::Scrollbar::new().width(4).scroller_width(4).margin(0),
                            ))
                            .height(Length::Fill)
                            .style(th::scroll),
                        footer,
                    ]
                    .spacing(20),
                )
                .padding(Padding { top: 0.0, right: 30.0, bottom: 0.0, left: 0.0 }),
            ]
            .spacing(4),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding { top: 12.0, right: 14.0, bottom: 32.0, left: 52.0 })
        .style(|t: &Theme| {
            let r = if rounded_window() { CORNER } else { 0.0 };
            container::Style {
                border: iced::Border {
                    radius: iced::border::Radius { top_left: 0.0, bottom_left: 0.0, top_right: r, bottom_right: r },
                    ..Default::default()
                },
                ..th::page(t)
            }
        })
        .into()
    }

    /// The password the key is queued with (and that the blue screen asks for).
    fn key_password(&self) -> &str {
        if self.own_password { &self.password } else { secureboot::MOK_PASSWORD }
    }

    /// What Install puts on the system, in plain words, before anything happens.
    fn what_gets_installed(&self) -> Element<'_, Message> {
        let v = self.bundled.clone().unwrap_or_default();
        let mut items = column![
            bullet(format!(
                "The OpenFuse driver (openfuse-dkms {v}). It replaces the standard USB audio driver with the same \
                 driver plus MiniFuse controls, for this kernel and future ones. Uninstall puts the standard one back."
            )),
            bullet(format!("The OpenFuse app (openfuse {v}).")),
        ]
        .spacing(8);
        if !self.info.headers {
            items = items.push(bullet("The build files for your kernel (from your system's software sources).".into()));
        }
        items = items.push(bullet(
            "To build the driver, the matching Linux sources may be downloaded from Launchpad or kernel.org.".into(),
        ));
        if self.info.secure_boot == Some(true) && self.info.mok_enrolled != Some(true) {
            items = items.push(bullet("A key to sign the driver, which you approve once at the next start (Secure Boot).".into()));
        }
        card(items).padding(Padding::from([14, 20])).into()
    }

    fn view_welcome(&self) -> Screen<'_> {
        let blocked = !self.info.blockers().is_empty();
        let mut checks = Column::new();
        let all = self.info.checks();
        let n = all.len();
        for (i, c) in all.into_iter().enumerate() {
            checks = checks.push(check_row(c));
            if i + 1 < n {
                checks = checks.push(hairline());
            }
        }
        let note: Element<'_, Message> = if blocked {
            callout(Icon::Cross, th::LIGHT.bad, "OpenFuse can't be installed on this system yet. See the item marked above.")
        } else {
            Space::new(0, 0).into()
        };
        let install = if blocked { primary_disabled("Install") } else { primary("Install", Message::Install) };
        Screen::new("Bring your MiniFuse settings to Linux")
            .hero(device_hero())
            .subtitle("Instrument inputs, phantom power, direct monitoring, loopback and meters. Audio already works — OpenFuse adds the controls.")
            .body(
                column![
                    section_label("What gets installed"),
                    self.what_gets_installed(),
                    section_label("Your system"),
                    card(checks).padding(Padding::from([6, 20])),
                    note
                ]
                .spacing(14),
            )
            .button(secondary("Cancel", Message::Close))
            .button(install)
    }

    fn view_installed(&self) -> Screen<'_> {
        let version = self.info.installed_app.clone().or(self.info.installed_dkms.clone()).unwrap_or_default();
        let bundled = self.bundled.clone().unwrap_or_default();
        let order = self.bundled.as_deref().map(|b| engine::compare_versions(b, &version));
        let (title, subtitle, main, main_desc, action) = match order {
            Some(std::cmp::Ordering::Greater) => (
                "An update is available",
                format!("Version {version} is installed. This installer brings version {bundled}."),
                "Update",
                format!("Installs OpenFuse {bundled}. Your settings are kept."),
                Some(primary("Update", Message::Install)),
            ),
            Some(std::cmp::Ordering::Less) => (
                "OpenFuse is installed",
                format!("Version {version} is installed. This installer has the older version {bundled}: download the current one to repair."),
                "Repair",
                "Not available with this installer: it carries an older version.".to_string(),
                Some(primary_disabled("Repair")),
            ),
            _ => (
                "OpenFuse is installed",
                format!("Version {version} is installed on this computer."),
                "Repair",
                "Installs OpenFuse again. Use this if the controls stopped working, for example after a system update.".to_string(),
                Some(primary("Repair", Message::Install)),
            ),
        };
        let options = row![
            option_card(Icon::Repair, main, main_desc),
            option_card(
                Icon::Remove,
                "Uninstall",
                "Removes OpenFuse and switches back to the standard Linux driver. Audio keeps working.".to_string()
            ),
        ]
        .spacing(14);
        let mut screen = Screen::new(title)
            .hero(device_hero())
            .subtitle_owned(subtitle)
            .body(options)
            // the way out first, then the actions in the order of the cards above
            .button(secondary("Close", Message::Close));
        if let Some(a) = action {
            screen = screen.button(a);
        }
        screen.button(danger("Uninstall", Message::Uninstall))
    }

    fn view_working(&self, job: Job) -> Screen<'_> {
        let (title, subtitle) = match job {
            Job::Install => ("Installing", "Enter your password when asked, and keep this window open until it's done."),
            Job::Uninstall => ("Removing OpenFuse", "Enter your password when asked."),
            Job::Enroll => ("Setting up the key", "Enter your password when asked. This queues the key for the next start."),
            Job::Restore => ("Switching to the standard driver", "Enter your password when asked."),
        };
        let mut steps = Column::new().spacing(16);
        for (name, state) in &self.steps {
            let mark: Element<'_, Message> = match state {
                StepState::Pending => container(Space::new(0, 0))
                    .width(22)
                    .height(22)
                    .style(|t: &Theme| {
                        let tk = th::tokens(t);
                        container::Style {
                            border: iced::Border { color: tk.border, width: 2.0, radius: 999.0.into() },
                            ..Default::default()
                        }
                    })
                    .into(),
                StepState::Running => container(container(Space::new(8, 8)).style(|t: &Theme| th::dot(th::tokens(t).accent)(t)))
                    .width(22)
                    .height(22)
                    .center_x(22)
                    .center_y(22)
                    .style(|t: &Theme| th::badge(th::tokens(t).accent, 0.16)(t))
                    .into(),
                StepState::Done | StepState::Skipped => badge_icon(Icon::Check, th::LIGHT.good, 22.0),
            };
            let label = match state {
                StepState::Pending => text(*name).size(15).style(th::muted),
                StepState::Running => text(*name).size(15).font(SEMIBOLD),
                _ => text(*name).size(15),
            };
            let mut r = row![mark, label].spacing(14).align_y(Alignment::Center);
            if *state == StepState::Skipped {
                r = r.push(text("already in place").size(13).style(th::muted));
            }
            steps = steps.push(r);
        }

        let bar: Element<'_, Message> = match self.progress {
            Some(p) => progress_bar(0.0..=100.0, p as f32).height(6).style(th::progress).into(),
            None => indeterminate(self.phase),
        };
        let pct = match self.progress {
            Some(p) => text(format!("{p}%")).size(13).style(th::muted),
            None => text(""),
        };
        let mut body = column![
            card(steps).padding(22),
            column![row![text(&self.status).size(14).font(MEDIUM), horizontal_space(), pct], bar].spacing(10),
        ]
        .spacing(22);
        if !self.log.is_empty() {
            body = body.push(link(if self.show_details { "Hide details" } else { "Show details" }, Message::ToggleDetails));
            if self.show_details {
                let lines = self.log.iter().rev().take(8).rev().cloned().collect::<Vec<_>>().join("\n");
                body = body.push(card(text(lines).size(12).font(iced::Font::MONOSPACE).style(th::muted)).padding(16));
            }
        }
        Screen::new(title).subtitle(subtitle).body(body)
    }

    fn view_key(&self) -> Screen<'_> {
        let steps = column![
            text("At the next start").size(15).font(SEMIBOLD),
            numbered(1, "A blue screen appears. Press any key within 10 seconds."),
            numbered(2, "Choose “Enroll MOK”, then “Continue”, then “Yes”."),
            numbered(3, "Type the password above and choose “Reboot”."),
        ]
        .spacing(12);
        let mut own = column![checkbox("Use my own password", self.own_password).on_toggle(Message::OwnPassword).size(18).text_size(14)]
            .spacing(8);
        if self.own_password {
            own = own.push(text_input("Your password", &self.password).on_input(Message::Password).padding(10).size(15));
            let hint = if secureboot::valid_password(&self.password) || self.password.is_empty() {
                "Up to 256 letters, digits or symbols. The blue screen may use a US keyboard: avoid a, q, w, y, z and m, \
                 which move on other layouts."
            } else {
                "Use 1 to 256 plain letters, digits or symbols (no accents)."
            };
            own = own.push(muted_text(hint));
        }
        let ok = !self.own_password || secureboot::valid_password(&self.password);
        Screen::new("Approve the driver once")
            .subtitle("Secure Boot only starts trusted drivers. The OpenFuse driver is signed with a key made on this computer, and you approve it once at the next start.")
            .body(column![password_card(self.key_password()), own, card(steps).padding(22)].spacing(16))
            .button(secondary("Set up later", Message::KeyLater))
            .button(if ok { primary("Continue", Message::EnrollKey) } else { primary_disabled("Continue") })
    }

    fn view_restart(&self, key: bool) -> Screen<'_> {
        let mut s = Screen::new("Restart to finish")
            .hero(badge_icon(Icon::Restart, th::LIGHT.accent, 56.0))
            .subtitle("OpenFuse is installed. Restart to start the new driver; this window comes back afterwards to finish up.");
        if key {
            s = s.body(
                card(
                    column![
                        text("Remember at the blue screen").size(15).font(SEMIBOLD),
                        numbered(1, "Press any key, then choose “Enroll MOK”."),
                        numbered(2, "Choose “Continue”, then “Yes”."),
                        numbered(3, "Type the password and choose “Reboot”."),
                    ]
                    .spacing(12),
                )
                .padding(22),
            );
            s = s.body_prefix(password_card(self.key_password()));
        } else {
            s = s.body(muted_text("Save your work in other apps before restarting."));
        }
        s.button(secondary("Restart later", Message::RestartLater)).button(primary("Restart now", Message::RestartNow))
    }

    fn view_finish(&self, d: &Diagnosis) -> Screen<'_> {
        match d {
            Diagnosis::Ready => {
                let device = self.info.devices.first().map(|(_, n)| n.clone()).unwrap_or_else(|| "MiniFuse".into());
                Screen::new("You're all set")
                    .hero(badge_icon(Icon::Check, th::LIGHT.good, 56.0))
                    .subtitle_owned(format!(
                        "Your {device} is ready. Find OpenFuse in your apps, or keep it in the tray for one-click access to phantom power, instrument inputs and direct monitoring."
                    ))
                    .body(
                        card(
                            checkbox("Start OpenFuse in the tray when I log in", self.autostart)
                                .on_toggle(Message::Autostart)
                                .size(18)
                                .spacing(12)
                                .text_size(15)
                                .style(th::check),
                        )
                        .padding(Padding::from([16, 20])),
                    )
                    .button(secondary("Close", Message::Close))
                    .button(primary("Open OpenFuse", Message::OpenApp))
            }
            Diagnosis::NotPluggedIn => Screen::new("Plug in your MiniFuse")
                .hero(badge_icon(Icon::Plug, th::LIGHT.accent, 56.0))
                .subtitle("OpenFuse is installed. Connect your MiniFuse with its USB cable, wait a moment, then check again.")
                .button(secondary("Close", Message::Close))
                .button(primary("Check again", Message::CheckAgain)),
            Diagnosis::KeyNotConfirmed => Screen::new("The driver wasn't approved")
                .hero(badge_icon(Icon::Info, th::LIGHT.warn, 56.0))
                .subtitle("The blue screen at startup was skipped or timed out, so the OpenFuse driver can't start. Set up the key again and restart, or switch back to the standard driver for now.")
                .button(secondary("Use the standard driver", Message::RestoreStock))
                .button(primary("Set up the key again", Message::Retry)),
            Diagnosis::DriverNotActive => Screen::new("Almost there")
                .hero(badge_icon(Icon::Plug, th::LIGHT.warn, 56.0))
                .subtitle("OpenFuse is installed and your MiniFuse is connected, but the new driver isn't running yet. Unplug the MiniFuse, plug it back in, and check again. If that doesn't help, restart.")
                .button(secondary("Close", Message::Close))
                .button(primary("Check again", Message::CheckAgain)),
        }
    }

    fn view_failed<'a>(&'a self, f: &'a Failure, retry: Option<Job>) -> Screen<'a> {
        let mut body = Column::new().spacing(14);
        if !f.details.is_empty() {
            body = body.push(link(if self.show_details { "Hide details" } else { "Show details" }, Message::ToggleDetails));
            if self.show_details {
                body = body.push(card(text(&f.details).size(12).font(iced::Font::MONOSPACE)).padding(16));
            }
        }
        let mut s = Screen::new_owned(f.message.clone())
            .hero(badge_icon(Icon::Cross, th::LIGHT.bad, 56.0))
            .subtitle("Nothing was left half-installed, and your MiniFuse keeps working as a normal audio interface.")
            .body(body)
            .button(secondary("Close", Message::Close));
        if retry.is_some() {
            s = s.button(primary("Try again", Message::Retry));
        }
        s
    }
}

// ================================================================ screen builder

struct Screen<'a> {
    title: String,
    subtitle: Option<String>,
    hero: Option<Element<'a, Message>>,
    body: Option<Element<'a, Message>>,
    buttons: Vec<Element<'a, Message>>,
}

impl<'a> Screen<'a> {
    fn new(title: &str) -> Self {
        Self::new_owned(title.to_string())
    }
    fn new_owned(title: String) -> Self {
        Screen { title, subtitle: None, hero: None, body: None, buttons: Vec::new() }
    }
    fn subtitle(mut self, s: &str) -> Self {
        self.subtitle = Some(s.to_string());
        self
    }
    fn subtitle_owned(mut self, s: String) -> Self {
        self.subtitle = Some(s);
        self
    }
    fn hero(mut self, e: impl Into<Element<'a, Message>>) -> Self {
        self.hero = Some(e.into());
        self
    }
    fn body(mut self, e: impl Into<Element<'a, Message>>) -> Self {
        self.body = Some(e.into());
        self
    }
    /// Put `e` above the current body.
    fn body_prefix(mut self, e: impl Into<Element<'a, Message>>) -> Self {
        let first: Element<'a, Message> = e.into();
        self.body = Some(match self.body.take() {
            Some(b) => column![first, b].spacing(16).into(),
            None => first,
        });
        self
    }
    fn button(mut self, e: Element<'a, Message>) -> Self {
        self.buttons.push(e);
        self
    }
}

// ================================================================ worker plumbing

/// Run blocking engine work on a thread, streaming its events into the UI.
fn run_worker<F>(work: F) -> Task<Message>
where
    F: FnOnce(&mut dyn FnMut(Event)) -> Worker + Send + 'static,
{
    Task::run(worker_stream(work), Message::Worker)
}

fn worker_stream<F>(work: F) -> impl Stream<Item = Worker>
where
    F: FnOnce(&mut dyn FnMut(Event)) -> Worker + Send + 'static,
{
    iced::stream::channel(64, move |output| async move {
        std::thread::spawn(move || {
            let mut tx = output.clone();
            let mut emit = |ev: Event| {
                let _ = iced::futures::executor::block_on(tx.send(Worker::Ev(ev)));
            };
            let result = work(&mut emit);
            let mut tx = output;
            let _ = iced::futures::executor::block_on(tx.send(result));
        });
    })
}

// ================================================================ components

fn body_text(s: &str) -> iced::widget::Text<'_> {
    text(s).size(15).line_height(1.5)
}

fn muted_text(s: &str) -> Element<'_, Message> {
    text(s).size(15).line_height(1.5).style(th::muted).into()
}

fn section_label(s: &str) -> Element<'_, Message> {
    text(s.to_uppercase()).size(12).font(SEMIBOLD).style(th::muted).into()
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> container::Container<'a, Message> {
    container(content).width(Length::Fill).style(th::card)
}

fn hairline<'a>() -> Element<'a, Message> {
    container(Space::new(Length::Fill, 1)).style(th::divider).into()
}

fn device_hero<'a>() -> Element<'a, Message> {
    container(center(svg(svg::Handle::from_memory(DEVICE_RENDER)).height(92)))
        .width(Length::Fill)
        .height(118)
        .padding(Padding::from([8, 24]))
        .style(th::hero)
        .into()
}

fn check_row(c: sysinfo::Check) -> Element<'static, Message> {
    let (kind, color) = match c.status {
        Status::Good => (Icon::Check, th::LIGHT.good),
        Status::Note => (Icon::Info, th::LIGHT.warn),
        Status::Blocker => (Icon::Cross, th::LIGHT.bad),
    };
    let mut label = column![text(c.label).size(14).font(MEDIUM)].spacing(3);
    if !c.detail.is_empty() {
        label = label.push(text(c.detail).size(13).style(th::muted));
    }
    container(
        row![badge_icon(kind, color, 24.0), container(label).width(Length::Fill), text(c.value).size(14).style(th::muted)]
            .spacing(14)
            .align_y(Alignment::Center),
    )
    .padding(Padding::from([9, 0]))
    .into()
}

fn callout(kind: Icon, color: iced::Color, msg: &str) -> Element<'_, Message> {
    container(row![icon(kind, color, 16.0), text(msg).size(14).line_height(1.45)].spacing(12).align_y(Alignment::Start))
        .padding(Padding::from([12, 16]))
        .width(Length::Fill)
        .style(move |_: &Theme| container::Style {
            background: Some(iced::Background::Color(th::alpha(color, 0.08))),
            border: iced::Border { color: th::alpha(color, 0.25), width: 1.0, radius: 12.0.into() },
            ..Default::default()
        })
        .into()
}

fn option_card<'a>(kind: Icon, title: &'a str, desc: String) -> Element<'a, Message> {
    card(
        column![
            badge_icon(kind, th::LIGHT.accent, 36.0),
            text(title).size(16).font(SEMIBOLD),
            text(desc).size(14).line_height(1.45).style(th::muted),
        ]
        .spacing(10),
    )
    .padding(20)
    .into()
}

/// The fixed key-approval password, shown large.
fn bullet<'a>(s: String) -> Element<'a, Message> {
    row![text("•").size(14).style(th::muted), text(s).size(14).line_height(1.4)].spacing(8).into()
}

fn password_card(password: &str) -> Element<'_, Message> {
    container(column![
        row![
            column![
                text("PASSWORD AT THE BLUE SCREEN").size(12).font(SEMIBOLD).style(th::muted),
                text(password).size(30).font(BOLD).style(th::accent_text),
            ]
            .spacing(6),
            horizontal_space(),
            text(if password == secureboot::MOK_PASSWORD { "all lowercase" } else { "" }).size(13).style(th::muted),
        ]
        .align_y(Alignment::Center),
        text("Only type this right after installing OpenFuse. If this screen appears unexpectedly, choose “Continue boot”.")
            .size(13)
            .line_height(1.4)
            .style(th::muted),
    ]
    .spacing(10))
    .padding(Padding::from([16, 22]))
    .width(Length::Fill)
    .style(|t: &Theme| {
        let tk = th::tokens(t);
        container::Style {
            background: Some(iced::Background::Color(th::alpha(tk.accent, 0.08))),
            border: iced::Border { color: th::alpha(tk.accent, 0.35), width: 1.0, radius: 14.0.into() },
            ..Default::default()
        }
    })
    .into()
}

fn numbered(n: u32, s: &str) -> Element<'_, Message> {
    row![
        container(text(n.to_string()).size(12).font(BOLD).style(th::accent_text))
            .width(24)
            .height(24)
            .center_x(24)
            .center_y(24)
            .style(|t: &Theme| th::badge(th::tokens(t).accent, 0.14)(t)),
        text(s).size(14).line_height(1.45),
    ]
    .spacing(12)
    .align_y(Alignment::Center)
    .into()
}

/// A slim bar with a gliding segment, for work without a known percentage.
fn indeterminate<'a>(phase: f32) -> Element<'a, Message> {
    let total = 1000u16;
    let seg = 280u16;
    let head = (phase * (total + seg) as f32) as u16;
    let start = head.saturating_sub(seg).min(total);
    let end = head.min(total);
    let left = start.max(1);
    let mid = end.saturating_sub(start).max(1);
    let right = total.saturating_sub(end).max(1);
    container(
        row![
            Space::new(Length::FillPortion(left), 6),
            container(Space::new(Length::Fill, 6))
                .width(Length::FillPortion(mid))
                .style(|t: &Theme| th::dot(th::tokens(t).accent)(t)),
            Space::new(Length::FillPortion(right), 6),
        ]
        .height(6),
    )
    .width(Length::Fill)
    .style(|t: &Theme| container::Style {
        background: Some(iced::Background::Color(th::tokens(t).surface_hover)),
        border: iced::border::rounded(999),
        ..Default::default()
    })
    .into()
}

fn btn(label: &str) -> button::Button<'_, Message> {
    button(text(label).size(15).font(MEDIUM)).padding(Padding::from([11, 22]))
}

fn primary(label: &str, msg: Message) -> Element<'_, Message> {
    btn(label).style(th::primary).on_press(msg).into()
}

fn primary_disabled(label: &str) -> Element<'_, Message> {
    btn(label).style(th::primary).into()
}

fn secondary(label: &str, msg: Message) -> Element<'_, Message> {
    btn(label).style(th::secondary).on_press(msg).into()
}

fn danger(label: &str, msg: Message) -> Element<'_, Message> {
    btn(label).style(th::danger).on_press(msg).into()
}

fn link(label: &str, msg: Message) -> Element<'_, Message> {
    button(text(label).size(14).font(MEDIUM)).padding(0).style(th::link).on_press(msg).into()
}

// ================================================================ icons

#[derive(Clone, Copy)]
enum Icon {
    Check,
    Cross,
    Info,
    Restart,
    Plug,
    Repair,
    Remove,
    Minimize,
    Close,
}

/// Small vector icons, drawn in a single colour.
fn icon<'a>(kind: Icon, color: iced::Color, size: f32) -> Element<'a, Message> {
    let body: &str = match kind {
        Icon::Check => r#"<path d="M5 12.5l4.5 4.5L19 7.5" fill="none" stroke="black" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
        Icon::Cross => r#"<path d="M7 7l10 10M17 7L7 17" fill="none" stroke="black" stroke-width="2.6" stroke-linecap="round"/>"#,
        Icon::Info => r#"<path d="M12 7v6.5" stroke="black" stroke-width="2.6" stroke-linecap="round"/><circle cx="12" cy="17.2" r="1.5" fill="black"/>"#,
        Icon::Restart => r#"<path d="M19 12a7 7 0 1 1-2.05-4.95" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round"/><path d="M19.5 4.5v4h-4" fill="none" stroke="black" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>"#,
        Icon::Plug => r#"<path d="M9 3v5M15 3v5M6.5 8h11v3a5.5 5.5 0 0 1-11 0zM12 16.5V21" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
        Icon::Repair => r#"<path d="M4 12a8 8 0 0 1 13.7-5.6L20 8.5M20 4v4.5h-4.5M20 12a8 8 0 0 1-13.7 5.6L4 15.5M4 20v-4.5h4.5" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
        Icon::Minimize => r#"<path d="M6 12h12" stroke="black" stroke-width="2" stroke-linecap="round"/>"#,
        Icon::Close => r#"<path d="M7 7l10 10M17 7L7 17" stroke="black" stroke-width="2" stroke-linecap="round"/>"#,
        Icon::Remove => r#"<path d="M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
    };
    let src = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">{body}</svg>"#);
    svg(svg::Handle::from_memory(src.into_bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

/// Window-control icon that follows the theme's text colour.
/// Drawn as vector geometry, not as a rasterised SVG: tiny SVGs get resampled at
/// fractional pixel offsets and their straight lines come out wobbly.
fn chrome_icon<'a>(kind: Icon) -> Element<'a, Message> {
    iced::widget::canvas(ChromeGlyph { kind }).width(30).height(30).into()
}

struct ChromeGlyph {
    kind: Icon,
}

impl iced::widget::canvas::Program<Message> for ChromeGlyph {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: iced::Rectangle,
        cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, LineCap, Path, Stroke};
        use iced::Point;
        let t = th::tokens(theme);
        let hovered = cursor.is_over(bounds);
        let color = match (self.kind, hovered) {
            (Icon::Close, true) => iced::Color::WHITE,
            (_, true) => t.text,
            _ => t.muted,
        };
        let mut frame = Frame::new(renderer, bounds.size());
        let c = frame.center();
        let stroke = Stroke::default().with_width(1.6).with_color(color).with_line_cap(LineCap::Round);
        match self.kind {
            Icon::Minimize => {
                frame.stroke(&Path::line(Point::new(c.x - 5.5, c.y), Point::new(c.x + 5.5, c.y)), stroke);
            }
            _ => {
                let d = 4.75;
                frame.stroke(&Path::line(Point::new(c.x - d, c.y - d), Point::new(c.x + d, c.y + d)), stroke);
                frame.stroke(&Path::line(Point::new(c.x + d, c.y - d), Point::new(c.x - d, c.y + d)), stroke);
            }
        }
        vec![frame.into_geometry()]
    }
}

/// Icon centred in a softly tinted circle.
fn badge_icon<'a>(kind: Icon, color: iced::Color, size: f32) -> Element<'a, Message> {
    container(icon(kind, color, size * 0.55))
        .width(size)
        .height(size)
        .center_x(size)
        .center_y(size)
        .style(th::badge(color, 0.14))
        .into()
}

// ================================================================ misc

fn simulated_info() -> SystemInfo {
    SystemInfo {
        distro_id: "ubuntu".into(),
        distro_name: "Ubuntu 26.10".into(),
        kernel: "7.3.0-10-generic".into(),
        secure_boot: Some(true),
        packagekit: true,
        pkexec: true,
        mokutil: true,
        headers: true,
        devices: sysinfo::usb_devices(),
        ..Default::default()
    }
}

fn save_png(path: &PathBuf, shot: &window::Screenshot) -> Result<(), Box<dyn std::error::Error>> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), shot.size.width, shot.size.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&shot.bytes)?;
    Ok(())
}
