//! Update checks, downloads and installs, the update card on the canvas, and the Updates
//! section of Settings. The editor owns the state; background work runs through its inbox.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use rune::{El, Icon, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, Card, Kit, SegmentedControl, State, Switch};
use sniplet_core::{Document, Project};
use sniplet_platform::{
    SettingsStore,
    updates::{self as platform, BuildIdentity, DownloadedUpdate, ReleaseChannel, UpdateRelease},
};

use crate::{
    editor::Editor,
    settings_window::view,
    ui::{self, Inbox, icons},
};

pub(crate) const RESTORE_ARG: &str = "--restore-update";
const RELEASES_URL: &str = "https://github.com/m4tta/sniplet/releases";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10);

/// A release found by a check.
#[derive(Debug)]
pub(crate) struct Release {
    version: String,
    size: u64,
    /// The release installs a different way, so the user gets it from the website.
    reinstall: bool,
    /// This copy cannot update itself, so the user gets the release from the website.
    manual: bool,
    /// Absent for test doubles, which cannot build a signed platform release.
    package: Option<UpdateRelease>,
}

impl PartialEq for Release {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version && self.size == other.size
    }
}

impl Release {
    /// The app can download and install this release itself.
    fn in_app(&self) -> bool {
        !self.reinstall && !self.manual
    }
}

impl From<UpdateRelease> for Release {
    fn from(package: UpdateRelease) -> Self {
        Self {
            version: package.version.clone(),
            size: package.size,
            reinstall: package.reinstall,
            manual: false,
            package: Some(package),
        }
    }
}

/// Network and process work, kept behind a trait so tests never download or restart.
pub(crate) trait Backend: Send + Sync {
    fn check(
        &self,
        channel: ReleaseChannel,
        current: &BuildIdentity,
    ) -> anyhow::Result<Option<Release>>;
    fn download(&self, release: &Release) -> anyhow::Result<Box<dyn Staged>>;
}

/// A downloaded update.
pub(crate) trait Staged: Send {
    /// Start the updater, which installs after the app quits and then opens the new version.
    /// `restore` opens the recovery snapshot there.
    fn install(self: Box<Self>, restore: bool) -> anyhow::Result<()>;
}

impl Staged for DownloadedUpdate {
    fn install(self: Box<Self>, restore: bool) -> anyhow::Result<()> {
        let args: &[&str] = if restore { &[RESTORE_ARG] } else { &[] };
        platform::install(*self, args)
    }
}

struct Platform;

impl Backend for Platform {
    fn check(
        &self,
        channel: ReleaseChannel,
        current: &BuildIdentity,
    ) -> anyhow::Result<Option<Release>> {
        let manual = !platform::can_update();
        Ok(platform::check(channel, current)?.map(|package| Release {
            manual,
            ..Release::from(package)
        }))
    }

    fn download(&self, release: &Release) -> anyhow::Result<Box<dyn Staged>> {
        let package = release
            .package
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("This release cannot be downloaded"))?;
        Ok(Box::new(platform::download(package)?))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Phase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(Arc<Release>),
    Downloading(Arc<Release>),
    Ready(Arc<Release>),
    Installing(Arc<Release>),
    CheckFailed(String),
    DownloadFailed(Arc<Release>, String),
    InstallFailed(Arc<Release>, String),
}

impl Phase {
    fn busy(&self) -> bool {
        matches!(self, Self::Downloading(_) | Self::Installing(_))
    }

    /// A download is running or done, so a check would only replace it.
    fn transferring(&self) -> bool {
        self.busy() || matches!(self, Self::Ready(_))
    }
}

pub(crate) struct Updates {
    pub(crate) phase: Phase,
    current: Arc<BuildIdentity>,
    backend: Arc<dyn Backend>,
    staged: Option<Box<dyn Staged>>,
    /// Changes with the channel, so results for an older channel are dropped.
    generation: u64,
    check_running: bool,
    check_pending: Option<bool>,
    last_check: Option<Instant>,
    next_automatic: Option<Instant>,
    pub(crate) notice_dismissed: bool,
}

impl Updates {
    pub(crate) fn new() -> Self {
        Self::with_backend(Arc::new(Platform))
    }

    pub(crate) fn with_backend(backend: Arc<dyn Backend>) -> Self {
        Self {
            phase: Phase::Idle,
            current: Arc::new(platform::current_build()),
            backend,
            staged: None,
            generation: 0,
            check_running: false,
            check_pending: None,
            last_check: None,
            next_automatic: None,
            notice_dismissed: false,
        }
    }

    pub(crate) fn installing(&self) -> bool {
        matches!(self.phase, Phase::Installing(_))
    }

    fn automatic_check_due(&self) -> bool {
        !self.check_running
            && !self.phase.transferring()
            && self
                .next_automatic
                .is_none_or(|next| Instant::now() >= next)
    }
}

enum InstallError {
    /// Nothing changed on disk; the staged update can be installed again.
    Prepare(anyhow::Error, Box<dyn Staged>),
    Install(anyhow::Error),
}

impl Editor {
    /// Check at startup and once a day. Debug and test builds only check when asked.
    pub(crate) fn schedule_updates(&mut self) {
        if cfg!(any(test, debug_assertions)) {
            return;
        }
        let inbox = self.inbox.clone();
        std::thread::spawn(move || {
            std::thread::sleep(FIRST_CHECK_DELAY);
            loop {
                inbox.send(|editor, _| {
                    if editor.settings.check_for_updates && editor.updates.automatic_check_due() {
                        editor.check_for_updates(true);
                    }
                });
                std::thread::sleep(RETRY_INTERVAL);
            }
        });
    }

    /// An automatic check downloads what it finds; a check from Settings only shows it.
    pub(crate) fn check_for_updates(&mut self, automatic: bool) {
        if self.updates.phase.transferring() {
            return;
        }
        self.updates.phase = Phase::Checking;
        if self.updates.check_running {
            self.updates.check_pending = Some(automatic);
            return;
        }
        self.updates.check_running = true;
        let generation = self.updates.generation;
        let channel = self.settings.update_channel;
        let current = self.updates.current.clone();
        let backend = self.updates.backend.clone();
        self.inbox.spawn(
            move || backend.check(channel, &current),
            move |editor, result, _| editor.finish_check(generation, automatic, result),
        );
    }

    fn finish_check(
        &mut self,
        generation: u64,
        automatic: bool,
        result: anyhow::Result<Option<Release>>,
    ) {
        let updates = &mut self.updates;
        updates.check_running = false;
        let pending = updates.check_pending.take();
        if generation != updates.generation {
            if let Some(automatic) = pending {
                self.check_for_updates(automatic);
            }
            return;
        }
        let now = Instant::now();
        updates.last_check = Some(now);
        updates.next_automatic = Some(
            now + if result.is_ok() {
                CHECK_INTERVAL
            } else {
                RETRY_INTERVAL
            },
        );
        updates.phase = match result {
            Ok(Some(release)) => Phase::Available(Arc::new(release)),
            Ok(None) => Phase::UpToDate,
            Err(error) => Phase::CheckFailed(error.to_string()),
        };
        if automatic && matches!(&self.updates.phase, Phase::Available(release) if release.in_app())
        {
            self.download_update();
        }
    }

    pub(crate) fn download_update(&mut self) {
        let release = match &self.updates.phase {
            Phase::Available(release)
            | Phase::DownloadFailed(release, _)
            | Phase::InstallFailed(release, _)
                if release.in_app() =>
            {
                release.clone()
            }
            _ => return,
        };
        self.updates.phase = Phase::Downloading(release.clone());
        let generation = self.updates.generation;
        let backend = self.updates.backend.clone();
        let package = release.clone();
        self.inbox.spawn(
            move || backend.download(&package),
            move |editor, result, _| {
                if editor.updates.generation != generation {
                    return;
                }
                match result {
                    Ok(staged) => {
                        editor.updates.staged = Some(staged);
                        editor.updates.phase = Phase::Ready(release);
                        editor.updates.notice_dismissed = false;
                    }
                    Err(error) => {
                        editor.updates.phase = Phase::DownloadFailed(release, error.to_string())
                    }
                }
            },
        );
    }

    /// Save settings and the open screenshot, start the updater, then quit. The updater
    /// installs and starts the new version.
    pub(crate) fn restart_to_update(&mut self) {
        let release = match &self.updates.phase {
            Phase::Ready(release) | Phase::InstallFailed(release, _) => release.clone(),
            _ => return,
        };
        let Some(staged) = self.updates.staged.take() else {
            self.download_update();
            return;
        };
        self.updates.phase = Phase::Installing(release.clone());
        self.install_when_saved(release, staged);
    }

    /// The single settings writer finishes first, so the final snapshot is the last write.
    fn install_when_saved(&mut self, release: Arc<Release>, staged: Box<dyn Staged>) {
        if self.settings_save_running {
            self.inbox.spawn(
                || std::thread::sleep(Duration::from_millis(10)),
                move |editor, (), _| editor.install_when_saved(release, staged),
            );
            return;
        }
        let settings = self.settings.clone();
        let store = self.settings_store();
        let document = self.document.clone();
        let install = move || {
            let prepared = store.and_then(|store| {
                store.save(&settings)?;
                document
                    .map(|document| save_recovery(&recovery_path(&store), &document))
                    .transpose()
            });
            let recovery = match prepared {
                Ok(recovery) => recovery,
                Err(error) => return Err(InstallError::Prepare(error, staged)),
            };
            staged
                .install(recovery.is_some())
                .map_err(InstallError::Install)
        };
        self.inbox.spawn(install, move |editor, result, tx| {
            editor.finish_install(release, result, tx)
        });
    }

    fn finish_install(
        &mut self,
        release: Arc<Release>,
        result: Result<(), InstallError>,
        tx: &mut Tx,
    ) {
        match result {
            Ok(()) => {
                self.updates.phase = Phase::Idle;
                tx.quit();
            }
            Err(InstallError::Prepare(error, staged)) => {
                let message = format!("Sniplet could not save your work first: {error}");
                self.updates.phase = Phase::InstallFailed(release, message);
                self.updates.staged = Some(staged);
            }
            Err(InstallError::Install(error)) => {
                self.updates.phase = Phase::InstallFailed(release, error.to_string());
            }
        }
        self.updates.notice_dismissed = false;
    }

    /// A new channel drops what the old one found, and checks again.
    pub(crate) fn set_update_channel(&mut self, channel: ReleaseChannel) {
        if self.settings.update_channel == channel || self.updates.phase.busy() {
            return;
        }
        self.settings.update_channel = channel;
        self.persist_settings();
        self.updates.generation += 1;
        self.updates.staged = None;
        self.updates.phase = Phase::Idle;
        self.updates.next_automatic = None;
        self.check_for_updates(false);
    }

    /// A card over the canvas while an update is ready, installs, or failed to install.
    pub(crate) fn render_update_notice(&self, k: &Kit) -> Option<El> {
        if self.updates.notice_dismissed {
            return None;
        }
        let phase = &self.updates.phase;
        let (title, description, glyph, failed) = match phase {
            Phase::Available(release) if release.reinstall => (
                format!("Sniplet {} needs a new install", release.version),
                "This version installs a new way. Download it from the website and install it over this copy. Your settings stay.".to_owned(),
                &icons::DOWNLOAD,
                false,
            ),
            Phase::Ready(release) => (
                format!("Sniplet {} is ready", release.version),
                "Sniplet restarts. Your screenshot and edits come back.".to_owned(),
                &icons::DOWNLOAD,
                false,
            ),
            Phase::Installing(release) => (
                format!("Installing Sniplet {}…", release.version),
                "Sniplet restarts when it is done.".to_owned(),
                &icons::DOWNLOAD,
                false,
            ),
            Phase::InstallFailed(_, error) => (
                failure_title(phase).to_owned(),
                error.clone(),
                &icons::CIRCLE_ALERT,
                true,
            ),
            _ => return None,
        };
        let installing = matches!(phase, Phase::Installing(_));
        let reinstall = matches!(phase, Phase::Available(release) if release.reinstall);
        let card = Card::new()
            .accent(if failed {
                k.c.danger.icon
            } else {
                k.c.bg_accent
            })
            .title(title, Some(glyph))
            .description(description)
            .action(
                Button::new("Later")
                    .variant(ButtonVariant::Ghost)
                    .disabled(installing)
                    .on_press(|s: &mut Editor| s.updates.notice_dismissed = true)
                    .build(k)
                    .key("update-later"),
            )
            .action(if reinstall {
                Button::new("Open website")
                    .variant(ButtonVariant::Contrast)
                    .on_press_tx(|s: &mut Editor, tx: &mut Tx| {
                        s.updates.notice_dismissed = true;
                        let _ = tx.open_url(RELEASES_URL);
                    })
                    .build(k)
                    .key("update-website")
            } else {
                Button::new(if failed {
                    "Try again"
                } else {
                    "Restart & update"
                })
                .variant(ButtonVariant::Contrast)
                .disabled(installing)
                .on_press(|s: &mut Editor| s.restart_to_update())
                .build(k)
                .key("update-restart")
            })
            .build(k);
        Some(
            ui::surface::<Editor>(k, el().w(440.0).padding(4.0))
                .key("update-notice")
                .pin(None, Some(12.0), Some(12.0), None)
                .child(card),
        )
    }

    /// After Later, the app menu shows a dot and this row label until the update is used.
    pub(crate) fn waiting_update_label(&self) -> Option<String> {
        if !self.updates.notice_dismissed {
            return None;
        }
        match &self.updates.phase {
            Phase::Available(release) if release.reinstall => {
                Some(format!("Get Sniplet {} from the website", release.version))
            }
            Phase::Ready(release) | Phase::InstallFailed(release, _) => {
                Some(format!("Update to Sniplet {}", release.version))
            }
            _ => None,
        }
    }

    pub(crate) fn show_update_notice(&mut self) {
        self.panel = None;
        self.updates.notice_dismissed = false;
    }

    pub(crate) fn update_summary(&self) -> UpdateSummary {
        UpdateSummary {
            version: self.updates.current.version.clone(),
            channel: self.settings.update_channel,
            automatic: self.settings.check_for_updates,
            phase: self.updates.phase.clone(),
            last_check: self.updates.last_check,
        }
    }
}

/// The whole editor window while an update installs, so nothing changes after the recovery
/// snapshot.
pub(crate) fn installing_screen(k: &Kit) -> El {
    col()
        .key("update-installing")
        .w_full()
        .h_full()
        .center()
        .gap(12.0)
        .padding(24.0)
        .fill(k.c.page)
        .child(rune_kit::Spinner::new().build(k))
        .child(
            text("Installing update…")
                .font(k.ty.body_medium)
                .color(k.c.text_base),
        )
        .child(
            text("Sniplet saves your work and opens again after the update.")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        )
}

/// What the Settings window shows about updates. The editor owns the update state; Settings
/// gets this copy whenever it changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct UpdateSummary {
    /// The running version.
    pub(crate) version: String,
    pub(crate) channel: ReleaseChannel,
    /// Check for updates automatically.
    pub(crate) automatic: bool,
    pub(crate) phase: Phase,
    pub(crate) last_check: Option<Instant>,
}

// Settings receives the summary through its window's inbox, from the editor's thread.
const _: () = {
    const fn send<T: Send>() {}
    send::<UpdateSummary>()
};

/// The Updates section of Settings. Its buttons send their actions to the editor's inbox, so it
/// works in any window's state `S`.
pub(crate) fn settings_section<S: 'static>(
    k: &Kit,
    summary: &UpdateSummary,
    editor: &Inbox<Editor>,
) -> El {
    let phase = &summary.phase;
    let version = &summary.version;
    let current = format!("You have {version}. Checking {}.", summary.channel.label());
    let last_check = summary
        .last_check
        .map(|time| format!(" Last check: {}.", elapsed_label(time.elapsed())))
        .unwrap_or_default();
    let (title, description, glyph, failed): (String, String, &'static Icon, bool) = match phase {
        Phase::Idle => (
            format!("Sniplet {version}"),
            format!("{current}{last_check}"),
            &icons::REFRESH_CW,
            false,
        ),
        Phase::Checking => (
            "Checking for updates…".into(),
            current.clone(),
            &icons::REFRESH_CW,
            false,
        ),
        Phase::UpToDate => (
            "No newer update is available".into(),
            format!("{current}{last_check}"),
            &icons::CIRCLE_CHECK,
            false,
        ),
        Phase::Available(release) if release.reinstall => (
            format!("Sniplet {} needs a new install", release.version),
            format!("You have {version}. Get the new version from the website."),
            &icons::DOWNLOAD,
            false,
        ),
        Phase::Available(release) if release.manual => (
            format!("Sniplet {} is available", release.version),
            format!(
                "You have {version}. This copy cannot update itself. Get the new version from the website."
            ),
            &icons::DOWNLOAD,
            false,
        ),
        Phase::Available(release) => (
            format!("Sniplet {} is available", release.version),
            format!("{current} Size: {}.{last_check}", size_label(release.size)),
            &icons::DOWNLOAD,
            false,
        ),
        Phase::Downloading(release) => (
            format!("Downloading {}…", release.version),
            format!(
                "Size: {}. You can use Sniplet while you wait.",
                size_label(release.size)
            ),
            &icons::DOWNLOAD,
            false,
        ),
        Phase::Ready(release) => (
            format!("Sniplet {} is ready", release.version),
            "Sniplet restarts. Your screenshot and edits come back.".into(),
            &icons::CIRCLE_CHECK,
            false,
        ),
        Phase::Installing(release) => (
            format!("Installing {}…", release.version),
            "Sniplet restarts when it is done.".into(),
            &icons::DOWNLOAD,
            false,
        ),
        Phase::CheckFailed(error) => (
            failure_title(phase).into(),
            format!("Make sure that you are connected to the internet. Then try again. ({error})"),
            &icons::CIRCLE_ALERT,
            true,
        ),
        Phase::DownloadFailed(_, error) => (
            failure_title(phase).into(),
            format!("Check your internet connection. Then try again. ({error})"),
            &icons::CIRCLE_ALERT,
            true,
        ),
        Phase::InstallFailed(_, error) => (
            failure_title(phase).into(),
            format!("{current} Try again, or get the new version from the website. ({error})"),
            &icons::CIRCLE_ALERT,
            true,
        ),
    };
    // A button that runs `run` on the editor.
    let send = |run: fn(&mut Editor)| {
        let editor = editor.clone();
        move |_: &mut S, _: &mut Tx| editor.send(move |editor, _| run(editor))
    };
    let check: fn(&mut Editor) = |editor| editor.check_for_updates(false);
    let download: fn(&mut Editor) = Editor::download_update;
    let install: fn(&mut Editor) = Editor::restart_to_update;
    let mut controls = row().gap(8.0).center_y();
    let button = |label: &str, primary: bool| {
        Button::new(label).variant(if primary {
            ButtonVariant::Contrast
        } else {
            ButtonVariant::Neutral
        })
    };
    if !phase.transferring() {
        let checking = matches!(phase, Phase::Checking);
        controls = controls.child(
            button("Check now", false)
                .icon(&icons::REFRESH_CW)
                .disabled(checking)
                .on_press_tx(send(check))
                .build(k)
                .key("update-check"),
        );
    }
    controls = match phase {
        Phase::Available(release) if !release.in_app() => controls.child(
            button("Open website", true)
                .icon(&icons::GLOBE)
                .on_press_tx(|_: &mut S, tx: &mut Tx| {
                    let _ = tx.open_url(RELEASES_URL);
                })
                .build(k)
                .key("update-website"),
        ),
        Phase::Available(_) | Phase::Downloading(_) => controls.child(
            button("Download", true)
                .icon(&icons::DOWNLOAD)
                .disabled(matches!(phase, Phase::Downloading(_)))
                .on_press_tx(send(download))
                .build(k)
                .key("update-download"),
        ),
        Phase::DownloadFailed(..) => controls.child(
            button("Try again", false)
                .on_press_tx(send(download))
                .build(k)
                .key("update-download-retry"),
        ),
        Phase::Ready(_) | Phase::Installing(_) => controls.child(
            button("Restart & update", true)
                .disabled(matches!(phase, Phase::Installing(_)))
                .on_press_tx(send(install))
                .build(k)
                .key("update-install"),
        ),
        Phase::InstallFailed(..) => controls
            .child(
                button("Open website", false)
                    .icon(&icons::GLOBE)
                    .on_press_tx(|_: &mut S, tx: &mut Tx| {
                        let _ = tx.open_url(RELEASES_URL);
                    })
                    .build(k)
                    .key("update-website"),
            )
            .child(
                button("Try again", false)
                    .on_press_tx(send(install))
                    .build(k)
                    .key("update-install-retry"),
            ),
        _ => controls,
    };
    let busy = phase.busy();
    let choices = [ReleaseChannel::Stable, ReleaseChannel::Nightly];
    let mut channels = SegmentedControl::new(
        &choices.map(ReleaseChannel::label),
        choices.iter().position(|choice| *choice == summary.channel),
    )
    .label("Update channel")
    .fit()
    .on_pick({
        let editor = editor.clone();
        move |_: &mut S, index, _| {
            let choice = choices[index];
            editor.send(move |editor, _| editor.set_update_channel(choice))
        }
    });
    if busy {
        channels = channels
            .segment_state(0, State::Disabled)
            .segment_state(1, State::Disabled);
    }
    let channels = channels.build(k).key("update-channel");
    let automatic = {
        let editor = editor.clone();
        let on = summary.automatic;
        Switch::new(on)
            .label("Check for updates automatically")
            .hide_label()
            .on_toggle(move |_: &mut S| {
                editor.send(move |editor, _| {
                    editor.settings.check_for_updates = !on;
                    editor.persist_settings();
                })
            })
            .build(k)
            .key("check-for-updates")
    };
    view::section(
        k,
        "Updates",
        view::card(k)
            .key("settings-updates")
            .child(
                row()
                    .key("update-status")
                    .padding(16.0)
                    .gap(12.0)
                    .center_y()
                    .child(status_icon(k, glyph, failed))
                    .child(status_text(k, title, description))
                    .child(controls),
            )
            .child(view::rule(k))
            .child(view::setting_row(
                k,
                "Update channel",
                "Stable is tested. Nightly can have bugs. Switching channels never installs an older version.",
                channels,
            ))
            .child(view::rule(k))
            .child(view::setting_row(
                k,
                "Check for updates automatically",
                "One time each day. Sniplet asks before it installs.",
                automatic,
            )),
    )
}

fn failure_title(phase: &Phase) -> &'static str {
    match phase {
        Phase::CheckFailed(_) => "Sniplet could not check for updates",
        Phase::DownloadFailed(..) => "Sniplet could not download the update",
        _ => "The update did not install",
    }
}

/// A status's title over its description, taking the row's spare width.
fn status_text(k: &Kit, title: String, description: String) -> El {
    col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(4.0)
        .child(text(title).font(k.ty.ui_medium).color(k.c.text_base).wrap())
        .child(view::note(k, description))
}

/// The status's icon on a tinted square: red for failures, green for done, else the accent.
fn status_icon(k: &Kit, glyph: &'static Icon, failed: bool) -> El {
    let (fill, color) = if failed {
        (k.c.danger.bg, k.c.danger.icon)
    } else if std::ptr::eq(glyph, &icons::CIRCLE_CHECK) {
        (k.c.success.bg, k.c.success.icon)
    } else {
        (k.c.bg_accent.alpha(0.16), k.c.bg_accent)
    };
    el().size(30.0, 30.0)
        .center()
        .radius(k.radius.lg)
        .fill(fill)
        .child(icon(glyph, 16.0).color(color))
}

fn size_label(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

fn elapsed_label(elapsed: Duration) -> String {
    let minutes = elapsed.as_secs() / 60;
    match minutes {
        0 => "just now".into(),
        1 => "1 minute ago".into(),
        2..=59 => format!("{minutes} minutes ago"),
        60..=119 => "1 hour ago".into(),
        _ => format!("{} hours ago", minutes / 60),
    }
}

/// The fixed project that carries the open screenshot across an update restart.
pub(crate) fn recovery_path(store: &SettingsStore) -> PathBuf {
    store.path().with_file_name("update-recovery.sniplet")
}

fn save_recovery(project: &Path, document: &Document) -> anyhow::Result<PathBuf> {
    if let Some(parent) = project.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let image = project.with_extension("png");
    document.original().save(&image)?;
    let source = PathBuf::from(image.file_name().unwrap_or_default());
    document.to_project(source).save(project)?;
    Ok(project.to_owned())
}

/// Open the screenshot saved before an update. Clear it after the app window opens.
pub(crate) fn restore(store: &SettingsStore) -> anyhow::Result<Document> {
    let project = recovery_path(store);
    let document = Project::load(&project)?.open_document()?;
    Ok(document)
}

impl Editor {
    /// A later launch handed over `--restore-update`: bring back the screenshot saved before the
    /// update. Work already open in the editor wins, and the saved screenshot stays on disk.
    pub(crate) fn restore_forwarded_update(&mut self) {
        if self.document.is_some() || self.updates.installing() {
            return;
        }
        let Ok(store) = self.settings_store() else {
            return;
        };
        if let Ok(document) = restore(&store) {
            self.load(document, "Sniplet is updated. Your screenshot is back.");
            clear_recovery(&store);
        }
    }
}

pub(crate) fn clear_recovery(store: &SettingsStore) {
    let project = recovery_path(store);
    let _ = std::fs::remove_file(project.with_extension("png"));
    let _ = std::fs::remove_file(project);
}

type Scene = (&'static str, fn(&mut Editor));

/// Headless scenes for `--snap --scene NAME`: the update card, the waiting app menu, and the
/// install screen, with a made-up release.
pub(crate) const SCENES: &[Scene] = &[
    ("update-ready", |s| {
        s.updates.phase = Phase::Ready(scene_release())
    }),
    ("update-failed", |s| {
        s.updates.phase = Phase::InstallFailed(scene_release(), "Disk is full".into())
    }),
    ("update-waiting", |s| {
        s.updates.phase = Phase::Ready(scene_release());
        s.updates.notice_dismissed = true;
        s.panel = Some(crate::editor::Panel::Menu);
    }),
    ("update-installing", |s| {
        s.updates.phase = Phase::Installing(scene_release())
    }),
    ("update-reinstall", |s| {
        s.updates.phase = Phase::Available(Arc::new(Release {
            version: "9.0.0".into(),
            size: 0,
            reinstall: true,
            manual: false,
            package: None,
        }))
    }),
];

fn scene_release() -> Arc<Release> {
    Arc::new(Release {
        version: "9.0.0".into(),
        size: 6_400_000,
        reinstall: false,
        manual: false,
        package: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<String>>,
        /// While a test holds this lock, backend calls cannot finish, so the test can look at the
        /// editor while a check or download runs.
        gate: Mutex<()>,
        fail_check: AtomicBool,
        /// Checks find a release that needs a new install.
        reinstall: AtomicBool,
        /// Checks run in a copy that cannot update itself.
        manual: AtomicBool,
        fail_install: AtomicBool,
    }

    struct FakeStaged(Arc<Fake>, String);

    impl Staged for FakeStaged {
        fn install(self: Box<Self>, restore: bool) -> anyhow::Result<()> {
            self.0.record(format!("install {} {restore}", self.1));
            if self.0.fail_install.load(Ordering::SeqCst) {
                anyhow::bail!("disk is full");
            }
            Ok(())
        }
    }

    struct FakeBackend(Arc<Fake>);

    impl Fake {
        /// Log a backend call, then wait while a test holds `gate`.
        fn record(&self, entry: String) {
            self.log.lock().unwrap().push(entry);
            drop(self.gate.lock().unwrap());
        }

        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.log.lock().unwrap())
        }
    }

    impl Backend for FakeBackend {
        fn check(
            &self,
            channel: ReleaseChannel,
            _: &BuildIdentity,
        ) -> anyhow::Result<Option<Release>> {
            self.0.record(format!("check {}", channel.label()));
            if self.0.fail_check.load(Ordering::SeqCst) {
                anyhow::bail!("offline");
            }
            Ok(Some(Release {
                version: match channel {
                    ReleaseChannel::Stable => "9.0.0".into(),
                    ReleaseChannel::Nightly => "9.1.0-nightly.20261104.1".into(),
                },
                size: 6_400_000,
                reinstall: self.0.reinstall.load(Ordering::SeqCst),
                manual: self.0.manual.load(Ordering::SeqCst),
                package: None,
            }))
        }

        fn download(&self, release: &Release) -> anyhow::Result<Box<dyn Staged>> {
            self.0.record(format!("download {}", release.version));
            Ok(Box::new(FakeStaged(
                self.0.clone(),
                release.version.clone(),
            )))
        }
    }

    /// An editor with an image, a fake backend, and settings in a new temporary folder.
    fn fixture() -> (Editor, Arc<Fake>, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let fake = Arc::new(Fake::default());
        let mut editor = Editor::new(
            Some(Document::new(image::RgbaImage::from_pixel(
                80,
                60,
                image::Rgba([20, 40, 60, 255]),
            ))),
            Default::default(),
            1.0,
        );
        editor.settings_path = Some(directory.path().join("settings.json"));
        editor.updates = Updates::with_backend(Arc::new(FakeBackend(fake.clone())));
        (editor, fake, directory)
    }

    /// Run the editor's inbox with `tx` until its background work is done.
    fn settle(editor: &mut Editor, tx: &mut Tx) {
        for _ in 0..5000 {
            Inbox::drain(editor, |editor| &editor.inbox, tx);
            let updates = &editor.updates;
            let running = updates.check_running
                || matches!(updates.phase, Phase::Downloading(_) | Phase::Installing(_))
                || editor.settings_save_running;
            if !running {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("the background work did not finish");
    }

    /// The Settings section in its own runtime, sending to `editor`'s inbox.
    struct Section {
        summary: UpdateSummary,
        editor: Inbox<Editor>,
    }

    fn section_view(s: &Section, _: &mut rune::Cx<Section>) -> El {
        let k = ui::kit(true);
        col()
            .w_full()
            .h_full()
            .padding(16.0)
            .fill(k.c.page)
            .child(settings_section::<Section>(&k, &s.summary, &s.editor))
    }

    fn env() -> rune::Env {
        rune::Env::new(true, 2.0)
    }

    fn click<S: 'static>(rt: &mut rune::Runtime<S>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is not shown"));
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn a_channel_change_drops_the_old_result_and_locks_while_downloading() {
        let (mut editor, fake, directory) = fixture();
        let mut settings = rune::Runtime::new(
            Section {
                summary: editor.update_summary(),
                editor: editor.inbox.clone(),
            },
            section_view,
            (900.0, 600.0),
            env(),
        );
        settings.frame(0.0);
        // Settings shows the editor's latest summary after each change.
        let show = |editor: &mut Editor, settings: &mut rune::Runtime<Section>| {
            let summary = editor.update_summary();
            settings.update(|s, _| s.summary = summary);
            settings.settle_all();
            settings.frame(1.0 / 60.0);
        };
        let sync = |editor: &mut Editor, settings: &mut rune::Runtime<Section>| {
            settle(editor, &mut Tx::default());
            show(editor, settings);
        };

        fake.fail_check.store(true, Ordering::SeqCst);
        click(&mut settings, "update-check");
        sync(&mut editor, &mut settings);
        assert!(matches!(editor.updates.phase, Phase::CheckFailed(_)));

        fake.fail_check.store(false, Ordering::SeqCst);
        // Hold the backend, so the checks still run when the test looks at the phase. A drain
        // also runs results that come back during it, so instant checks could finish first.
        let held = fake.gate.lock().unwrap();
        click(&mut settings, "update-check");
        click(&mut settings, "update-channel/segment1");
        Inbox::drain(&mut editor, |e| &e.inbox, &mut Tx::default());
        assert!(matches!(editor.updates.phase, Phase::Checking));
        drop(held);
        sync(&mut editor, &mut settings);
        assert_eq!(
            fake.take(),
            ["check Stable", "check Stable", "check Nightly"]
        );
        let Phase::Available(release) = editor.updates.phase.clone() else {
            panic!("the Nightly check must show its release");
        };
        assert_eq!(release.version, "9.1.0-nightly.20261104.1");

        // Hold the download until the channel click is queued, so the click lands mid-download.
        let held = fake.gate.lock().unwrap();
        click(&mut settings, "update-download");
        Inbox::drain(&mut editor, |e| &e.inbox, &mut Tx::default());
        assert!(matches!(editor.updates.phase, Phase::Downloading(_)));
        show(&mut editor, &mut settings);
        assert!(
            settings.find("update-check").is_none(),
            "no check while downloading"
        );
        click(&mut settings, "update-channel/segment0");
        drop(held);
        sync(&mut editor, &mut settings);
        assert_eq!(editor.settings.update_channel, ReleaseChannel::Nightly);
        assert_eq!(fake.take(), ["download 9.1.0-nightly.20261104.1"]);
        assert!(matches!(editor.updates.phase, Phase::Ready(_)));
        let store = SettingsStore::at(directory.path().join("settings.json"));
        assert_eq!(
            store.load().unwrap().update_channel,
            ReleaseChannel::Nightly
        );
        assert!(editor.render_update_notice(&ui::kit(true)).is_some());

        click(&mut settings, "update-channel/segment0");
        sync(&mut editor, &mut settings);
        assert_eq!(fake.take(), ["check Stable"]);
        let Phase::Available(release) = editor.updates.phase.clone() else {
            panic!("a channel change must replace the ready Nightly update");
        };
        assert_eq!(release.version, "9.0.0");
        assert!(editor.render_update_notice(&ui::kit(true)).is_none());
    }

    #[test]
    fn later_leaves_a_menu_dot_and_the_menu_brings_the_notice_back() {
        let (mut editor, _fake, _directory) = fixture();
        editor.check_for_updates(true);
        settle(&mut editor, &mut Tx::default());
        assert!(matches!(editor.updates.phase, Phase::Ready(_)));
        let mut rt = rune::Runtime::new(editor, crate::editor::view, (1280.0, 850.0), env());
        rt.frame(0.0);
        assert!(rt.find("update-notice").is_some());
        click(&mut rt, "update-later");
        assert!(rt.find("update-notice").is_none());
        assert!(rt.find("update-dot").is_some());
        click(&mut rt, "app-menu");
        click(&mut rt, "menu-update");
        assert!(rt.find("update-notice").is_some());
        assert!(rt.find("update-dot").is_none());
        assert!(rt.find("app-menu-popover").is_none());
    }

    #[test]
    fn a_failed_install_keeps_the_document_and_a_retry_installs_with_recovery() {
        let (mut editor, fake, directory) = fixture();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        let original = editor.export_pixels().unwrap();
        editor.settings.check_for_updates = false;
        editor.check_for_updates(true);
        settle(&mut editor, &mut Tx::default());
        fake.take();
        let mut rt = rune::Runtime::new(editor, crate::editor::view, (1280.0, 850.0), env());
        rt.frame(0.0);
        // Press the card's button, then let the install finish.
        let restart = |rt: &mut rune::Runtime<Editor>| {
            click(rt, "update-restart");
            rt.update(settle);
            rt.frame(1.0 / 60.0);
        };

        fake.fail_install.store(true, Ordering::SeqCst);
        restart(&mut rt);
        assert_eq!(fake.take(), ["install 9.0.0 true"]);
        assert!(matches!(rt.state.updates.phase, Phase::InstallFailed(..)));
        assert_eq!(rt.state.export_pixels().unwrap(), original);
        assert!(!store.load().unwrap().check_for_updates);
        assert!(recovery_path(&store).exists());
        assert_eq!(rt.shell_request, None);

        // The failed install used up the staged package, so a retry downloads it again.
        // Edits made after the failure must replace the earlier snapshot.
        fake.fail_install.store(false, Ordering::SeqCst);
        restart(&mut rt);
        assert_eq!(fake.take(), ["download 9.0.0"]);
        rt.update(|editor, _| {
            editor
                .document
                .as_mut()
                .unwrap()
                .set_crop(sniplet_core::ImageRect::new(0.0, 0.0, 30.0, 20.0));
        });
        let latest = rt.state.export_pixels().unwrap();
        assert_ne!(latest.dimensions(), original.dimensions());
        rt.frame(1.0 / 60.0);
        restart(&mut rt);
        assert_eq!(fake.take(), ["install 9.0.0 true"]);
        assert_eq!(rt.shell_request, Some(rune::Shell::Quit));
        let restored = restore(&store).unwrap();
        assert_eq!(
            restored.render(&crate::editor::render_options()).unwrap(),
            latest
        );
        clear_recovery(&store);
        assert!(!recovery_path(&store).exists());
        assert!(!recovery_path(&store).with_extension("png").exists());
        assert!(restore(&store).is_err());
    }

    #[test]
    fn a_forwarded_restore_fills_an_empty_editor_and_never_replaces_open_work() {
        let (busy, _fake, directory) = fixture();
        let store = SettingsStore::at(directory.path().join("settings.json"));
        save_recovery(&recovery_path(&store), busy.document.as_ref().unwrap()).unwrap();

        let mut busy = busy;
        let before = busy.export_pixels().unwrap();
        busy.restore_forwarded_update();
        assert_eq!(busy.export_pixels().unwrap(), before);
        assert!(recovery_path(&store).exists());

        let mut empty = Editor::new(None, Default::default(), 1.0);
        empty.settings_path = busy.settings_path.clone();
        empty.restore_forwarded_update();
        assert_eq!(empty.export_pixels().unwrap(), before);
        assert!(!recovery_path(&store).exists());
    }

    #[test]
    fn a_release_that_needs_a_new_install_links_to_the_website_and_never_downloads() {
        let (mut editor, fake, _directory) = fixture();
        fake.reinstall.store(true, Ordering::SeqCst);
        editor.check_for_updates(true);
        settle(&mut editor, &mut Tx::default());
        editor.download_update();
        settle(&mut editor, &mut Tx::default());
        assert_eq!(fake.take(), ["check Stable"], "it must never download");
        let mut rt = rune::Runtime::new(editor, crate::editor::view, (1280.0, 850.0), env());
        rt.frame(0.0);
        assert!(rt.find("update-restart").is_none());
        click(&mut rt, "update-website");
        assert_eq!(rt.take_url_requests(), [RELEASES_URL]);
        assert!(rt.find("update-notice").is_none());
        assert_eq!(
            rt.state.waiting_update_label().as_deref(),
            Some("Get Sniplet 9.0.0 from the website")
        );
    }

    #[test]
    fn a_copy_that_cannot_update_itself_links_to_the_website_and_never_downloads() {
        let (mut editor, fake, _directory) = fixture();
        fake.manual.store(true, Ordering::SeqCst);
        editor.check_for_updates(true);
        settle(&mut editor, &mut Tx::default());
        editor.download_update();
        settle(&mut editor, &mut Tx::default());
        assert_eq!(fake.take(), ["check Stable"], "it must never download");
        assert!(editor.render_update_notice(&ui::kit(true)).is_none());
        let mut settings = rune::Runtime::new(
            Section {
                summary: editor.update_summary(),
                editor: editor.inbox.clone(),
            },
            section_view,
            (900.0, 600.0),
            env(),
        );
        settings.frame(0.0);
        assert!(settings.find("update-download").is_none());
        click(&mut settings, "update-website");
        assert_eq!(settings.take_url_requests(), [RELEASES_URL]);
    }
}
