use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use gpui_kit::{
    base::Disableable,
    component::{
        ActiveTheme, Selectable,
        button::{Button, ButtonVariants},
    },
    prelude::*,
    *,
};
use sniplet_core::{Document, Project};
use sniplet_platform::{
    SettingsStore,
    updates::{self as platform, BuildIdentity, DownloadedUpdate, ReleaseChannel, UpdateRelease},
};

use crate::{
    editor::{Editor, tool_icon},
    settings_window::{card, divider, note, row, section},
};

pub(crate) const RESTORE_ARG: &str = "--restore-update";
const RELEASES_URL: &str = "https://github.com/m4tta/sniplet/releases";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(60 * 60);
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(10);

/// A release found by a check.
pub(crate) struct Release {
    version: String,
    build_number: u64,
    channel: ReleaseChannel,
    size: u64,
    /// Absent for test doubles, which cannot build a signed platform release.
    package: Option<UpdateRelease>,
}

impl From<UpdateRelease> for Release {
    fn from(package: UpdateRelease) -> Self {
        Self {
            version: package.version.clone(),
            build_number: package.build_number,
            channel: package.channel,
            size: package.size,
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
    fn relaunch(&self, restore: bool) -> anyhow::Result<()>;
}

/// A downloaded update. Dropping it removes the staged files.
pub(crate) trait Staged: Send {
    fn install(self: Box<Self>) -> anyhow::Result<()>;
}

impl Staged for DownloadedUpdate {
    fn install(self: Box<Self>) -> anyhow::Result<()> {
        platform::install(*self)
    }
}

struct Platform {
    executable: Option<PathBuf>,
}

impl Backend for Platform {
    fn check(
        &self,
        channel: ReleaseChannel,
        current: &BuildIdentity,
    ) -> anyhow::Result<Option<Release>> {
        Ok(platform::check(channel, current)?.map(Release::from))
    }

    fn download(&self, release: &Release) -> anyhow::Result<Box<dyn Staged>> {
        let package = release
            .package
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("This release cannot be downloaded"))?;
        Ok(Box::new(platform::download(package)?))
    }

    fn relaunch(&self, restore: bool) -> anyhow::Result<()> {
        // The installer may move the running file aside, so prefer the path from startup.
        let executable = match &self.executable {
            Some(path) => path.clone(),
            None => std::env::current_exe()?,
        };
        let mut args = vec![format!("--update-parent={}", std::process::id())];
        if restore {
            args.push(RESTORE_ARG.to_owned());
        }
        #[cfg(target_os = "macos")]
        if let Some(bundle) = executable
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        {
            std::process::Command::new("/usr/bin/open")
                .arg("-n")
                .arg(bundle)
                .arg("--args")
                .args(&args)
                .spawn()?;
            return Ok(());
        }
        std::process::Command::new(executable).args(&args).spawn()?;
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) enum Phase {
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
    /// The new version is on disk, but the new process did not start.
    RestartFailed(Arc<Release>, String),
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
        Self::with_backend(Arc::new(Platform {
            executable: std::env::current_exe().ok(),
        }))
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

    fn busy(&self) -> bool {
        matches!(self.phase, Phase::Downloading(_) | Phase::Installing(_))
    }

    pub(crate) fn installing(&self) -> bool {
        matches!(self.phase, Phase::Installing(_))
    }

    fn transferring(&self) -> bool {
        self.busy() || matches!(self.phase, Phase::Ready(_))
    }

    fn automatic_check_due(&self) -> bool {
        !self.check_running
            && !self.transferring()
            && self
                .next_automatic
                .is_none_or(|next| Instant::now() >= next)
    }
}

enum InstallError {
    /// Nothing changed on disk; the staged update can be installed again.
    Prepare(anyhow::Error, Option<Box<dyn Staged>>),
    Install(anyhow::Error),
    Restart(anyhow::Error),
}

/// Check at startup and once a day. Debug and test builds only check when asked.
pub(crate) fn schedule(cx: &mut Context<Editor>) {
    if cfg!(any(test, debug_assertions)) {
        return;
    }
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(FIRST_CHECK_DELAY).await;
        loop {
            let alive = this.update(cx, |editor, cx| {
                if editor.settings.check_for_updates && editor.updates.automatic_check_due() {
                    editor.check_for_updates(true, cx);
                }
            });
            if alive.is_err() {
                return;
            }
            cx.background_executor().timer(RETRY_INTERVAL).await;
        }
    })
    .detach();
}

impl Editor {
    pub(crate) fn check_for_updates(&mut self, automatic: bool, cx: &mut Context<Self>) {
        if self.updates.transferring() {
            return;
        }
        self.updates.phase = Phase::Checking;
        cx.notify();
        if self.updates.check_running {
            self.updates.check_pending = Some(automatic);
            return;
        }
        self.updates.check_running = true;
        let generation = self.updates.generation;
        let channel = self.settings.update_channel;
        let current = self.updates.current.clone();
        let backend = self.updates.backend.clone();
        let task = cx
            .background_executor()
            .spawn(async move { backend.check(channel, &current) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_check(generation, automatic, result, cx)
            });
        })
        .detach();
    }

    fn finish_check(
        &mut self,
        generation: u64,
        automatic: bool,
        result: anyhow::Result<Option<Release>>,
        cx: &mut Context<Self>,
    ) {
        let updates = &mut self.updates;
        updates.check_running = false;
        let pending = updates.check_pending.take();
        if generation != updates.generation {
            if let Some(automatic) = pending {
                self.check_for_updates(automatic, cx);
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
        cx.notify();
        if automatic && matches!(self.updates.phase, Phase::Available(_)) {
            self.download_update(cx);
        }
    }

    pub(crate) fn download_update(&mut self, cx: &mut Context<Self>) {
        let release = match &self.updates.phase {
            Phase::Available(release)
            | Phase::DownloadFailed(release, _)
            | Phase::InstallFailed(release, _) => release.clone(),
            _ => return,
        };
        self.updates.phase = Phase::Downloading(release.clone());
        cx.notify();
        let generation = self.updates.generation;
        let backend = self.updates.backend.clone();
        let task = {
            let release = release.clone();
            cx.background_executor()
                .spawn(async move { backend.download(&release) })
        };
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.updates.generation != generation {
                    return;
                }
                match result {
                    Ok(staged) => {
                        this.updates.staged = Some(staged);
                        this.updates.phase = Phase::Ready(release);
                        this.updates.notice_dismissed = false;
                    }
                    Err(error) => {
                        this.updates.phase = Phase::DownloadFailed(release, error.to_string())
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Save settings and the open screenshot, install, then start the new version.
    pub(crate) fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        let (release, installed) = match &self.updates.phase {
            Phase::Ready(release) | Phase::InstallFailed(release, _) => (release.clone(), false),
            Phase::RestartFailed(release, _) => (release.clone(), true),
            _ => return,
        };
        let staged = self.updates.staged.take();
        if !installed && staged.is_none() {
            self.download_update(cx);
            return;
        }
        self.updates.phase = Phase::Installing(release.clone());
        cx.notify();
        let settings = self.settings.clone();
        let store = self.settings_store();
        let document = self.document.clone();
        let backend = self.updates.backend.clone();
        cx.spawn(async move |this, cx| {
            // Let the single settings writer finish before the final snapshot.
            while this
                .read_with(cx, |editor, _| editor.settings_save_running)
                .unwrap_or(false)
            {
                cx.background_executor()
                    .timer(Duration::from_millis(10))
                    .await;
            }
            let task = cx.background_executor().spawn(async move {
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
                if let Some(staged) = staged
                    && let Err(error) = staged.install()
                {
                    return Err(InstallError::Install(error));
                }
                backend
                    .relaunch(recovery.is_some())
                    .map_err(InstallError::Restart)?;
                Ok(())
            });
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.finish_install(release, result, cx));
        })
        .detach();
    }

    fn finish_install(
        &mut self,
        release: Arc<Release>,
        result: Result<(), InstallError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(()) => {
                self.updates.phase = Phase::Idle;
                cx.quit();
            }
            Err(InstallError::Prepare(error, staged)) => {
                let message = format!("Sniplet could not save your work first: {error}");
                self.updates.phase = if staged.is_some() {
                    Phase::InstallFailed(release, message)
                } else {
                    Phase::RestartFailed(release, message)
                };
                self.updates.staged = staged;
            }
            Err(InstallError::Install(error)) => {
                self.updates.phase = Phase::InstallFailed(release, error.to_string());
            }
            Err(InstallError::Restart(error)) => {
                self.updates.phase = Phase::RestartFailed(release, error.to_string());
            }
        }
        self.updates.notice_dismissed = false;
        cx.notify();
    }

    pub(crate) fn set_update_channel(&mut self, channel: ReleaseChannel, cx: &mut Context<Self>) {
        if self.settings.update_channel == channel || self.updates.busy() {
            return;
        }
        self.settings.update_channel = channel;
        self.persist_settings(cx);
        self.updates.generation += 1;
        self.updates.staged = None;
        self.updates.phase = Phase::Idle;
        self.updates.next_automatic = None;
        self.check_for_updates(false, cx);
    }

    pub(crate) fn dismiss_update_notice(&mut self, cx: &mut Context<Self>) {
        self.updates.notice_dismissed = true;
        cx.notify();
    }

    /// A notice over the canvas while an update is ready or failed to install.
    pub(crate) fn render_update_notice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.updates.notice_dismissed {
            return None;
        }
        let (title, description, icon, failed) = match &self.updates.phase {
            Phase::Ready(release) => (
                format!("Sniplet {} is ready", release_label(release)),
                "Sniplet restarts. Your screenshot and edits come back.".to_owned(),
                "download",
                false,
            ),
            Phase::Installing(release) => (
                format!("Installing Sniplet {}…", release_label(release)),
                "Sniplet restarts when it is done.".to_owned(),
                "download",
                false,
            ),
            Phase::InstallFailed(_, error) | Phase::RestartFailed(_, error) => (
                failure_title(&self.updates.phase).to_owned(),
                error.clone(),
                "circle-alert",
                true,
            ),
            _ => return None,
        };
        let installing = matches!(self.updates.phase, Phase::Installing(_));
        let notice = div()
            .id("update-notice")
            .occlude()
            .absolute()
            .right(px(12.0))
            .bottom(px(12.0))
            .w(px(440.0))
            .p_3()
            .flex()
            .items_center()
            .gap_3()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .border_1()
            .border_color(cx.theme().border)
            .rounded_lg()
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(status_icon(icon, failed, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(note(description, cx)),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .flex_shrink_0()
                    .child(
                        Button::new("update-later")
                            .ghost()
                            .label("Later")
                            .disabled(installing)
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss_update_notice(cx))),
                    )
                    .child(
                        Button::new("update-restart")
                            .primary()
                            .label(if failed {
                                "Try again"
                            } else {
                                "Restart & update"
                            })
                            .loading(installing)
                            .disabled(installing)
                            .on_click(cx.listener(|this, _, _, cx| this.restart_to_update(cx))),
                    ),
            );
        #[cfg(feature = "ui-tests")]
        let notice = {
            use gpui_kit::test::TestSupportExt;
            notice.test_support()
        };
        Some(notice.into_any_element())
    }

    /// After Later, the app menu shows a dot and this row label until the update is used.
    pub(crate) fn waiting_update_label(&self) -> Option<String> {
        if !self.updates.notice_dismissed {
            return None;
        }
        match &self.updates.phase {
            Phase::Ready(release)
            | Phase::InstallFailed(release, _)
            | Phase::RestartFailed(release, _) => {
                Some(format!("Update to Sniplet {}", release_label(release)))
            }
            _ => None,
        }
    }

    pub(crate) fn show_update_notice(&mut self, cx: &mut Context<Self>) {
        self.panel = None;
        self.updates.notice_dismissed = false;
        cx.notify();
    }
}

/// Prevent edits after the recovery snapshot until replacement has finished.
pub(crate) fn installing_screen(cx: &App) -> AnyElement {
    div()
        .id("update-installing")
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .child("Installing update…")
        .child(note(
            "Sniplet saves your work and opens again after the update.",
            cx,
        ))
        .into_any_element()
}

pub(crate) fn settings_section(editor: &Entity<Editor>, cx: &App) -> AnyElement {
    let editor_state = editor.read(cx);
    let updates = &editor_state.updates;
    let channel = editor_state.settings.update_channel;
    let automatic = editor_state.settings.check_for_updates;
    let current = format!(
        "You have {}. Checking {}.",
        updates.current.version,
        channel.label()
    );
    let last_check = updates
        .last_check
        .map(|time| format!(" Last check: {}.", elapsed_label(time.elapsed())))
        .unwrap_or_default();
    let phase = updates.phase.clone();
    let checking = matches!(phase, Phase::Checking);
    let transferring = updates.transferring();
    let busy = updates.busy();
    let (title, description, icon, failed): (String, String, &str, bool) = match &phase {
        Phase::Idle => (
            format!("Sniplet {}", build_label(&updates.current, channel)),
            format!("{current}{last_check}"),
            "refresh-cw",
            false,
        ),
        Phase::Checking => (
            "Checking for updates…".into(),
            current.clone(),
            "refresh-cw",
            false,
        ),
        Phase::UpToDate => (
            "No newer update is available".into(),
            format!("{current}{last_check}"),
            "circle-check",
            false,
        ),
        Phase::Available(release) => (
            format!("Sniplet {} is available", release_label(release)),
            format!("{current} Size: {}.{last_check}", size_label(release.size)),
            "download",
            false,
        ),
        Phase::Downloading(release) => (
            format!("Downloading {}…", release_label(release)),
            format!(
                "Size: {}. You can use Sniplet while you wait.",
                size_label(release.size)
            ),
            "download",
            false,
        ),
        Phase::Ready(release) => (
            format!("Sniplet {} is ready", release_label(release)),
            "Sniplet restarts. Your screenshot and edits come back.".into(),
            "circle-check",
            false,
        ),
        Phase::Installing(release) => (
            format!("Installing {}…", release_label(release)),
            "Sniplet restarts when it is done.".into(),
            "download",
            false,
        ),
        Phase::CheckFailed(error) => (
            failure_title(&phase).into(),
            format!("Make sure that you are connected to the internet. Then try again. ({error})"),
            "circle-alert",
            true,
        ),
        Phase::DownloadFailed(_, error) => (
            failure_title(&phase).into(),
            format!("Check your internet connection. Then try again. ({error})"),
            "circle-alert",
            true,
        ),
        Phase::InstallFailed(_, error) => (
            failure_title(&phase).into(),
            format!("{current} Try again, or get the new version from the website. ({error})"),
            "circle-alert",
            true,
        ),
        Phase::RestartFailed(_, error) => (
            failure_title(&phase).into(),
            format!("Close Sniplet and open it again to use the new version. ({error})"),
            "circle-alert",
            true,
        ),
    };
    let action =
        |id: &'static str, label: &'static str, run: fn(&mut Editor, &mut Context<Editor>)| {
            let editor = editor.clone();
            Button::new(id)
                .label(label)
                .on_click(move |_, _, cx| editor.update(cx, run))
        };
    let check: fn(&mut Editor, &mut Context<Editor>) =
        |editor, cx| editor.check_for_updates(false, cx);
    let download: fn(&mut Editor, &mut Context<Editor>) = Editor::download_update;
    let install: fn(&mut Editor, &mut Context<Editor>) = Editor::restart_to_update;
    let mut controls = div().flex().items_center().gap_2().flex_shrink_0();
    if !transferring {
        controls = controls.child(
            action("update-check", "Check now", check)
                .ghost()
                .icon(tool_icon("refresh-cw"))
                .loading(checking)
                .disabled(checking),
        );
    }
    controls = match &phase {
        Phase::Available(_) => controls.child(
            action("update-download", "Download", download)
                .primary()
                .icon(tool_icon("download")),
        ),
        Phase::Downloading(_) => controls.child(
            action("update-download", "Download", download)
                .primary()
                .loading(true)
                .disabled(true),
        ),
        Phase::DownloadFailed(..) => {
            controls.child(action("update-download-retry", "Try again", download))
        }
        Phase::Ready(_) | Phase::Installing(_) => controls.child(
            action("update-install", "Restart & update", install)
                .primary()
                .loading(matches!(phase, Phase::Installing(_)))
                .disabled(matches!(phase, Phase::Installing(_))),
        ),
        Phase::InstallFailed(..) => controls
            .child(
                Button::new("update-website")
                    .ghost()
                    .icon(tool_icon("globe"))
                    .label("Open website")
                    .on_click(|_, _, cx| cx.open_url(RELEASES_URL)),
            )
            .child(action("update-install-retry", "Try again", install)),
        Phase::RestartFailed(..) => {
            controls.child(action("update-install-retry", "Restart", install))
        }
        _ => controls,
    };
    let channels = div().flex().gap_1().children(
        [
            ("update-channel-stable", ReleaseChannel::Stable),
            ("update-channel-nightly", ReleaseChannel::Nightly),
        ]
        .map(|(id, choice)| {
            let editor = editor.clone();
            Button::new(id)
                .ghost()
                .label(choice.label())
                .selected(channel == choice)
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    editor.update(cx, |editor, cx| editor.set_update_channel(choice, cx))
                })
        }),
    );
    let toggle = {
        let editor = editor.clone();
        gpui_kit::component::switch::Switch::new("check-for-updates")
            .checked(automatic)
            .accessibility_label("Check for updates automatically")
            .on_click(move |value: &bool, _, cx| {
                let value = *value;
                editor.update(cx, |editor, cx| {
                    editor.settings.check_for_updates = value;
                    editor.persist_settings(cx);
                })
            })
    };
    let status = div()
        .id("update-status")
        .test_support()
        .p_4()
        .flex()
        .items_center()
        .gap_3()
        .child(status_icon(icon, failed, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(note(description, cx)),
        )
        .child(controls);
    section(
        "Updates",
        card(cx)
            .child(status)
            .child(divider(cx))
            .child(row(
                "Update channel",
                "Stable is tested. Nightly can have bugs. Switching channels never installs an older version.",
                channels,
                cx,
            ))
            .child(divider(cx))
            .child(row(
                "Check for updates automatically",
                "One time each day. Sniplet asks before it installs.",
                toggle,
                cx,
            )),
        cx,
    )
    .into_any_element()
}

fn failure_title(phase: &Phase) -> &'static str {
    match phase {
        Phase::CheckFailed(_) => "Sniplet could not check for updates",
        Phase::DownloadFailed(..) => "Sniplet could not download the update",
        Phase::RestartFailed(..) => "Sniplet is updated, but it could not restart",
        _ => "The update did not install",
    }
}

fn status_icon(name: &'static str, failed: bool, cx: &App) -> Div {
    let color: Hsla = if failed {
        cx.theme().danger
    } else if name == "circle-check" {
        cx.theme().success
    } else {
        rgb(0x0a84ff).into()
    };
    div()
        .size(px(30.0))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(8.0))
        .bg(color.opacity(0.16))
        .text_color(color)
        .child(tool_icon(name).size(px(16.0)))
}

fn build_label(build: &BuildIdentity, channel: ReleaseChannel) -> String {
    match channel {
        ReleaseChannel::Stable => build.version.clone(),
        ReleaseChannel::Nightly => format!("{} (build {})", build.version, build.build_number),
    }
}

fn release_label(release: &Release) -> String {
    match release.channel {
        ReleaseChannel::Stable => release.version.clone(),
        ReleaseChannel::Nightly => format!(
            "{} Nightly (build {})",
            release.version, release.build_number
        ),
    }
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

fn remove_recovery(project: &Path) {
    let _ = std::fs::remove_file(project.with_extension("png"));
    let _ = std::fs::remove_file(project);
}

/// Open the screenshot saved before an update. Clear it after the app window opens.
pub(crate) fn restore(store: &SettingsStore) -> anyhow::Result<Document> {
    let project = recovery_path(store);
    let document = Project::load(&project)?.open_document()?;
    Ok(document)
}

pub(crate) fn clear_recovery(store: &SettingsStore) {
    remove_recovery(&recovery_path(store));
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::settings_window::Page;
    use gpui_kit::test::TestWindowExt;
    use std::prelude::v1::test;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct Fake {
        log: Mutex<Vec<String>>,
        fail_check: AtomicBool,
        fail_install: AtomicBool,
        fail_restart: AtomicBool,
    }

    struct FakeStaged(Arc<Fake>, String);

    impl Staged for FakeStaged {
        fn install(self: Box<Self>) -> anyhow::Result<()> {
            self.0
                .log
                .lock()
                .unwrap()
                .push(format!("install {}", self.1));
            if self.0.fail_install.load(Ordering::SeqCst) {
                anyhow::bail!("disk is full");
            }
            Ok(())
        }
    }

    struct FakeBackend(Arc<Fake>);

    impl Backend for FakeBackend {
        fn check(
            &self,
            channel: ReleaseChannel,
            _: &BuildIdentity,
        ) -> anyhow::Result<Option<Release>> {
            self.0
                .log
                .lock()
                .unwrap()
                .push(format!("check {}", channel.label()));
            if self.0.fail_check.load(Ordering::SeqCst) {
                anyhow::bail!("offline");
            }
            Ok(Some(Release {
                version: match channel {
                    ReleaseChannel::Stable => "9.0.0".into(),
                    ReleaseChannel::Nightly => "9.1.0".into(),
                },
                build_number: 42,
                channel,
                size: 6_400_000,
                package: None,
            }))
        }

        fn download(&self, release: &Release) -> anyhow::Result<Box<dyn Staged>> {
            self.0
                .log
                .lock()
                .unwrap()
                .push(format!("download {}", release.version));
            Ok(Box::new(FakeStaged(
                self.0.clone(),
                release.version.clone(),
            )))
        }

        fn relaunch(&self, restore: bool) -> anyhow::Result<()> {
            self.0
                .log
                .lock()
                .unwrap()
                .push(format!("relaunch {restore}"));
            if self.0.fail_restart.load(Ordering::SeqCst) {
                anyhow::bail!("Could not start the new process");
            }
            Ok(())
        }
    }

    fn fixture(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Editor>, Arc<Fake>, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "sniplet-updates-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("settings.json");
        let fake = Arc::new(Fake::default());
        let (handle, editor) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    let mut editor = Editor::new(
                        Some(Document::new(image::RgbaImage::from_pixel(
                            80,
                            60,
                            image::Rgba([20, 40, 60, 255]),
                        ))),
                        Default::default(),
                        window,
                        cx,
                    );
                    editor.settings_path = Some(path.clone());
                    editor.updates = Updates::with_backend(Arc::new(FakeBackend(fake.clone())));
                    editor
                })
            })
            .unwrap()
        });
        (handle, editor, fake, path)
    }

    fn open_settings(
        cx: &mut TestAppContext,
        handle: AnyWindowHandle,
        editor: &Entity<Editor>,
    ) -> AnyWindowHandle {
        handle
            .update(cx, |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.open_settings(Page::General, window, cx)
                })
            })
            .unwrap();
        cx.run_until_parked();
        let settings = cx.read(|cx| editor.read(cx).settings_window.as_ref().unwrap().0);
        settings
            .update(cx, |_, window, cx| {
                window.resize(size(px(900.0), px(710.0)));
                window.scroll(
                    ("settings-content", Page::General as usize),
                    ScrollDelta::Pixels(point(px(0.0), px(-3000.0))),
                    cx,
                );
            })
            .unwrap();
        settings
    }

    fn log(fake: &Fake) -> Vec<String> {
        std::mem::take(&mut *fake.log.lock().unwrap())
    }

    #[gpui_kit::test]
    fn a_channel_change_drops_the_old_result_and_locks_while_downloading(cx: &mut TestAppContext) {
        let (handle, editor, fake, path) = fixture(cx);
        let settings = open_settings(cx, handle, &editor);
        fake.fail_check.store(true, Ordering::SeqCst);
        settings
            .update(cx, |_, window, cx| window.click("update-check", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(matches!(
            cx.read(|cx| editor.read(cx).updates.phase.clone()),
            Phase::CheckFailed(_)
        ));
        fake.fail_check.store(false, Ordering::SeqCst);
        settings
            .update(cx, |_, window, cx| {
                window.click("update-check", cx);
                window.click("update-channel-nightly", cx);
                assert!(matches!(editor.read(cx).updates.phase, Phase::Checking));
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            log(&fake),
            ["check Stable", "check Stable", "check Nightly"]
        );
        let Phase::Available(release) = cx.read(|cx| editor.read(cx).updates.phase.clone()) else {
            panic!("the Nightly check must show its release");
        };
        assert_eq!(release.channel, ReleaseChannel::Nightly);
        settings
            .update(cx, |_, window, cx| {
                window.click("update-download", cx);
                window.click("update-channel-stable", cx);
                assert_eq!(
                    editor.read(cx).settings.update_channel,
                    ReleaseChannel::Nightly
                );
                assert!(window.try_find("update-check").is_none());
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(log(&fake), ["download 9.1.0"]);
        assert!(cx.read(|cx| matches!(editor.read(cx).updates.phase, Phase::Ready(_))));
        assert_eq!(
            SettingsStore::at(&path).load().unwrap().update_channel,
            ReleaseChannel::Nightly
        );
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.find("update-notice").visible());
            })
            .unwrap();
        settings
            .update(cx, |_, window, cx| {
                window.click("update-channel-stable", cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(log(&fake), ["check Stable"]);
        let Phase::Available(release) = cx.read(|cx| editor.read(cx).updates.phase.clone()) else {
            panic!("a channel change must replace the ready Nightly update");
        };
        assert_eq!(release.channel, ReleaseChannel::Stable);
        handle
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                assert!(window.try_find("update-notice").is_none());
            })
            .unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[gpui_kit::test]
    fn later_leaves_a_menu_dot_and_the_menu_brings_the_notice_back(cx: &mut TestAppContext) {
        let (handle, editor, _, path) = fixture(cx);
        handle
            .update(cx, |_, _, cx| {
                editor.update(cx, |editor, cx| editor.check_for_updates(true, cx))
            })
            .unwrap();
        cx.run_until_parked();
        assert!(cx.read(|cx| matches!(editor.read(cx).updates.phase, Phase::Ready(_))));
        handle
            .update(cx, |_, window, cx| {
                window.click("update-later", cx);
                assert!(window.try_find("update-notice").is_none());
                assert!(window.find("update-dot").visible());
                window.click("app-menu", cx);
                window.click("menu-update", cx);
                window.render_frame(cx);
                assert!(window.find("update-notice").visible());
                assert!(window.try_find("update-dot").is_none());
            })
            .unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[gpui_kit::test]
    fn a_failed_install_keeps_the_document_and_a_retry_restarts_with_recovery(
        cx: &mut TestAppContext,
    ) {
        let (handle, editor, fake, path) = fixture(cx);
        let original = cx.read(|cx| editor.read(cx).export_pixels().unwrap());
        handle
            .update(cx, |_, _, cx| {
                editor.update(cx, |editor, cx| {
                    editor.settings.check_for_updates = false;
                    editor.check_for_updates(true, cx);
                })
            })
            .unwrap();
        cx.run_until_parked();
        log(&fake);
        fake.fail_install.store(true, Ordering::SeqCst);
        handle
            .update(cx, |_, window, cx| window.click("update-restart", cx))
            .unwrap();
        cx.run_until_parked();
        let store = SettingsStore::at(&path);
        assert_eq!(log(&fake), ["install 9.0.0"]);
        assert!(matches!(
            cx.read(|cx| editor.read(cx).updates.phase.clone()),
            Phase::InstallFailed(..)
        ));
        assert_eq!(
            cx.read(|cx| editor.read(cx).export_pixels().unwrap()),
            original
        );
        assert!(!store.load().unwrap().check_for_updates);
        assert!(recovery_path(&store).exists());

        fake.fail_install.store(false, Ordering::SeqCst);
        handle
            .update(cx, |_, window, cx| window.click("update-restart", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(log(&fake), ["download 9.0.0"]);
        fake.fail_restart.store(true, Ordering::SeqCst);
        handle
            .update(cx, |_, window, cx| window.click("update-restart", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(log(&fake), ["install 9.0.0", "relaunch true"]);
        assert!(recovery_path(&store).exists());
        assert!(cx.read(|cx| matches!(editor.read(cx).updates.phase, Phase::RestartFailed(..))));
        // Edits made after a failed restart must replace the earlier snapshot.
        handle
            .update(cx, |_, _, cx| {
                editor.update(cx, |editor, cx| {
                    editor
                        .document
                        .as_mut()
                        .unwrap()
                        .set_crop(sniplet_core::ImageRect::new(0.0, 0.0, 30.0, 20.0));
                    cx.notify();
                });
            })
            .unwrap();
        let latest = cx.read(|cx| editor.read(cx).export_pixels().unwrap());
        assert_ne!(latest.dimensions(), original.dimensions());
        fake.fail_restart.store(false, Ordering::SeqCst);
        handle
            .update(cx, |_, window, cx| window.click("update-restart", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(log(&fake), ["relaunch true"]);
        let restored = restore(&store).unwrap();
        assert_eq!(
            restored.render(&crate::editor::render_options()).unwrap(),
            latest
        );
        clear_recovery(&store);
        assert!(!recovery_path(&store).exists());
        assert!(!recovery_path(&store).with_extension("png").exists());
        assert!(restore(&store).is_err());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn update_icons_are_bundled() {
        for icon in [
            "download",
            "refresh-cw",
            "circle-check",
            "circle-alert",
            "globe",
        ] {
            assert!(
                AssetSource::load(&gpui_kit::assets::AllAssets, &format!("icons/{icon}.svg"))
                    .unwrap()
                    .is_some(),
                "{icon}"
            );
        }
    }
}
