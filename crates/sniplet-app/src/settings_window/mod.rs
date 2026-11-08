//! The Settings window: four tabs (General, Capture, Shortcuts, Save & Share) where every change
//! saves as it is made.
//!
//! The editor owns `Settings`. This window keeps a copy for its view. Each change applies to the
//! copy, and the same change goes to the editor, which saves it. What the editor learns back (the
//! save result, shortcut errors, update progress) returns through [`sync`].
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use rune::{
    AppSpec, FileDialogOptions, Icon, Key, KeyEvent, Pixels, Shortcut, TextEdit, Titlebar, Tx,
    Waker, WindowHandle, WindowOptions, WindowSpec,
};
use rune_kit::ShortcutRecorderState;
use sniplet_platform::{CloudUploadConfig, HotkeySettings, Settings};

use crate::{editor::Editor, ui::Inbox, ui::icons, updates::UpdateSummary};

mod view;

#[cfg(test)]
mod tests;

/// Settings pages in tab order. `page as usize` is part of the tab and content keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Capture,
    Shortcuts,
    Share,
}

impl Page {
    const ALL: [Self; 4] = [Self::General, Self::Capture, Self::Shortcuts, Self::Share];

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Capture => "Capture",
            Self::Shortcuts => "Shortcuts",
            Self::Share => "Save & Share",
        }
    }

    fn icon(self) -> &'static Icon {
        match self {
            Self::General => &icons::SETTINGS,
            Self::Capture => &icons::SCAN,
            Self::Shortcuts => &icons::KEYBOARD,
            Self::Share => &icons::CLOUD_UPLOAD,
        }
    }
}

/// Shortcut rows by group. Each index points into `HotkeySettings::slots`.
const SHORTCUT_GROUPS: [(&str, &[(usize, &str)]); 3] = [
    (
        "Capture",
        &[
            (0, "Capture area"),
            (1, "Capture screen"),
            (2, "Capture window"),
            (3, "Scrolling capture"),
            (6, "Capture text / QR"),
        ],
    ),
    (
        "Repeat",
        &[(4, "Repeat last area"), (5, "Capture active window")],
    ),
    ("App", &[(7, "Reopen editor")]),
];

/// The title band's height where the window draws its own title bar.
const TITLE_BAR: f32 = 48.0;

/// A control that owns a validation message. A notice without one shows as a toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Startup,
    BackgroundColor,
    Wallpaper,
    Upload,
}

struct Notice {
    text: String,
    error: bool,
    field: Option<Field>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UploadKind {
    SignedUrl,
    S3,
}

/// The two sliders. Each shows its value live while dragged and saves on release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slider {
    Padding,
    Speed,
}

impl Slider {
    /// Minimum, maximum and step.
    fn range(self) -> (u32, u32, u32) {
        match self {
            Self::Padding => (0, 120, 4),
            Self::Speed => (100, 700, 50),
        }
    }

    /// The slider's value for `settings`. Speed is the inverse of the scroll settle delay.
    fn value(self, settings: &Settings) -> u32 {
        match self {
            Self::Padding => settings.window_capture.padding.min(120),
            Self::Speed => 800 - settings.scroll_settle_ms.clamp(100, 700) as u32,
        }
    }

    fn apply(self, settings: &mut Settings, value: u32) {
        match self {
            Self::Padding => settings.window_capture.padding = value,
            Self::Speed => settings.scroll_settle_ms = u64::from(800 - value),
        }
    }
}

/// What the window shows from the editor's state. [`sync`] sends a new copy when it changes.
#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    save_result: Option<(u64, Option<String>)>,
    hotkey_error: Option<String>,
    updates: UpdateSummary,
    installing: bool,
}

impl Snapshot {
    fn of(editor: &Editor) -> Self {
        Self {
            save_result: editor.settings_save_result.clone(),
            hotkey_error: editor.hotkey_error.clone(),
            updates: editor.update_summary(),
            installing: editor.updates.installing(),
        }
    }
}

/// The editor's end of an open Settings window: its inbox and the last snapshot sent there.
struct Link {
    inbox: Inbox<SettingsUi>,
    sent: Snapshot,
}

impl Link {
    fn sync(&mut self, editor: &Editor) {
        let snapshot = Snapshot::of(editor);
        if snapshot != self.sent {
            self.sent = snapshot.clone();
            self.inbox.send(move |ui, _| ui.receive(snapshot));
        }
    }
}

struct Open {
    id: u64,
    handle: WindowHandle,
    link: Link,
}

thread_local! {
    /// The open Settings window. Its state clears this when the window closes.
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Open the settings window on `page`, or bring it forward on that page.
pub(crate) fn open(editor: &mut Editor, page: Page) -> anyhow::Result<()> {
    let open = OPEN.with_borrow(|open| {
        open.as_ref()
            .map(|open| (open.handle.clone(), open.link.inbox.clone()))
    });
    if let Some((handle, inbox)) = open {
        inbox.send(move |ui, _| ui.select_page(page));
        handle.focus();
        return Ok(());
    }
    anyhow::ensure!(
        !rune::is_headless(),
        "Settings opens its own window, which headless runs do not create"
    );
    let ui = SettingsUi::new(editor, page);
    ui.refresh_startup();
    let link = Link {
        inbox: ui.inbox.clone(),
        sent: ui.snapshot.clone(),
    };
    let id = ui.id;
    let handle = rune::open_window(
        WindowSpec::new("settings", app_spec())
            .state(ui)
            .titlebar(Titlebar::new(TITLE_BAR))
            .on_focus(on_focus)
            .options(WindowOptions {
                min_size: Some((560.0, 480.0)),
                ..Default::default()
            }),
    );
    OPEN.with_borrow_mut(|open| *open = Some(Open { id, handle, link }));
    Ok(())
}

/// The window's app. Its state comes from `open`, which reads the editor.
fn app_spec() -> AppSpec<SettingsUi> {
    AppSpec {
        min_size: Some((560.0, 480.0)),
        setup: Some(setup),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new(
            "Sniplet Settings",
            (680.0, 720.0),
            || unreachable!("open supplies the state"),
            view::view,
        )
    }
}

/// Send the open window what changed in the editor. The editor's `poll` calls it each frame.
pub(crate) fn sync(editor: &Editor) {
    OPEN.with_borrow_mut(|open| {
        if let Some(open) = open {
            open.link.sync(editor);
        }
    });
}

fn setup(ui: &mut SettingsUi, waker: Waker) {
    ui.inbox.set_waker(waker);
}

fn poll(ui: &mut SettingsUi, _: f64, tx: &mut Tx) -> bool {
    let changed = Inbox::drain(ui, |ui| &ui.inbox, tx);
    ui.after_input() || changed
}

/// Escape and Ctrl/Cmd W close the window. A recording shortcut takes its keys first.
fn on_key(_: &mut SettingsUi, event: &KeyEvent, tx: &mut Tx) -> bool {
    let close = matches!(event.key, Key::Escape) || event.is_command('w');
    if close {
        tx.close_window();
    }
    close
}

/// Leaving the window ends a recording, so global shortcuts work again. Coming back reads the
/// launch-at-login state, which system settings can change.
fn on_focus(ui: &mut SettingsUi, focused: bool, _: &mut Tx) {
    if focused {
        ui.refresh_startup();
    } else {
        ui.stop_recording();
        ui.after_input();
    }
}

#[derive(Default)]
struct Startup {
    enabled: bool,
    /// A change is in progress; the switch waits for it.
    busy: bool,
}

/// The Settings window's state.
pub(crate) struct SettingsUi {
    id: u64,
    page: Page,
    /// The window's copy of the editor's settings.
    settings: Settings,
    editor: Inbox<Editor>,
    inbox: Inbox<SettingsUi>,
    snapshot: Snapshot,
    /// The save result last shown, so each shows once.
    shown_save: u64,
    recorders: [ShortcutRecorderState; 8],
    /// Whether this window has paused the editor's global shortcuts for a recording.
    paused: bool,
    startup: Startup,
    background_color: TextEdit,
    color_focused: bool,
    upload: [TextEdit; 7],
    upload_kind: UploadKind,
    /// Shows the destination form. It can be open before a destination is saved.
    upload_open: bool,
    default_directory: Option<PathBuf>,
    previews: [Pixels; 4],
    /// A slider's value while it is dragged.
    sliding: Option<(Slider, u32)>,
    notice: Option<Notice>,
    notice_generation: u64,
}

impl SettingsUi {
    fn new(editor: &Editor, page: Page) -> Self {
        let settings = editor.settings.clone();
        let mut values: [String; 7] = Default::default();
        let mut upload_kind = UploadKind::SignedUrl;
        match &settings.cloud_upload {
            Some(CloudUploadConfig::PresignedPut { url, public_url }) => {
                values[0] = url.clone();
                values[1] = public_url.clone().unwrap_or_default();
            }
            Some(CloudUploadConfig::S3 {
                bucket,
                region,
                endpoint,
                key_prefix,
                public_base_url,
            }) => {
                upload_kind = UploadKind::S3;
                values[2] = bucket.clone();
                values[3] = region.clone();
                values[4] = endpoint.clone().unwrap_or_default();
                values[5] = key_prefix.clone();
                values[6] = public_base_url.clone().unwrap_or_default();
            }
            None => values[3] = "us-east-1".into(),
        }
        let snapshot = Snapshot::of(editor);
        let previews = std::array::from_fn(|_| Pixels::new());
        view::render_previews(&previews, settings.window_capture.color);
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            page,
            editor: editor.inbox.clone(),
            inbox: Inbox::default(),
            shown_save: snapshot
                .save_result
                .as_ref()
                .map_or(0, |(revision, _)| *revision),
            snapshot,
            recorders: recorders(&settings.hotkeys),
            paused: false,
            startup: Startup::default(),
            background_color: TextEdit::new(color_hex(settings.window_capture.color)),
            color_focused: false,
            upload: values.map(TextEdit::new),
            upload_kind,
            upload_open: settings.cloud_upload.is_some(),
            default_directory: sniplet_platform::default_export_directory().ok(),
            previews,
            sliding: None,
            notice: None,
            notice_generation: 0,
            settings,
        }
    }

    fn receive(&mut self, snapshot: Snapshot) {
        if let Some((revision, error)) = snapshot.save_result.clone()
            && revision != self.shown_save
        {
            self.shown_save = revision;
            match error {
                Some(error) => self.fail(error, None),
                // A late "Saved" never hides a message about a later change.
                None if self.notice.as_ref().is_some_and(|notice| notice.error) => {}
                None => self.show("Saved", false, None),
            }
        }
        self.snapshot = snapshot;
    }

    fn select_page(&mut self, page: Page) {
        self.page = page;
        self.stop_recording();
        self.clear_notice();
    }

    /// Apply one change to this window's copy and send the same change to the editor, which
    /// saves it in the background. The save result comes back as a toast.
    fn change(&mut self, change: impl Fn(&mut Settings) + Send + 'static) {
        let before = (
            self.settings.hotkeys.clone(),
            self.settings.window_capture.color,
        );
        change(&mut self.settings);
        self.editor.send(move |editor, _| {
            change(&mut editor.settings);
            editor.persist_settings();
        });
        if self.settings.hotkeys != before.0 {
            self.recorders = recorders(&self.settings.hotkeys);
        }
        if self.settings.window_capture.color != before.1 {
            view::render_previews(&self.previews, self.settings.window_capture.color);
        }
        self.stop_recording();
        self.clear_notice();
    }

    /// Show a message. Success messages hide after a short time; errors stay until the next
    /// change.
    fn show(&mut self, text: impl Into<String>, error: bool, field: Option<Field>) {
        self.notice_generation += 1;
        self.notice = Some(Notice {
            text: text.into(),
            error,
            field,
        });
        if !error {
            let generation = self.notice_generation;
            self.inbox.spawn(
                || std::thread::sleep(Duration::from_millis(1600)),
                move |ui, (), _| {
                    if ui.notice_generation == generation {
                        ui.clear_notice();
                    }
                },
            );
        }
    }

    fn fail(&mut self, text: impl Into<String>, field: Option<Field>) {
        self.show(text, true, field);
    }

    fn clear_notice(&mut self) {
        if self.notice.take().is_some() {
            self.notice_generation += 1;
        }
    }

    fn field_error(&self, field: Field) -> Option<&str> {
        self.notice
            .as_ref()
            .filter(|notice| notice.error && notice.field == Some(field))
            .map(|notice| notice.text.as_str())
    }

    /// Ask for a `poll` soon. Handlers the kit runs without a `Tx` (a recorder starting, a field
    /// losing focus) call it so `after_input` sees their change.
    fn wake(&self) {
        self.inbox.send(|_, _| {});
    }

    /// Work that follows input: pause global shortcuts while one is recorded, and save the
    /// background color when its field loses focus. True when it changed what the view shows.
    fn after_input(&mut self) -> bool {
        let recording = self.recorders.iter().any(|r| r.is_recording());
        if recording != self.paused {
            self.paused = recording;
            self.editor
                .send(move |editor, _| editor.hotkeys_paused = recording);
        }
        let blurred = self.color_focused && !self.background_color.focused;
        self.color_focused = self.background_color.focused;
        if blurred {
            self.save_background_color();
        }
        blurred
    }

    fn stop_recording(&mut self) {
        for recorder in &mut self.recorders {
            recorder.cancel();
        }
    }

    /// Read the launch-at-login state off the UI thread: OS startup APIs can wait on another
    /// process.
    fn refresh_startup(&self) {
        if self.startup.busy {
            return;
        }
        self.inbox.spawn(crate::startup::enabled, |ui, result, _| {
            if !ui.startup.busy
                && let Ok(enabled) = result
            {
                ui.startup.enabled = enabled;
            }
        });
    }

    fn set_startup(&mut self, enabled: bool) {
        self.startup.busy = true;
        self.clear_notice();
        self.inbox.spawn(
            move || crate::startup::set_enabled(enabled).map_err(|error| error.to_string()),
            move |ui, result, _| {
                ui.startup.busy = false;
                match result {
                    Ok(actual) => {
                        ui.startup.enabled = actual;
                        if actual == enabled {
                            ui.show("Saved", false, None);
                        } else {
                            ui.fail(
                                if cfg!(target_os = "macos") {
                                    "Turn on Sniplet in System Settings → General → Login Items."
                                } else {
                                    "Allow Sniplet to start in your system's startup settings."
                                },
                                Some(Field::Startup),
                            );
                        }
                    }
                    Err(error) => ui.fail(error, Some(Field::Startup)),
                }
            },
        );
    }

    fn choose_folder(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Choose screenshots folder".into(),
            directory: self
                .settings
                .screenshot_directory
                .clone()
                .or_else(|| self.default_directory.clone()),
            ..Default::default()
        };
        tx.open_folder(options, |ui: &mut SettingsUi, result, _| match result {
            Ok(Some(path)) => ui.change(move |s| s.screenshot_directory = Some(path.clone())),
            Ok(None) => {}
            Err(error) => ui.fail(error.to_string(), None),
        });
    }

    fn choose_wallpaper(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Choose wallpaper image".into(),
            ..Default::default()
        };
        tx.open_file(options, |ui: &mut SettingsUi, result, _| match result {
            Ok(Some(path)) => ui.inbox.spawn(
                move || sniplet_platform::load_wallpaper_image(&path).map(|_| path),
                |ui, result, _| match result {
                    Ok(path) => ui.change(move |s| s.window_capture.wallpaper = Some(path.clone())),
                    Err(error) => ui.fail(error, Some(Field::Wallpaper)),
                },
            ),
            Ok(None) => {}
            Err(error) => ui.fail(error.to_string(), Some(Field::Wallpaper)),
        });
    }

    fn set_background_color(&mut self, color: [u8; 3]) {
        self.background_color.replace_text(color_hex(color));
        self.change(move |s| s.window_capture.color = color);
    }

    fn save_background_color(&mut self) {
        let value = self.background_color.text.trim();
        let value = value.strip_prefix('#').unwrap_or(value);
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            self.fail(
                "Use a six-digit hex color, such as #F2F2F7.",
                Some(Field::BackgroundColor),
            );
            return;
        }
        let color = u32::from_str_radix(value, 16).unwrap();
        let color = [(color >> 16) as u8, (color >> 8) as u8, color as u8];
        if self.settings.window_capture.color == color {
            self.clear_notice();
            return;
        }
        self.change(move |s| s.window_capture.color = color);
    }

    /// Save one shortcut, or say why it cannot be used. An empty value turns it off.
    fn set_hotkey(&mut self, index: usize, value: String) -> Result<(), String> {
        let mut hotkeys = self.settings.hotkeys.clone();
        *hotkeys.slots_mut()[index] = value.clone();
        if !value.is_empty() {
            match crate::shell::conflicting_hotkey(&hotkeys.slots(), index) {
                Ok(None) => {}
                Ok(Some(other)) => {
                    return Err(format!(
                        "{} is already used by {}.",
                        shortcut_label(&value),
                        shortcut_name(other)
                    ));
                }
                Err(_) => return Err("That key can't be used in a shortcut.".into()),
            }
        }
        self.change(move |s| s.hotkeys = hotkeys.clone());
        Ok(())
    }

    /// `set_hotkey` from a button, which shows a refusal as a toast.
    fn press_hotkey(&mut self, index: usize, value: String) {
        if let Err(error) = self.set_hotkey(index, value) {
            self.fail(error, None);
        }
    }

    fn set_upload_open(&mut self, open: bool) {
        self.upload_open = open;
        if !open && self.settings.cloud_upload.is_some() {
            self.change(|s| s.cloud_upload = None);
        } else {
            self.clear_notice();
        }
    }

    fn save_upload(&mut self) {
        let values = self
            .upload
            .each_ref()
            .map(|edit| edit.text.trim().to_owned());
        let optional = |value: &String| (!value.is_empty()).then(|| value.clone());
        let config = match self.upload_kind {
            UploadKind::SignedUrl => CloudUploadConfig::PresignedPut {
                url: values[0].clone(),
                public_url: optional(&values[1]),
            },
            UploadKind::S3 => CloudUploadConfig::S3 {
                bucket: values[2].clone(),
                region: values[3].clone(),
                endpoint: optional(&values[4]),
                key_prefix: values[5].clone(),
                public_base_url: optional(&values[6]),
            },
        };
        if let Err(error) = config.validate() {
            self.fail(error.to_string(), Some(Field::Upload));
            return;
        }
        self.change(move |s| s.cloud_upload = Some(config.clone()));
    }
}

impl Drop for SettingsUi {
    /// A closed window turns global shortcuts back on and lets `open` create a new one.
    fn drop(&mut self) {
        if self.paused {
            self.editor.send(|editor, _| editor.hotkeys_paused = false);
        }
        let id = self.id;
        let _ = OPEN.try_with(|open| {
            let mut open = open.borrow_mut();
            if open.as_ref().is_some_and(|open| open.id == id) {
                *open = None;
            }
        });
    }
}

fn recorders(hotkeys: &HotkeySettings) -> [ShortcutRecorderState; 8] {
    let slots = hotkeys.slots();
    std::array::from_fn(|index| ShortcutRecorderState::new(shortcut(slots[index])))
}

/// A saved shortcut, such as `CommandOrControl+Shift+1`, as the recorder's chord. None when it
/// is off or unreadable.
fn shortcut(text: &str) -> Option<Shortcut> {
    if text.is_empty() {
        return None;
    }
    text.split('+')
        .map(|token| match token.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "commandorctrl" | "cmdorctrl" | "cmdorcontrol" => "mod",
            _ => token,
        })
        .collect::<Vec<_>>()
        .join("+")
        .parse()
        .ok()
}

/// A recorded chord in the saved form that `shell::parse_hotkey` reads. The platform's main
/// modifier (Cmd on Mac, Ctrl elsewhere) becomes `CommandOrControl`, like the defaults.
fn hotkey_text(shortcut: &Shortcut) -> String {
    let mods = shortcut.modifiers();
    let mac = cfg!(target_os = "macos");
    let (primary, secondary) = if mac {
        (mods.logo, mods.ctrl)
    } else {
        (mods.ctrl, mods.logo)
    };
    let mut tokens: Vec<String> = [
        (primary, "CommandOrControl"),
        (secondary, if mac { "Control" } else { "Super" }),
        (mods.alt, "Alt"),
        (mods.shift, "Shift"),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .map(|(_, token)| token.to_owned())
    .collect();
    // `Display` ends in the key's lowercase name, such as `2`, `f5` or `pageup`.
    let name = shortcut.to_string();
    let name = name.rsplit('+').next().unwrap_or_default();
    let function = name.len() > 1 && name.starts_with('f') && name[1..].parse::<u8>().is_ok();
    tokens.push(match name {
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "bracketleft" => "BracketLeft".into(),
        "bracketright" => "BracketRight".into(),
        _ if name.len() == 1 || function => name.to_uppercase(),
        _ => name[..1].to_uppercase() + &name[1..],
    });
    tokens.join("+")
}

/// A saved shortcut as key caps, such as `Ctrl+Shift+2` (`⇧⌘2` on Mac).
fn shortcut_label(text: &str) -> String {
    shortcut(text).map_or_else(
        || text.to_owned(),
        |shortcut| {
            shortcut
                .keycaps()
                .join(if cfg!(target_os = "macos") { "" } else { "+" })
        },
    )
}

fn shortcut_name(index: usize) -> &'static str {
    SHORTCUT_GROUPS
        .iter()
        .flat_map(|(_, items)| items.iter())
        .find(|(slot, _)| *slot == index)
        .map_or("another shortcut", |(_, name)| name)
}

/// Move the scrolling capture limit to the next multiple of 10, within 2–200.
fn step_limit(limit: usize, up: bool) -> usize {
    if up {
        (limit / 10 * 10 + 10).min(200)
    } else {
        (limit.div_ceil(10) * 10).saturating_sub(10).max(2)
    }
}

fn color_hex(color: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}

/// Show `path` in the system file manager, selected where the platform can.
fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
    if let Err(error) = result {
        eprintln!("Could not show {}: {error}", path.display());
    }
}
