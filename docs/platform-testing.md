# Sniplet platform testing

Sniplet needs two kinds of evidence. The repeatable suite proves the Rust workspace, image model, platform adapters, and headless GPUI tests build and pass. A real desktop session proves capture, global input, tray, clipboard, pinning, and mixed-DPI behavior. A CI runner cannot substitute for the second category.



## Repeatable checks

Use Rust 1.92 or newer, with `rustfmt` and Clippy installed. GPUI Kit 0.7's current locked dependency graph establishes that minimum. Linux builders also need the packages installed by the CI workflow; the verification scripts intentionally do not change the host.

Run the full suite from the repository root:

```powershell
./scripts/verify.ps1 -Suite all
./scripts/verify.ps1 -Suite all -Native
```

```sh
bash scripts/verify.sh all
bash scripts/verify.sh all --native
```

Both scripts accept four suites:

| Suite | Purpose |
| --- | --- |
| `core` | Format, check, test, and lint `sniplet-core` without a windowing or capture backend. |
| `workspace` | Format, check, test, and lint every workspace package with default features. |
| `ui` | Format, check, test, and lint `sniplet-app` with the `ui-tests` feature. |
| `all` | Run the complete workspace with all features, a separately visible core test command, default-feature workspace tests, UI-feature tests, and warnings-as-errors Clippy. |

The scripts use `Cargo.lock` and only create ordinary Cargo build artifacts. They do not install packages, modify user settings, register hotkeys, request permissions, or launch a desktop capture.

Adding `-Native` or `--native` changes that final sentence: after the selected automated suite passes, the script builds `sniplet-app`, opens `--demo --smoke --normal-window` with a 20-second outer limit, then runs `--self-test artifacts/self-test` with a 60-second limit. Smoke should close itself after roughly three seconds. Self-test performs a real primary-display capture and writes `native-capture.png` plus `editor-demo.png`; the OS may request screen-capture permission. These checks prove only the current host/session and never run in ordinary CI.

CI runs `core` independently on Linux and runs `all` on Windows, macOS, and Linux. Linux receives the union of the native dependencies documented by GPUI Kit and xcap. Passing CI means that all packages compile and their automated tests pass on those runner images. It does not prove that a runner had an interactive desktop, a working GPU session, permission to capture, a global hotkey service, or a system tray.

Earlier dated records retain the measurements and process IDs observed at that time. Their package paths now use Sniplet because the distribution trees have been renamed and refreshed; the older hashes do not describe the current contents. Historical capture filenames and the active checkout/build-cache paths are preserved.

## Observed Linux run: WSLg

The following is a development record, not release sign-off, because it was collected from an uncommitted working tree on October 2, 2026.

| Field | Evidence |
| --- | --- |
| Host | `codex-yarr-dev`, Ubuntu 24.04.5 LTS under WSL2 kernel `6.18.33.2-microsoft-standard-WSL2` |
| Rust | `rustc 1.97.1`, `cargo 1.97.1`, host `x86_64-unknown-linux-gnu` |
| Source / build output | `/mnt/c/code/clippy` / `/tmp/clippy-linux-target`; two Cargo jobs maximum |
| Desktop bridge | WSLg exposed X11 display `:0` and Wayland socket `wayland-0` |
| Build dependencies | The CI Linux package set, plus `libgbm-dev`, was sufficient to link GPUI |
| Format | `cargo fmt --all -- --check` passed |
| Compile | Final workspace all-target/all-feature `cargo check` passed in 1.74 seconds with the warm dependency cache. A complete all-feature build passed in 15.87 seconds immediately before the final app-only error-path patch; the final default-feature app rebuilt in 4.22 seconds |
| Tests | `sniplet-core` 32/32; default-feature `sniplet-app` 10/10; `sniplet-platform` 20/20; `sniplet-app --features ui-tests` 26/26. The all-feature app result includes its ten ordinary tests, for 78 distinct tests across the final core/platform/all-feature app suites |
| Strict Clippy | Workspace all-target/all-feature Clippy passed with warnings denied |
| Window smoke | The final copied debug package ran `--demo --smoke --normal-window` through WSLg Wayland and forced X11; both exited 0 within the 20-second bound |
| Native self-test | Blocked: `libwayshot-xcap` could not bind the WSLg compositor's required `ZxdgOutputManagerV1` version. Sniplet converted the backend panic into an error and exited 1 without artifacts |
| Forced X11 | With `WAYLAND_DISPLAY` unset only for the process, demo smoke exited 0. Native self-test was blocked by WSLg's X server: XCB `GetImage` returned `X(Match)` with error code 8 and no artifacts |
| Package | Current default-feature debug Linux portable tree created at `dist-linux-test/sniplet-x86_64-unknown-linux-gnu-debug`; its executable is a 206,740,760-byte ELF64 x86-64 position-independent executable with SHA-256 `6631149BD2A043E38624123F6FD86097E08DB35413F5E956B404782DFAF7213B`, and the bundle includes the desktop entry, README, licenses, and documentation |

The Wayland smoke log also contained EGL/Zink fallback warnings and the panic hook's `UnsupportedVersion` text before the catch boundary returned control. Forced X11 emitted a DRI3 software-rendering warning but no Wayland panic. Exit 0 proves the bounded GPUI demo lifecycle through each WSLg presentation route; it is not visual review evidence. Both native self-test results are clear WSLg backend limitations, not capture passes. WSLg does not substitute for a physical Linux X11 desktop, another Wayland compositor, global hotkeys, tray behavior, pin/topmost behavior, or mixed-DPI hardware. The recurring `Failed to mount E:\\` WSL startup warning was independent of the repository and appeared before every command.

## Observed Windows run

The same October 2, 2026 development session produced partial native evidence on Windows 11 Pro build 26200. This also came from an uncommitted working tree, so the status remains in progress.

| Check | Evidence |
| --- | --- |
| Core | Final 32-test suite passed |
| Platform | 20 tests passed; one environment-gated OCR test was ignored, and native OCR had passed in an earlier direct probe |
| GPUI interaction | Final all-feature app suite passed 26 tests, including ordinary tests and headless pointer/keyboard editor paths; with 32 core and 20 platform tests, the final Windows development suites cover 78 distinct tests |
| Strict Clippy | Workspace all-target Clippy passed with warnings denied for both default features and all features on the final production code |
| Process smoke | Debug app and packaged release both linked with an 8 MiB Windows stack and bounded smoke exited 0 |
| Self-test | The final native diagnostic passed: primary-display capture reported 2560×1440 physical pixels, then annotation/undo/redo/crop/PNG reload/font checks passed. A prior measured run wrote `artifacts/self-test/native-capture.png` (320×240, 18,882 bytes) and `editor-demo.png` (1000×660, 40,647 bytes) |
| Native editor input | The normal window accepted toolbar clicks after the toolbar occlusion fix, drew on the canvas, selected Text, typed `Sniplet native UI test`, committed it to the canvas, and removed it with Ctrl+Z |
| Release package | Final locked release build succeeded. `dist/sniplet-x86_64-pc-windows-msvc/Sniplet.exe` is 37,688,320 bytes with SHA-256 `47DED64528FAFC5FF01C0023D805AE53FEA30D47B3249FABF8D64636E4CD8835`. The copied executable passed both `--demo --smoke --normal-window` and `--self-test artifacts/release-self-test`; its bundle includes README, licenses, and documentation |

This record does not yet prove pixel-perfect appearance, mixed-DPI capture, tray lifecycle, global hotkeys, clipboard interoperability, or pin/topmost behavior on Windows. A later GUI inspection helper could not identify the foreground process, so no new screenshot review was recorded; the earlier real toolbar, drawing, and text-input exercise remains the native interaction evidence. Preserve the other items as separate release gates.

## October 3, 2026 capture-focus regression fix

This is a focused development record layered on the October 2 baseline above. It addresses two reported native workflows: Escape did not reliably cancel an area overlay when the overlay lacked keyboard focus, and the global Window shortcut could leave the window chooser inside a minimized editor.

This section records the first October 3 behavior. Later that day the cancellation requirement changed: cancelling capture must leave the editor minimized or hidden. The restoration behavior and native observations below remain useful historical evidence for Escape delivery and overlay removal, but they are superseded by the [cancellation visibility correction](#october-3-2026-cancellation-visibility-correction). The in-editor Window chooser is superseded by the [standalone capture picker](#october-3-2026-standalone-window-capture-picker).

The capture runtime now creates a capture-scoped Escape session before the delayed native capture starts, temporarily registers Escape with the global-hotkey service when available, attaches the eventual overlay to that session, activates the overlay, and removes the registration when the capture finishes or the overlay closes. Session IDs prevent cleanup from an older capture from clearing a newer registration. Cancellation removes the overlay and reactivates its owner. Invoking Window capture first cancels an existing capture session, then activates the editor and presents the chooser. The chooser filters Sniplet by process ID, retains eligible same-title windows from other processes, and renders a visible explanation for empty or failed enumeration.

| Check | October 3 evidence |
| --- | --- |
| Linux format | `cargo fmt --all -- --check` passed |
| Linux ordinary app suite | `sniplet-app` 11/11 passed, including `old_capture_cleanup_cannot_clear_a_new_escape_session` |
| Linux all-feature app suite | `sniplet-app --features ui-tests` 31/31 passed. Focused regressions were `active_escape_removes_overlay_while_owner_is_borrowed`, `removing_overlay_releases_its_escape_session`, `cancelling_overlay_keeps_the_existing_document`, `window_picker_activates_background_editor_and_escape_preserves_document`, and `empty_and_failed_window_enumeration_show_a_visible_explanation` |
| Linux strict lint | App all-target Clippy passed with warnings denied under both default and all features |
| Linux build/smoke | Fresh default-feature app build passed; bounded WSLg Wayland and forced-X11 `--demo --smoke --normal-window` runs both exited 0 |
| Windows automated gate | Format check and the 31/31 all-feature app suite passed. Workspace all-target Clippy passed with warnings denied under both default and all features |
| Windows capture-fix release/package | Release build passed in 4 minutes 7 seconds. `dist/capture-fix/sniplet-x86_64-pc-windows-msvc/Sniplet.exe` is 37,742,080 bytes with SHA-256 `B56AC4DD071647B8048A28868E3534B4B30BE0917E85FFA0B82EBAA38C633D3F`. Initially the separate path preserved the running October 2 app. After the user approved restarting, the same tested executable replaced the standard `dist/sniplet-x86_64-pc-windows-msvc/Sniplet.exe` path and launched as PID 36476 |
| Windows packaged smoke | The copied release package ran `--demo --smoke --normal-window` and exited 0. Logs are under `artifacts/capture-fix-self-test` |
| Windows packaged self-test | Exited 0 on the 2560×1440 `XG270QG` display; native capture, annotation, undo, redo, crop, PNG export/reload, and font rendering passed. It wrote `native-capture.png` (320×240, 15,890 bytes) and `editor-demo.png` (1000×660, 40,647 bytes) under `artifacts/capture-fix-self-test` |
| Windows native area cancellation | Passed two consecutive cycles on the restarted tested executable through Computer Use OS input. First, clicking **Capture an area** opened the dimmed overlay with its Escape hint; Escape sent to Chrome removed the overlay and restored Sniplet. Second, `Ctrl+Shift+1` sent to Chrome opened another area overlay; Escape again removed it and restored the editor. Before and after, the editor showed its empty welcome state, `0 × 0`, and `Ready`, with no new capture. Tool screenshots in this chat record the overlay and restored editor |

The automated tests prove session ownership, cleanup, editor activation, chooser filtering, visible error states, and document preservation in GPUI's test context. Packaged smoke proves window creation and shutdown; self-test proves the listed capture and image-processing path. The later native retry additionally verifies Windows area-capture hotkey delivery, Escape cancellation from another app, editor restoration, and a second capture/cancel cycle. The original still-running executable explained the repeated failure before the approved restart. Native cancellation during the startup delay or a held drag, an existing document/clipboard comparison, mixed-DPI capture, and minimized-editor Window-hotkey delivery remain separate open checks. WSLg still cannot exercise native capture in this environment.

## October 3, 2026 cancellation visibility correction

This correction supersedes only the cancellation restoration behavior recorded above. Starting capture minimizes the editor. Escape during the startup delay, Escape in the area overlay, a capture-scoped global Escape, and overlay disposal now cancel without activating the editor. A completed capture may activate the editor to show its result, and a capture error may activate it to show the failure. At this stage the Window chooser still activated the editor; the later [standalone capture picker](#october-3-2026-standalone-window-capture-picker) supersedes that behavior.

The runtime no longer keeps an editor-owner handle in its Escape registration. Global cancellation removes the attached overlay while preserving whichever other application window is active. Local overlay cancellation and release remove the overlay without restoring the editor. Successful area selection still activates the editor after loading the captured pixels.

| Check | October 3 correction evidence |
| --- | --- |
| Linux environment | `codex-yarr-dev`, source `/mnt/c/code/clippy`, target `/tmp/clippy-linux-target`, two Cargo jobs |
| Linux format | `cargo fmt --all -- --check` passed |
| Linux ordinary app suite | `sniplet-app` 11/11 passed |
| Linux all-feature app suite | `sniplet-app --features ui-tests` 31/31 passed. `active_escape_removes_overlay_without_activating_editor` preserved a separate active background window; `cancelling_overlay_keeps_the_existing_document` and `removing_overlay_releases_its_escape_session` left the editor inactive; `area_overlay_drag_produces_exact_source_pixels` retained successful editor activation |
| Linux strict lint | App all-target Clippy passed with warnings denied under both default and all features |
| GPUI test limit | GPUI's test platform does not implement minimize-state observation. The headless regressions prove that cancellation does not activate the editor; the native Windows result below supplies the minimized-state evidence |
| Windows automated gate | Format check, the 31/31 all-feature app suite, and app all-target Clippy with warnings denied under both default and all features passed |
| Windows release/package | Locked release build passed in 3 minutes 7 seconds. The fresh staged package at `dist/cancel-hidden/sniplet-x86_64-pc-windows-msvc/Sniplet.exe` is 37,740,544 bytes with SHA-256 `49AA991ED4008EC6FEF918253F4A77CCAAF872786B9F13E70192847FFF73F8D5`. The standard package and the prior `dist/capture-fix` package were updated to the same verified executable |
| Windows packaged smoke | The staged executable ran `--demo --smoke --normal-window` and exited 0 |
| Windows native area cancellation | Passed twice through native OS input. Button-started capture displayed the dimmed area overlay and Escape hint; Escape sent from Chrome removed the overlay, left Chrome visible, and did not show Sniplet. An editor window-state probe reported `window is minimized`. `Ctrl+Shift+1` from Chrome opened a second overlay; Escape removed it with the same minimized-editor result. The current app was left minimized as PID 10996. Tool screenshots in this chat record the visible states; the test harness did not save separate screenshot files |

The Linux commands emitted the already-observed WSL startup warning about failing to mount `E:\`, before Cargo began. It did not affect the checks. No new WSLg desktop smoke was needed for this state-transition-only correction, and WSLg still cannot supply native capture evidence in this environment. The native Windows exercise proves that ordinary and global-shortcut area cancellation leave the editor minimized. Existing-document and clipboard comparison, held-drag cancellation, startup-delay cancellation, and the Window picker remain separate checks.

## October 3, 2026 arrow editing update



The curve and its tangent-oriented head use the same raster renderer for preview, clipboard, and file export. Old straight-arrow projects load without a bend field; newly curved arrows preserve their bend in editable project JSON. Undo groups a whole control/shaft drag, and Escape restores an unfinished edit.

The repeatable visual fixture is generated with `cargo run -p sniplet-core --example arrow_gallery -- artifacts/arrow-update/arrow-gallery.png`. It contains straight arrows, both bend directions, steep diagonals, and a 300×140 reference-like curve. Visual review corrected the shaft termination so it stays beneath the head and does not protrude beyond the pointed tip. A flat-tail assertion checks that rendering does not extend behind the start, and a degenerate-arrow regression ensures an arrow with coincident endpoints cannot select itself from every canvas position.

| Check | Arrow update evidence |
| --- | --- |
| Windows automated gate | `scripts/verify.ps1 -Suite all` passed: format, workspace all-target/all-feature check/build, 39 core tests, 11 ordinary app tests, 20 platform tests, 34 UI-feature app tests, and strict workspace lint. The UI-feature count includes ordinary app tests. The native OCR-language-pack test remains environment-gated and ignored. |
| Linux automated gate | Final `scripts/verify.sh all` passed under `codex-yarr-dev` with `/tmp/clippy-linux-target`: format, workspace all-target/all-feature check/build, 39 core tests, 11 ordinary app tests, 20 platform tests, 34 UI-feature app tests, and strict workspace lint. WSL's existing `E:\\` mount warning did not affect Cargo. |
| Arrow interaction regressions | `arrow_key_draws_bold_arrow_with_three_handles_and_remembers_its_width`, `arrow_middle_and_endpoints_edit_independently_and_undo_as_one_drag`, and `curved_arrow_shaft_moves_all_handles_and_escape_restores_in_progress_bend` pass under GPUI real pointer/key event dispatch. Existing text-entry regression also preserves `A smooth arrow` as text. |
| Capture preservation | Before updating the running app, its unsaved 1150×648 capture and eight annotations were saved through the native project dialog to `artifacts/arrow-update/capture-before-arrow-update.clippy` with its `.source.png` companion. Native arrow testing saved a separate `native-arrow-proof.clippy`: its first eight annotations equal the original JSON exactly, and both companion source images have the same SHA-256. The ninth annotation preserves the new 10-pixel curved arrow and its edited points. |
| Windows native controls | OS input on the current debug build selected Arrow with A, drew a bold arrow with three handles, bent the middle control, repositioned both endpoints independently, moved the whole curve by its shaft, and reversed that whole move with one Ctrl+Z. The saved proof project confirmed all three coordinates, bend persistence, and the 10-pixel style. Native inspection also found clipped two-digit width labels; the palette controls were widened before packaging. |
| Final palette correction | The width buttons now reserve 40 pixels with explicit eight-pixel side padding. Native inspection verified readable 6, 10, and 14 choices with 10 selected. The Windows and Linux UI verification suites were repeated after this layout refinement. |
| Windows release package | Final locked release build passed in 3 minutes 53 seconds. `dist/arrow-tool/sniplet-x86_64-pc-windows-msvc/Sniplet.exe` is 36,361,216 bytes with SHA-256 `9C4BDC3759509B1BD6459ACF6BC3CE63CC802248533281EF9361824AD639276F`. The standard, prior capture-fix, and cancel-hidden portable directories are updated from this staged package. |
| Packaged diagnostics | The staged release passed bounded `--demo --smoke --normal-window` and `--self-test artifacts/arrow-update/release-self-test`. Self-test exercised native capture on the 2560×1440 XG270QG display, annotation, undo/redo, crop, PNG export/reload, and font rendering. Logs and generated diagnostic images are under `artifacts/arrow-update`. |
| Final capture snapshot | Immediately before the final restart, `current-capture.clippy` and its source PNG were saved through the native dialog. Its eight annotations equal the original capture JSON exactly, and its source PNG has the original SHA-256. This is the project reopened by the final release. |
| Installed release input | The standard portable executable reopened the preserved capture as PID 42836. Native input verified A selection, readable 6/10/14 width choices with 10 selected, a freshly drawn bold arrow with three controls, midpoint bending, and two separate undo steps restoring the original capture. It remains running with Arrow selected. |



## October 3, 2026 appearance update

Sniplet now follows the system appearance by default. Settings → Appearance offers System, Light, and Dark, applies the choice immediately, and saves it in the existing atomic settings store. System changes use the editor window's GPUI appearance notification; forced choices ignore those notifications. The window's appearance is used for Linux compatibility. Toolbar, canvas surround, floating panels, property palette, hints, status, text, borders, and built-in components use the toolkit's active theme. Capture pixels, annotation colors, and white editing handles retain their original colors.

| Check | Appearance update evidence |
| --- | --- |
| Windows automated gate | `scripts/verify.ps1 -Suite all` passed format, workspace all-target/all-feature check/build, 39 core tests, 21 platform tests, 13 ordinary app tests, 38 UI-feature app tests, and strict workspace lint. The UI count includes the ordinary app tests, for 98 distinct tests. One native OCR-language-pack test remains ignored. The first build attempt hit the running debug executable's Windows file lock; the app was closed and the complete gate then passed. |
| Linux automated gate | `scripts/verify.sh all` passed the same compile, test, format, and strict-lint gate under `codex-yarr-dev`, using `/tmp/clippy-linux-target`. WSL's existing `E:\\` mount warning did not affect Cargo. Logs for both platforms are in `artifacts/theme-update`. |
| Preference defaults and persistence | Existing settings without the new field load System. Tests cover lowercase JSON values, round-trip Dark, and all three choices saved through the Settings UI to an isolated temporary store. |
| Appearance regression | `system_tracks_all_system_appearances` covers both ordinary and vibrant Light/Dark values. `explicit_preferences_ignore_system_appearance` covers each forced preference against every OS appearance. `appearance_callback_tracks_system_and_ignores_explicit_preferences` installs the real subscription and exercises its callback boundary with a real GPUI test window. GPUI's platform appearance simulator is crate-private, so this test does not inject a native OS notification. |
| Settings interaction | `appearance_choices_apply_immediately_and_preserve_capture` clicks System/Light/Dark at the minimum 900×560 window size, verifies the active mode and saved preference, and compares exported pixels after each switch. Existing capture cancellation and arrow interaction regressions still pass. |
| Windows native appearance | With Windows `AppsUseLightTheme=0`, an existing-settings startup opened dark in System mode. Native clicks immediately switched the toolbar, settings panel, canvas, and status between Light and Dark. Light persisted through an actual process restart despite dark Windows settings. Selecting System immediately restored dark and persisted `theme: system`. Native inspection also verified readable arrow widths and color swatches in the dark property palette. |
| Linux presentation smoke | The final default-feature debug app completed bounded `--demo --smoke --normal-window` under WSLg Wayland with exit 0. The log contains the previously observed EGL/Zink fallback warnings and the caught `libwayshot-xcap` `UnsupportedVersion` panic from monitor enumeration. This proves the native GPUI lifecycle with System appearance, without qualifying desktop theme-change notifications or capture. |
| Capture preservation | The original unsaved 1150×648 capture was saved through the native project dialog before restart. `artifacts/theme-update/capture-before-theme-update.clippy` preserves all eight annotations exactly, and its companion source PNG retains SHA-256 `244B4610C7668A003EDFD8514E1FA346392B2A93B248ED4981C6CC50C24F33FB`. |
| Native capture comparison | After switching themes and restarting, the native Save Project dialog saved `capture-after-theme-switches.clippy`. Its eight annotations equal the pre-update project exactly, and its companion source PNG has the same SHA-256. |
| Windows release | Locked release compilation passed in 4 minutes 20 seconds. The 36,390,912-byte executable has SHA-256 `5BACC93206CD32E974EFCD3FF0ABBC82B9091C1D40FE2D2B176C006DF2D08E16`. It is staged under `dist/system-theme` for portable installation. |
| Packaged smoke | The staged release completed bounded `--demo --smoke --normal-window` with exit 0. Logs are under `artifacts/theme-update/release-smoke.*.log`. |
| Installed release | The standard portable executable reopened `capture-after-theme-switches.clippy` as PID 20680. Native inspection verified dark startup, the preserved capture, and System selected alongside the Light/Dark controls. Settings remain open for review. The standard, capture-fix, cancel-hidden, and arrow-tool directories and Windows ZIPs contain the same verified release. |

Native theme inspection was performed on Windows 11. No macOS desktop or physical Linux theme-change session was available; their native OS notification delivery remains a separate qualification check.

## October 3, 2026 standalone window-capture picker

Capture Window now hides the editor and opens a separate centered 520×460 picker. Each row shows the app name and window title. Clicking a row removes the picker before the existing capture flow starts, then opens the completed screenshot in the editor. Escape or the close button dismisses the picker without activating the editor. Repeated capture commands replace the previous picker. Empty and failed enumeration have visible explanations. Sniplet's own, minimized, untitled, and zero-size windows are omitted; on Windows, tool windows such as notification toasts and floating helper overlays are also omitted.

| Check | Standalone picker evidence |
| --- | --- |
| Windows automated gate | `scripts/verify.ps1 -Suite all` passed format, workspace all-target/all-feature check/build, 39 core tests, 21 platform tests, 13 ordinary app tests, 42 UI-feature app tests, and strict workspace lint. The UI count includes the ordinary app tests, for 102 distinct tests. One native OCR-language-pack test remains ignored. Default-feature app strict lint and the final native debug build also passed. |
| Linux automated gate | `scripts/verify.sh all` passed the same compile, test, format, and strict-lint gate under `codex-yarr-dev`, using `/tmp/clippy-linux-target`. Default-feature app strict lint also passed. Logs for both platforms are under `artifacts/window-picker-update`. WSL's existing `E:\\` mount warning did not affect Cargo. |
| Picker regressions | Six real GPUI interaction tests cover centered bounds and eligibility, opening while the Editor command is mutably updating, local/global Escape and close-button cancellation with document preservation, replacing an existing picker, loading/error/empty states, and removing the picker before invoking capture. |
| Native startup regression | The first Windows hotkey test caught a panic caused by reading the Editor from inside its command update. The redundant theme application was removed; the popup uses the already-active global theme. `picker_can_open_while_the_editor_command_is_updating` now exercises that borrow context. The corrected native process opens the picker without an error or panic. |
| Windows native picker | `Ctrl+Shift+3` sent to Chrome opened only the centered app/window list while the editor stayed minimized. Repeating the shortcut kept one picker. Escape sent from Chrome removed the replacement picker, and the native window probe confirmed the editor remained minimized. Clicking the close button also dismissed the picker and left the editor minimized. After the Windows tool-window filter, notification and Computer Use helper overlays were absent from the list. |
| Windows native selection | Clicking the Google Chrome row closed the picker and opened an 1818×1365 Chrome screenshot with status `Window captured`. Native inspection confirmed that the picker was absent from the captured image. The native Save Project dialog wrote `native-window-capture-proof.clippy` and its source PNG, SHA-256 `D00C1A205AAF3C9C38F241A919A782099F4D7A1A70272224C663B8A4D523D988`. |
| Capture preservation | Before restarting, the user's 1150×648 capture was saved through the native project dialog as `capture-before-picker-update.clippy`. All eight annotations equal the prior theme-update capture exactly, and the source PNG retains SHA-256 `244B4610C7668A003EDFD8514E1FA346392B2A93B248ED4981C6CC50C24F33FB`. The test screenshot was saved separately. |
| Windows release | Locked release compilation passed in 4 minutes 19 seconds. The 36,460,032-byte executable has SHA-256 `946F34734EA40C06666C201CBB0024FA7ECF520B30E93A706A7B418CCC554D81`. The fresh portable stage is `dist/window-picker/sniplet-x86_64-pc-windows-msvc`. |
| Packaged smoke | The staged release completed bounded `--demo --smoke --normal-window` with exit 0 and an empty stderr log. Logs are under `artifacts/window-picker-update/release-smoke.*.log`. |
| Installed release | The standard portable executable reopened the preserved original capture as PID 2824. Native inspection verified the 1150×648 annotated capture and dark System appearance. `Ctrl+Shift+3` from Chrome opened only the centered picker; Escape removed it, and the native probe confirmed the editor remained minimized. The release remains running with the original capture retained. Standard, capture-fix, cancel-hidden, arrow-tool, and system-theme portable directories contain the same executable. |

Native capture and focus evidence is from Windows 11. No macOS desktop or physical Linux capture session was available; those native qualification checks remain separate.

## October 4, 2026 arrow palette and hover window capture

This update replaces the October 3 list picker with desktop hover selection. The frozen desktop has a capture cursor and camera marker, a blue highlight follows the visible window under the pointer, and clicking removes the overlay before capturing that window. Foreground windows remain outside a background window's highlight. Escape and right-click dismiss capture while the editor stays hidden. The arrow palette adds a tapered size slider and Solid, Hand drawn, Thin, and Double ended styles; all retain the existing three editing controls and A shortcut.

| Check | Observed evidence |
| --- | --- |
| Windows automated gate | `scripts/verify.ps1 -Suite all` passed formatting, locked all-target/all-feature check/build, 46 core tests, 23 platform tests, and 46 UI-feature app tests, for 115 distinct tests. The ordinary app suite's 15 tests are included in the UI count. Strict workspace lint passed with warnings denied. One native Windows OCR-language-pack test remains ignored. The final default-feature app build also passed. |
| Linux automated gate | `scripts/verify.sh all` passed the same 115 distinct tests, build/check, formatting, and strict lint under WSL's `codex-yarr-dev` with `/tmp/clippy-linux-target`. The existing `E:\\` mount warning did not affect Cargo. Final logs for both hosts are under `artifacts/arrow-variants-update`. |
| Arrow regressions | Tests cover all four variants, unchanged Solid pixels, legacy project loading, export and hit testing, all three editing controls, retained geometry/style when changing variant, new-arrow preferences, multiple size movements grouped into one Undo, release outside the slider, Escape rollback, and the full 1–30 range in Light/Dark at the minimum editor size. `cargo run -p sniplet-core --example arrow_gallery -- artifacts/arrow-gallery.png` reruns the straight/curved size comparison. |
| Native arrow controls | The saved legacy project opened with Solid selected. Native input selected Hand drawn, Thin, and Double ended; a real thumb drag changed size from 10.00 to 21.50, and one Undo restored 10.00. Saved project comparison retained both endpoints, bend point, color, and source-image hash. The Double ended PNG exported at 1600×900 without editing handles. |
| Hover regressions | Tests cover desktop placement and scaling, z-order hit selection, exclusion of ineligible windows, overlapping-window rectangle subtraction, opening inside the Editor command update, replacing an existing overlay, Escape cleanup/document preservation, and removing the overlay before selection capture. |
| Native hover and capture | `Ctrl+Shift+3` from Spotify opened only the desktop overlay. Moving between Spotify and exposed Chrome changed the highlight and camera marker. Chrome's tint and border avoided the foreground Spotify window. Clicking exposed Chrome opened its complete 1818×1365 screenshot without Spotify, the capture overlay, or the debug console in the pixels. Native Save Project wrote `native-hover-window-proof.clippy` and its source PNG under the task artifact directory. |
| Native cancellation | Escape removed the window overlay, and the native window probe confirmed the editor remained minimized. The area shortcut followed by Escape also left the editor minimized. |
| User capture preservation | Before restarting, the user's 1600×900 Spotify capture and curved arrow were saved as `capture-before-arrow-variants.clippy`. The source PNG retains SHA-256 `723352EC45F23BF427CA696FB1FD70F0C33486FEF769E282E83C59AD848D4466`. Verification captures are saved separately. |
| Windows release | Locked release compilation passed in 4 minutes 44 seconds. The staged executable at `dist/arrow-variants/sniplet-x86_64-pc-windows-msvc/Sniplet.exe` is 36,666,880 bytes with SHA-256 `A58020002C8CF2D54160D190D6813A333D2492CF64D00913E7060A3AFA162572`. |
| Packaged diagnostics | `artifacts/arrow-variants-update/verify-release.ps1` reruns bounded startup smoke (20 seconds) and native self-test (60 seconds). Both exited 0. Self-test passed native capture on the 2560×1440 XG270QG display, annotation, undo/redo, crop, PNG export/reload, and font rendering. Logs and diagnostic PNGs are under the same artifact directory. |
| Installed release | The standard portable executable reopened the preserved Spotify project as PID 33468. Native inspection confirmed the 1600×900 capture and curved Solid arrow. A selected Arrow and exposed the size slider with all four variants. `Ctrl+Shift+3` from Spotify opened only the desktop overlay; Chrome and Spotify hover highlights followed the pointer correctly. Escape removed the overlay, and the native window probe confirmed Sniplet remained minimized with the original document retained. All seven portable directories and six Windows ZIPs contain the verified executable; the archives contain only the seven distribution files. |



## October 4, 2026 interactive preview performance

Dragging an arrow control previously rendered the entire document synchronously for every pointer event. Most ordinary annotations also allocated and blended a full-canvas temporary image. The editor now marks the preview dirty and renders the latest geometry once per GPUI frame. Replaced display images are explicitly removed from the GPU atlas. The renderer clones opaque screenshot sources directly, skips an identity-backdrop copy, reuses one scratch layer within painted bounds, blends bounded arrow masks directly, and scans narrow row spans for line/freehand capsules.

The rerunnable release benchmark is `crates/sniplet-core/examples/interactive_render_bench.rs`. Its report and raw before/after CSV files are under `artifacts/interactive-performance`. Median CPU timings on the Windows 11 Ryzen 9 5900X host are:

| Workload | Before | After |
| --- | --- | --- |
| 1600×900 curved arrow | 11.15 ms | 2.14 ms |
| 2560×1440 anchor update and render, seven large annotations | 142.77 ms | 13.43 ms |
| 3840×2160 curved arrow | 92.65 ms | 12.22 ms |
| Preserved 2576×1408 project, existing arrow drag plus BGRA conversion | Not measured | 5.36 ms median, 5.49 ms p95 |

The preserved project contains six annotations: two ruler lines and labels, one freehand stroke, and one curved Solid arrow. Its source PNG SHA-256 is `EA9C1FCDA834DDCEDE7073FC6F6B16A803B4EAD66BC953BD26D93C9551765A21`. Project fixtures remain local test artifacts and are excluded from distribution archives.

These measurements include CPU rasterization and, where stated, the app's BGRA conversion. GPU upload and presentation are not measured. A 4K curved-arrow preview plus BGRA takes 16.22 ms median; seven deliberately large 4K annotations still take 34.86 ms. This is not a claim that every document sustains 60 Hz.

The correctness tests compare every annotation kind and arrow variant byte-for-byte with the original full-canvas compositor, including random alpha/hidden RGB, previews, crop, backdrop, and expanded canvases. The line rasterizer matches the original exhaustive algorithm across explicit edge cases and 200 randomized strokes. GPUI regressions submit 128 raw pointer movements before drawing and assert one latest-state preview, exact export pixels, old GPU-image removal, grouped Undo/Redo, live draft cancellation, and stationary-cursor color resampling.

| Check | Observed evidence |
| --- | --- |
| Windows automated gate | `scripts/verify.ps1 -Suite all` passed formatting, locked all-target/all-feature check/build, 48 core tests, 23 platform tests, and 49 UI-feature app tests, for 120 distinct tests. The 15 ordinary app tests are included in the UI count. Strict workspace lint passed with warnings denied; default-feature app lint and build also passed. One Windows OCR-language-pack test remains ignored. |
| Linux automated gate | `scripts/verify.sh all` passed the same 120 distinct tests, formatting, check/build, and strict workspace lint under WSL `codex-yarr-dev` with `/tmp/clippy-linux-target`. WSL's existing `E:\\` mount warning did not affect Cargo. Logs for both hosts are in `artifacts/interactive-performance`. |
| Windows release | Locked optimized compilation passed in 4 minutes 38 seconds. The executable is 36,668,416 bytes with SHA-256 `187C6E6484CE65CB387F354B40E25949DFC760E67F3E7C5845CC501AC3959C9C`. Bounded startup smoke and native self-test exited 0. Self-test passed capture on the 2560×1440 XG270QG display, annotation, undo/redo, crop, PNG export/reload, and font rendering. `artifacts/interactive-performance/verify-release.ps1` reruns both probes. |
| Installed native editor | The standard portable executable reopened the preserved project as PID 55688. Native input selected Arrow with A, changed its middle bend and both endpoints, and restored each complete drag with one Undo. A diagonal Line also drew and undid correctly. Screenshots in this chat show the resulting geometry; native frame rate was not measured. |
| Installed capture cancellation | Ctrl+Shift+3 from Spotify opened the hover-window overlay with its blue highlight and camera marker; Escape closed it and a native probe confirmed the editor remained minimized. Area capture also opened and cancelled with Escape; the fresh area flow's before/after probes both reported the editor minimized. The inspection tool omitted the area popup from its window list, so the initial overlay was observed through related window-state screenshots. Ctrl+Shift+8 then deliberately reopened the editor for the preservation check. |
| Project preservation | Native Save Project wrote `capture-after-native-verification.clippy`. All six annotation IDs, types, coordinates, styles, source/canvas dimensions, crop, and backdrop match `capture-for-final-restart.clippy`; the companion source PNG retains the SHA-256 above. The original arrow is selected for continued editing. |
| Portable packages | All eight portable directories and seven Windows ZIPs contain the same verified release. Archives carry exactly the seven distribution files and exclude user captures. `artifacts/interactive-performance/refresh-packages.ps1` checks the executable hashes and archive inventory. |

WSL tests do not qualify capture on a physical Linux desktop. Native macOS, physical Linux/compositor, and mixed-DPI hardware checks remain open.

## October 4, 2026 Sniplet identity migration

The app, three Rust crates, executable, window/menu/tray text, export names, application IDs, package metadata, packaging environment variables, and active documentation now use Sniplet. New projects use `.sniplet`; the CLI and editor also recognize legacy `.clippy` projects case-insensitively. On first launch without a Sniplet settings file, the settings store copies valid previous preferences atomically into the Sniplet config directory. An existing Sniplet file takes precedence, and malformed legacy settings are reported without creating a new settings file.

| Check | Observed evidence |
| --- | --- |
| Windows and Linux automated gates | Both full verification scripts passed formatting, locked workspace check/build, 49 core tests, 26 platform tests, and 49 UI-feature app tests: 124 distinct tests per host. The 15 ordinary app tests are included in the UI count. Strict workspace lint passed on both; default-feature Windows app lint also passed. One environment-gated Windows OCR test remains ignored. Logs are under `artifacts/sniplet-rename`. |
| Migration regressions | Tests prove durable migration, current-settings precedence, rejection of malformed legacy settings, current/legacy project classification, and source-image resolution after saving and moving either project format. |
| Windows release | Locked optimized build passed in 5 minutes 3 seconds. The 38,054,400-byte executable has SHA-256 `D9A4539E9E6EA9A93D0D07F6E2B88E88419640DA4123F44845C446EE327D1D4F`. Bounded startup smoke and native self-test exited 0. Self-test passed capture on the 2560×1440 XG270QG display, annotation, undo/redo, crop, PNG export/reload, and font rendering. `artifacts/sniplet-rename/verify-release.ps1` reruns these probes. |
| Linux package and presentation | Default-feature debug build and portable packaging passed under `codex-yarr-dev`, reusing the existing `/tmp/clippy-linux-target` cache. The bundle contains `bin/sniplet` and a desktop entry with Sniplet display name and startup class. Its application ID was subsequently updated as recorded below. Bounded WSLg demo smoke exited 0; its log retains the previously documented EGL/Zink warnings and caught unsupported capture-protocol panic. This does not qualify physical Linux capture. |
| Native settings migration | Windows created the then-current Sniplet settings file. Its parsed contents and SHA-256 equaled the previous file; the original preferences remained intact. This migration behavior was subsequently removed as recorded below. |
| Native editor and projects | The standard portable `Sniplet.exe` launched as PID 34244 with the title **Sniplet** and opened the preserved `.clippy` project. The native Save Project dialog suggested `Sniplet.sniplet`. Saving `capture-after-rename.sniplet` preserved all annotation, source/canvas, crop, and backdrop fields exactly; its companion source PNG has the pre-restart SHA-256 `8706CA3EC566E041A876A50D72A87C852F76F6769F9B834928359DA7FDC4C1A9`. The source is 921×574, with one text annotation and a 100-pixel gradient backdrop. |
| Distribution census | Nine Windows portable directories and eight ZIPs contain the verified release; three Linux debug trees contain the new Linux build. Each Windows ZIP has exactly the seven distribution files, verified executable hash, and no capture fixtures. Product filenames under both distribution roots contain no Clippy branding. `artifacts/sniplet-rename/refresh-packages.ps1` reruns the refresh and checks. |
| Source census | Remaining active-source references to Clippy are the Rust linter and deliberate legacy-project/settings compatibility. Historical capture artifacts and the active checkout/build-cache paths keep their actual names. The one-time source codemod and pre-rename backups are under `artifacts/sniplet-rename`; rerunning it after the crate rename leaves compatibility references intact. |

This is development evidence from an uncommitted working tree. macOS packaging metadata was updated, but no native macOS build or desktop was available during this rename.

## October 4, 2026 application namespace correction

The application ID is now `io.github.m4tta.sniplet`. The editor and window-capture overlay use the shared Rust `APP_ID` constant. The packaging script applies the same ID to the macOS bundle, Linux desktop filename, and Linux startup class, matching GPUI's X11 window class and Wayland app ID.

Preferences use `ProjectDirs::from("io.github", "m4tta", "sniplet")`, preserving normal per-platform directory conventions. Settings migration is removed at the user's request; the current store reads only its current path and uses defaults when that file is missing. This supersedes the settings-migration behavior in the earlier rename record. Atomic saves, replacement of existing preferences, malformed-file errors, and legacy `.clippy` project support remain intact.

Final Windows and Linux full verification passed format, locked workspace check/build, strict lint, 49 core tests, 24 platform tests, and 49 UI-feature app tests: 122 distinct tests per host. Windows also passed default-feature lint, bounded packaged startup smoke, and native capture/image diagnostics on the 2560×1440 XG270QG display. Linux passed default-feature build, packaging with `io.github.m4tta.sniplet.desktop` and matching startup class, shell syntax checking, and bounded WSLg startup smoke. Logs and the rerunnable Windows native probe are under `artifacts/app-namespace`; they are local development evidence. The initial repository commit `3fffbcf` also passed all four [GitHub Actions jobs](https://github.com/m4tta/sniplet/actions/runs/37230646089), including the macOS workspace. The native Mac desktop record below covers the initial commit; the namespace correction still needs a native Mac test.

## Observed macOS run: October 4, 2026

The tests started from clean commit `3fffbcf426a422b152b18daa6fb71bb4aa9b8b43`. This record covers an Apple Silicon debug build before the namespace correction above. Logs, screenshots, and image fixtures are in `artifacts/mac-testing`.

| Check | Evidence |
| --- | --- |
| Host | macOS 26.5 build 25F71, Apple M1 Max, 32 GB RAM, built-in Retina display |
| Build tools | Rust and Cargo 1.99.0, Xcode 26.6 build 17F113, Metal compiler 32023.883. The existing Rust 1.84.1 could not parse edition 2024. Stable Rust was updated, and the missing Xcode Metal component was installed. |
| Automated suite | `bash scripts/verify.sh all` passed format, locked workspace check/build, 49 core tests, 26 platform tests, and 49 app tests with UI features. The 15 ordinary app tests are included in the UI count: 124 distinct tests, with no failures or ignored tests. Strict workspace Clippy passed with warnings denied. See `verify.log`. |
| Startup and native diagnostic | `--demo --smoke --normal-window` and `--self-test artifacts/mac-testing/self-test` exited 0. The diagnostic reported a 4112×2658 display and passed annotation, undo/redo, crop, PNG reload, and font checks. These command-line results do not prove the packaged app's Screen Recording access. |
| Debug package | `SNIPLET_PACKAGE_PROFILE=debug bash scripts/package.sh` created `dist/Sniplet-debug.app`. Its plist passed `plutil -lint`. Executable SHA-256: `796784e0d3872de1119dddc0858aaedfcdd637a912fb8c86873012c26075abd5`. See `package.log`. |
| Native editor | The packaged app opened the demo PNG, selected Arrow with A, and drew an arrow with a pointer drag. Cmd+Z removed it; Cmd+Shift+Z restored it. `native-arrow.png` records the result. |
| PNG export | The native Save dialog wrote `native-save.png` at 1000×660. Apple Preview opened and displayed it. See `preview-save.png`. |
| Clipboard | Cmd+C reported Image copied. Preview's New from Clipboard displayed the image, then saved it as `native-clipboard.png`. Both images are 1000×660 with equal alpha. Preview's saved copy differs at 29 pixels by at most one RGB unit; exact pixel equality is not claimed. See `clipboard-comparison.json` and `preview-clipboard.png`. |
| Project save/reload | The native Save Project dialog wrote `native-project.sniplet` and its source PNG. After the permission restart, the app reopened the project with its 400×300 source and solid arrow intact. See `project-reload.png`. |
| Screen Recording | The user approved the grant and completed the administrator prompt. System Settings showed Sniplet-debug enabled. Quit & Reopen restarted the packaged app. |
| Full-screen capture | After the grant, Capture screen loaded a 4112×2658 image with the desktop and app windows visible. See `screen-permission-granted.png`. |
| Area capture | Capture area loaded a 720×460 region of the Preview image after a drag from physical pixel (3260,700) to (3980,1160). The image showed the fixture's text and arrows. See `area-permission-granted.png`. |
| Window capture | Failed. With the picker open, selecting physical pixel (3500,900) inside the visible Preview fixture produced a 4112×2658 image of the Dock over a blank background. Status said Window captured. A second attempt produced the same wrong result. See `window-wrong-target.png`. |
| Window cancellation | Escape sent to the picker removed it and left the existing image intact. See `window-cancel.png`. The background editor's minimized state was not confirmed by the per-app inspection tool. |
| OCR | The native command reported that the `tesseract` executable was not found. OCR requires this optional dependency and its language data on this host. See `native-ocr.png`. |

Before Screen Recording was enabled, area selection produced a 400×300 wallpaper image. Escape removed an area overlay and preserved the existing document. Window selection closed its picker but did not produce a new capture. The later permission grant resolved capture of app windows for screen and area capture. It did not resolve window selection.

The window result needs a fix before macOS capture can pass as a whole. `WindowInfo::is_capture_candidate` accepts any non-minimized, titled window from another process. `window_at_point` then selects the largest z-order at that point. The macOS route has no filter for system helper windows. The initial run did not log the selected native window ID. The follow-up below confirms the cause.



Tray lifecycle, pin behavior, scrolling capture, mixed displays, Spaces/fullscreen, Intel Macs, and signed release packaging still need separate evidence. The upstream namespace record above reports the initial commit's GitHub Actions results. No application source files changed during this test run.

## macOS window picker fix: October 4, 2026

The fix starts from commit `1884b67` on `codex/fix-macos-window-capture`. A native window probe confirmed that the Dock owns window 9393 on layer 20. Its 2056×1329 point bounds cover the display. Preview window 63546 uses layer 0. At desktop point (504,425), the old picker selected the Dock because its z-order was 20, above Preview's 11. See `artifacts/mac-window-fix/native-before.json`.

Mac window enumeration now checks Core Graphics metadata and offers only normal application windows on layer 0. It preserves the existing z-order selection between those windows. The follow-up native probe offered no Dock windows and selected the front application window at the test point. That application was ChatGPT, which covered Preview during the probe. See `native-after.json`.

The focused test uses Core Foundation window dictionaries for a display-sized Dock window on layer 20 and an overlapping Preview window on layer 0. It failed with Dock before the filter and passed with Preview after the filter. All 25 platform tests and 49 app tests with UI features passed. Strict workspace Clippy passed with all targets and features and warnings denied. The default app build and debug packaging passed. Logs are in `artifacts/mac-window-fix`.

The rebuilt bundle uses `io.github.m4tta.sniplet`. Its plist passed validation. Executable SHA-256: `2d1de1a83dec3cd4f9a101c5c70cc19d1271eaa00161a9913e25af4c1ecf231c`. The Mac control tool needed a fresh test bundle path to clear its cached old app ID. The duplicate test instance was then closed; the final test uses `dist/Sniplet-debug.app`.

Native Preview capture is pending the administrator prompt for Screen Recording. The new app ID did not inherit the earlier grant. Before the grant, the rebuilt picker showed wallpaper only. See `picker-before-permission.png`.

## Local packaging

Packaging is host-native and does not install, sign, publish, or register the application:

```powershell
./scripts/package.ps1
./scripts/package.ps1 -Profile debug
```

```sh
./scripts/package.sh
SNIPLET_PACKAGE_PROFILE=debug ./scripts/package.sh
```

The PowerShell script builds a release executable and creates `dist/sniplet-<windows-host-target>/Sniplet.exe`. The shell script creates `dist/Sniplet.app` on macOS or a `dist/sniplet-<linux-host-target>` portable tree containing `bin/sniplet` and `share/applications/io.github.m4tta.sniplet.desktop` on Linux. The macOS bundle ID, editor/capture window app IDs, and settings namespace are `io.github.m4tta.sniplet`. Every package carries the project README and license, bundled Noto Sans OFL text, and Markdown documentation. Pass a relative output directory as the first shell argument or `-OutputDirectory` in PowerShell. Set `SNIPLET_PACKAGE_SKIP_BUILD=1` on macOS/Linux, or pass `-SkipBuild` on Windows, only when the matching selected-profile binary was already built on that host.

The optional debug profile reuses the faster development build and adds `-debug` to the package directory or app-bundle name. It is useful for local smoke evidence and is not a release artifact. With `-SkipBuild` or `SNIPLET_PACKAGE_SKIP_BUILD=1`, the selected profile must already exist on that host.

These outputs are developer artifacts. macOS signing/notarization, Windows signing, Linux distribution packaging, icons, installers, update metadata, and release publication require separate release work and evidence.

## Diagnostic entry points

The application CLI provides these diagnostics:

```sh
cargo run -p sniplet-app -- --self-test
cargo run -p sniplet-app -- --smoke
```

`--self-test` returns a nonzero exit status when monitor discovery, native capture, annotation/undo/redo, crop, font rendering, PNG export, or image reload fails. On success it prints a JSON result and writes `native-capture.png` and `editor-demo.png` under the supplied directory. It may trigger the operating system's screen-capture permission UI because capture is the behavior under test; it does not register hotkeys or install persistent OS state.

`--demo --smoke --normal-window` starts the real application with a generated document in an ordinary window and exits after roughly three seconds. It proves that GPUI can create, render, and close the application window in that session. It cannot report native capture, global hotkeys, pinning, tray, or clipboard integration as passed.

## Desktop test record

Create one record for every OS/display-session combination. Attach screenshots or a short recording for visual findings and preserve command output for failures.

```text
Commit:
Sniplet build/profile:
Date and tester:
OS and version:
Desktop/session: macOS / Windows / X11 desktop / Wayland compositor
Display topology: resolution, scale factor, position, primary display
GPU and driver:
Permissions granted:
Configured global shortcuts:
Result: pass / fail / blocked / not run
Evidence paths or links:
Notes:
```

Use a clean settings directory for first-launch tests, then repeat persistence cases with the same directory. Record exact versions instead of labels such as “latest.”

## Manual application flow

### 1. First launch, tray, and lifecycle

1. Start Sniplet with no existing settings.
2. Confirm that permission or capability guidance is specific to the current OS and that denial leaves a usable explanation and retry path.
3. Confirm that the tray/menu item exposes every implemented capture route and Settings.
4. Close the editor. Confirm the documented background behavior, then reopen it from the tray and configured reopen shortcut.
5. Quit from the tray and verify that the process and registered hotkeys are gone.



### 2. Global hotkeys and conflicts

1. Assign distinct global shortcuts for fullscreen, area, repeat area, any window, active window, scrolling capture, OCR/QR, and reopen when those routes are implemented.
2. Trigger each shortcut while Sniplet is unfocused and while another application is fullscreen.
3. Attempt to assign an OS-reserved or already registered combination. Sniplet must reject it clearly and retain the prior working binding.
4. Change a shortcut, restart Sniplet, and verify that only the new binding fires once.
5. Exercise non-US keyboard layout and modifier-only edge cases.



### 3. Area overlay and mixed DPI

Use at least two displays with different scale factors where hardware permits; include a display positioned left or above the primary display.

1. Start area capture on each display and drag in all directions. Escape must cancel, leave the editor hidden, and avoid changing the document or clipboard.
2. Select a region wholly within each display. Compare exported physical pixel dimensions to the selected physical extent.
3. Select across the display boundary. The overlay, pointer, selection rectangle, and resulting pixels must agree through the scale transition; no gap, duplicate strip, or coordinate jump is acceptable.
4. Verify negative desktop coordinates, menu/taskbar boundaries, and a one-pixel selection.
5. Toggle logical/physical dimension display in the editor and record both values.
6. Zoom into crisp UI edges at 100%; verify that the image is not silently resampled.

This is the release gate for coordinate conversion. A single-scale virtual display is insufficient evidence for mixed-DPI support.

### 4. Screen and window capture

1. Capture each complete monitor and compare bounds, rotation, and pixel dimensions.
2. Start window capture from another app and confirm only the desktop overlay appears. Hover an active window and a deliberately chosen inactive window; check the camera marker, highlight bounds, and label. With overlapping windows, verify the background highlight does not cover foreground windows. Click each target and confirm the overlay is absent from captured pixels.
3. Check transparent/shadow edges against the selected window presentation mode.
4. Capture a window partly outside a display and one spanning scaled displays.
5. Verify delayed capture timing and cancellation. Escape and right-click should dismiss window capture and leave the editor hidden with its prior document preserved.
6. Repeat with cursor inclusion enabled and disabled when available.

### 5. Editor interaction and annotation export

Use a fixture with fine one-pixel lines, flat-color regions, text, transparency, and a color chart.



### 6. Clipboard and file import

1. Load PNG and JPEG files, including a large image and a filename with non-ASCII characters.
2. Load an image from the clipboard and paste another image as a movable overlay.
3. Copy an edited image and paste it into at least one native application and one browser application.
4. Verify graceful errors for non-image clipboard data, corrupt files, unwritable destinations, and a destination disappearing during save.

### 7. Pin window

1. Pin the current image and confirm that the resting window has no editor chrome.
2. Confirm always-on-top behavior above ordinary windows. Record platform/compositor limitations for fullscreen spaces separately.
3. Resize with the mouse wheel and verify that repeated scaling does not alter the source bitmap.
4. Change opacity. A semitransparent pin should not retain an opaque window shadow.
5. Move the pin across mixed-scale displays and compare pointer hit targets and image sharpness.
6. Use the pencil control to return to editing, change the image, and pin it again.
7. Close the pin and verify that focus returns sensibly and no invisible topmost window remains.

### 8. Scrolling capture

Test a static page with repeated patterns, a fixed header, animation, and a known full height.

1. Run automatic capture downward and upward at slow and fast configured speeds.
2. Compare seams and output dimensions to the source page. Repeated rows, missing rows, or blended text are failures.
3. Cancel with Escape and stop naturally at the content end.
4. Reach the configured maximum height and verify an explicit message rather than truncation presented as success.
5. Run manual mode, scroll steadily, pause to finish, and intentionally exceed the recommended pace to verify the speed warning.
6. Repeat in a native scroll view and a browser. Record when OS security or the Wayland compositor prevents scroll injection or capture.

### 9. OCR, QR, colors, and settings

1. OCR a region containing multiple lines, then test line-break removal and a second configured language.
2. Decode a known QR fixture and reject an image without a valid code.
3. Copy pixel color, nearby text color, and average selection color; compare numeric values to the fixture.
4. Change every implemented setting, restart, and verify persistence. Confirm that corrupt settings produce a recoverable error or safe migration.
5. Verify that secrets, when upload support exists, are stored in the OS credential store rather than the ordinary settings JSON.

## macOS linked magnifier, October 4, 2026

The user supplied an image with a small source circle, a larger magnified circle and a connecting line. The old magnifier stored only lens bounds and a factor. The local change adds a source center and keeps the old rendering path for projects without that field.

| Check | Observed result |
| --- | --- |
| Source state | Base commit `1884b67`, branch `codex/fix-macos-window-capture`, uncommitted window-filter, image-layer and magnifier fixes |
| Automated checks | All 52 core tests and 50 app tests passed; strict workspace Clippy, format and diff checks passed |
| Focused tests | `linked_magnifier_keeps_its_source_when_the_lens_moves` failed before the renderer change and passed after it. `magnifier_circles_move_and_resize_independently_with_grouped_undo` passed for both move and size handles, undo/redo and Escape |
| Package | Locked debug build passed. `dist/Sniplet-debug.app/Contents/MacOS/Sniplet` SHA-256: `2c43d9a95a97f64a9e679e00fb6fc0f9d56183afd6e22a992e45804c831c1274` |
| Native editor | The updated bundle opened a project in a normal Mac window. Lens movement kept the source fixed; source movement changed the enlarged pixels. Undo restored both moves. The factor control changed 3× to 4× and undo restored 3× |
| Native creation and size | A drag on a clean fixture created both circles and their line at 3×. Source resize changed the factor; lens resize preserved the factor. Undo restored each size change |
| Native export | `native-export.png` is byte-identical to `linked-magnifier.png`, at 1000×660 pixels. SHA-256: `f92d49af34b5277c8d3b01b433bf0b0146ba6d115cad48e5176a2e65574163e9` |
| Preserved document | The previously open image and annotations were saved as `artifacts/magnifier-callout/open-before-magnifier-update.sniplet` with its source PNG before the app update |
| Evidence | Logs, the user reference, project, core/native exports and native editor screenshots are under `artifacts/magnifier-callout/` |

The native tests used focused editor commands and the toolbar. Both applications have overlapping global shortcuts, so this run does not verify global command delivery. Screen Recording was not needed to open the fixture. The pending grant and native Preview capture remain separate checks.



## macOS measurement, October 4, 2026

The user selected all three measurement mock states: live preview, placed gap and outer size. Ruler now opens keyboard help. Held measurement uses the visible source raster, including raster edits and excluding added image objects and drawings. Placement stores the line, caps and label as one raster operation. Capture scale persists in projects.

| Check | Result |
| --- | --- |
| Failure reproduction | The held-key UI test failed because no preview existed. A later pixel test failed because Retina text extended beyond its label background. Both pass after the changes |
| Core | 56 tests passed, including axes, gaps, borderless/outer size, sensitivity, alpha, raster layers, label bounds, export, project scale and undo/redo |
| App | 53 tests passed, including held keys, pointer placement, modifier events, wheel input, pt/px switching, crop boundaries, Escape, text entry and selection arrow keys |
| Build checks | Strict workspace Clippy, locked debug build, format and diff checks passed |
| Updated bundle | `dist/Sniplet-debug.app`; executable SHA-256 `017ec6f54323df01c937ac33c0670a580d7fc727f681acfe492c6b6e7504121f` |
| Native editor | Opened `outer-size.sniplet`, displayed `136px`, opened keyboard help and switched 400×220pt to 800×440px and back |
| Native export | `native-export.png` equals `outer-size.png` byte for byte: 800×440, 9,811 bytes, SHA-256 `8ed9132da3acf6a3334eb4834259a03bc44767b08d4922772c1b4bb13b590840` |
| Evidence | Source fixture, three PNG states, two projects, native help/unit screenshots, native export and logs are under `artifacts/measure-parity-2026-10-04/` |



## macOS image drag button, October 4, 2026



| Check | Result |
| --- | --- |
| App tests | All 56 passed, including three new image-drag tests |
| Drag gesture | GPUI pointer input starts the drag from the button; no image disables the gesture |
| Exported file | PNG reload matches Save/Copy pixels for annotations, crop, backdrop and an active raster selection |
| Build | Strict app Clippy and locked Mac debug build passed |
| Bundle | `dist/Sniplet-debug.app`; executable SHA-256 `da143db4f3a578be792c1e2477b927bf3c840c14887948e313f5d794033e9b62` |
| Native test | The user confirmed the button works after unlocking the Mac. The agent restarted the updated bundle and inspected the button, then stopped the drop test at the user's request |
| Evidence | `artifacts/drag-image-2026-10-04/results.json` and the supplied reference image |

Automated tests verify the file payload. The user confirmed the native button works; the drop target was not specified. The agent did not complete an independent Finder drop. Temporary-file cleanup remains open. The updated bundle is running.

## macOS window button alignment, October 4, 2026

The Mac titlebar used a 9-point top offset for the native window buttons. The 54-point editor toolbar needs a 20-point offset to center the 14-point buttons. The Mac window options now calculate this offset from the editor toolbar height.

Format, strict app Clippy and the locked debug build passed. The updated `dist/Sniplet-debug.app` executable has SHA-256 `84d65822b03f565d83fd539134975f7e960e0d6c20d77675ab1605da45a35b15`.

The native alignment test passed on October 5, 2026, on macOS 26.5 (25F71). The app was restarted with this bundle. All three buttons have a vertical center within one pixel of the toolbar center in a 2× screenshot. The green button opened full screen. Control-Command-F and Escape did not exit full screen during the test; the native buttons were hidden. Quit and relaunch restored the normal window. Minimize and close-to-tray remain unverified: the test tool sent the button clicks but continued to report the window. No document was open during these checks.

Sniplet is running in a normal window (`--normal-window`). The user reference, native screenshots and results are under `artifacts/window-controls-2026-10-04/`. This test confirms alignment; it does not complete the Spaces/fullscreen release gate.

## macOS menu bar and editor lifecycle, October 5, 2026

The user selected layout A from the menu mocks. The native scissors menu now
contains capture commands, a More submenu, GitHub, Launch at Startup, Settings
and Quit. It has no purchase or upload item. Shortcut labels and registrations
follow the saved hotkey settings without a restart. Load From Clipboard opens
the clipboard image as a document; editor Paste still adds an image layer.

Closing the editor hides its native window and changes the app to menu bar mode.
It keeps the document in memory. Reopen restores the Dock icon and editor.
Successful whole-image Copy and Save use this path when hide-after-export is on.

The signed debug bundle passed its native lifecycle check on macOS 26.5 (25F71).
Its executable SHA-256 is
`e295e74b654ad77e73bb2b57da4f336f17dce2a984edf6f3e569b8b229b876dd`.

| Check | Result |
| --- | --- |
| Automated checks | Format, strict app/platform Clippy with UI tests enabled, all 58 app tests and five scroll tests passed |
| Package | Locked debug build and `codesign --verify --strict` passed |
| Close | The native red button changed process 74984 from Foreground to UIElement; the process kept running |
| Reopen | Native reopen returned the same process to Foreground; the demo image, annotations and 100% zoom remained |
| Quit | Command-Q stopped the process; the updated app was then relaunched |
| Evidence | `artifacts/menu-bar-2026-10-05/results.json`, `native-before-close.png` and `native-reopened.png` |

CUA opens the app when it inspects Sniplet. The close test therefore read the
macOS application registry before the next Sniplet inspection. UIElement is
the macOS mode that removes the app from the Dock.

More → Scrolling (Up) sends upward wheel steps and reverses the collected frames
before stitching. The exact image-order test passed. Native upward capture,
native scissors-menu selection, exact icon placement and startup registration
remain unverified. The test did not enable Launch at Startup.

## macOS capture placement and delay, October 5, 2026

The area selection window was 39 points below the display's top edge. GPUI
kept the native titled-window style even with `titlebar: None`, so AppKit moved
the window below the menu bar. The Mac area and window pickers now use a
borderless panel with the captured display's full frame. The panel stays hidden
until its style and frame are set.

Capture now hides the Mac editor without animation. It no longer minimizes the
editor or waits a fixed 220 ms. The explicit three-second capture delay remains.
The debug profile also optimizes the app and `xcap` pixel conversion code.

| Check | Result |
| --- | --- |
| Environment | macOS 26.5 (25F71), one Retina display, 2056 × 1329 points / 4112 × 2658 pixels |
| Placement | Before: `(0, 39)` points. After: `(0, 0)`, with the full display size and one menu bar |
| Command to selection frame | Baseline: 2003 and 1730 ms. Final build: 204 and 168 ms. These are local samples, not a latency guarantee |
| Native selection | A drag from `(8, 8)` to `(608, 168)` physical pixels exported a 600 × 160 PNG with the expected menu bar and top window edge |
| Native cancellation | Escape closes area and window selection. Reopen retains the editor image |
| Automated checks | All 58 app tests, format, strict app/platform Clippy with UI tests enabled, locked debug build, and strict bundle signature verification passed |
| Bundle | `dist/Sniplet-debug.app`; executable SHA-256 `3c996646193426a4a51ca19d5dfecc9f23f3a22ba2eb78a76a8c1a70a35d257e` |
| Evidence | `artifacts/area-capture-2026-10-05/results.json`, timing logs, native screenshots, and `top-edge-export.png` |





## macOS area capture across displays, October 5, 2026

Area capture now opens one selection window per display and shares one desktop
rectangle. On macOS, that rectangle uses Quartz origins because GPUI reports
display-local bounds. A capture that spans displays produces one image at the
highest selected display scale. Repeat Capture retains the rectangle and checks
the display layout before reuse.

The native test used macOS 26.5 (25F71) on an M1 Max. The laptop display was
2056 × 1329 points at `(0, 0)`. The external DELL U2720QM was 3008 × 1692 points
at `(-356, -1692)`. Both displays used a 2× capture scale.

| Check | Result |
| --- | --- |
| External display only | A 300 × 60 point selection exported a 600 × 120 pixel PNG |
| Drag across displays | External-to-laptop and laptop-to-external drags both selected `(1094, -57, 250, 160)` points and produced 500 × 320 pixels |
| Repeat Capture | Exported the same 500 × 320 pixel PNG, with an identical file hash |
| Escape | Ended the native capture session; UI tests verify that both overlays close and the existing document remains |
| Automated checks | 63 app tests, 26 platform tests, format, strict app/platform Clippy with UI tests, debug build, and strict bundle signature verification passed |
| Bundle | `dist/Sniplet-debug.app`; executable SHA-256 `c0446eaa10fed0ddd366905df1111e301befb1f1c510f2ee433af9094edf0784` |
| Evidence | `artifacts/multi-display-2026-10-05/results.json`, native logs, editor screenshots, and PNG exports |



## macOS area capture cursor, October 5, 2026

The crosshair could change back to an arrow after the capture panels opened.
GPUI applies cursor changes to the key window, but the pointer could be on a
different display. Each capture panel now has a native crosshair region. The
panel under the pointer gets focus before selection; a mouse-down also sets
focus, and the starting panel keeps it throughout the drag.

The native check used the two displays listed above. Cursor traces confirmed
the crosshair at 200 ms and 1.5 seconds after opening, and during drags in both
directions across the display boundary. Both drags produced 500 × 220 pixel
captures. Escape closed both panels. Closing the panels restored the arrow.
Six focused capture tests, strict app Clippy, formatting, the debug build and
bundle signature checks passed.

Evidence is in `artifacts/capture-cursor-2026-10-05/results.json` and
`verified-cursor.log`. The tested executable has SHA-256
`d71c4d2c5a6d7182fb5025bdf690adf8bc4913fb5896d98d113ae459ac4f0a24`.
The native checks used toolbar commands; the UI tool did not deliver the global
capture shortcut, so this run does not verify shortcut delivery. The cursor
probes require `SNIPLET_CAPTURE_TRACE`; they do not run in a normal launch.

## macOS local development signing, October 5, 2026

The macOS package script now selects the local `Sniplet Development` certificate
when it is available. It does not select an unrelated developer identity. An
explicit `SNIPLET_SIGNING_IDENTITY` overrides this choice; `-` uses ad hoc signing.

The test used macOS 26.5 (25F71) on the M1 Max above. A new self-signed certificate
and private key were installed in the login Keychain. The certificate has no
Apple account or team. Its SHA-1 fingerprint is
`25D338C34E12575EBFB10D6D9BD1838BCA7212A9`. Certificate trust settings were unchanged.
The private key is in the Keychain; export files were removed after import.

Two existing Sniplet builds, from October 4 and October 5, were packaged and
signed with this certificate. Their executable hashes and CodeDirectory hashes
differ. Both pass strict signature verification and satisfy the other build's
designated requirement:

```text
identifier "io.github.m4tta.sniplet" and certificate leaf = H"25d338c34e12575ebfb10d6d9bd1838bca7212a9"
```

Shell syntax, whitespace, and explicit ad hoc signing checks passed. The signed
current app opened the saved editor document. The initial capture returned only
the desktop background. The macOS TCC log confirmed that the old permission
still required an ad hoc CodeDirectory hash. Turning the permission off and on
did not replace that requirement. Removing the old entry and adding the signed
app from `dist/Sniplet-debug.app` fixed capture.

The current signed build captured other app windows. The executable at that same
path was then replaced with the signed October 4 build. It also captured other
app windows without a new grant or a permission prompt. This verifies capture
permission across the two different builds. Evidence is in
`artifacts/local-signing-2026-10-05/results.json`, the signature reports,
`current-capture.png`, and `previous-capture.png`.
This local certificate is for development; it does not notarize public releases.

## App and tray icons, October 5, 2026

The app uses the selected capture icon with red scissors on a dark tile. macOS
packages include an ICNS file and `CFBundleIconFile`. Windows builds embed icon
resource 1, which GPUI loads for the window and taskbar. Linux packages include
the matching desktop entry, SVG, and PNG sizes under `share/icons/hicolor`; X11
windows also receive the 256-pixel icon directly.

The Windows tray uses the same scissors mask as the macOS menu bar. Its ink
changes with `SystemUsesLightTheme`, including after a theme change. The focused
mask test passed for both ink colors. PNG size and transparency checks passed;
the master PNG matches the selected option C. The ICO contains nine sizes from
16 to 256 pixels. Apple `iconutil` decoded the ICNS file.

The Mac debug build opened and passed strict signature verification. Its Mach-O
UUID matches the built executable. `NSWorkspace.icon(forFile:)` returned the new
app icon. Evidence is in `artifacts/app-icon/assets.json`, `results.json`, and
`macos-bundle-icon.png`. Windows and Linux native icon checks remain open.

## Capture Screen under the mouse pointer, October 5, 2026

Capture Screen previously used the monitor index chosen when the editor started.
That index was the primary display. Screen and delayed capture now read the
desktop mouse position when capture begins and use native display hit testing.
Delayed capture reads the position after its three-second wait. Manual scrolling
keeps its selected monitor.

The focused command-routing test failed with `Monitor(0)` before the fix and
passed afterward. Strict app Clippy passed. The Mac debug bundle passed strict
signature verification with the existing local Sniplet certificate.

Native checks used the two displays listed above and the toolbar's Capture
Screen command. At `(-58, -1644)`, the backend selected the external display,
ID 2, and returned 6016 × 3384 pixels at 2× scale. At `(1238, 1171)`, it selected
the laptop display, ID 1, and returned 4112 × 2658 pixels at 2× scale. Evidence
is in `artifacts/screen-under-pointer-2026-10-05/native.log`, `layout.json`, and
`results.json`. The UI tool did not deliver the global shortcut in this run.
Native delayed capture, Windows, and Linux checks remain open.

## Window capture across displays, October 5, 2026

Capture Any Window previously opened one selection panel on the primary display.
It now opens a panel on each display. The panels share the selected window and
show its visible portion on each screen. A click closes every panel before
capture starts. Escape and right-click cancel the whole selection session.

The external-display test failed before the fix because only one panel opened.
All five window-picker tests now pass. They cover external selection, a window
that spans displays, shared highlighting, overlap order, and cancellation.
Formatting, strict app Clippy with UI tests, and the locked Mac debug build passed.

Native checks used the two Mac displays listed above. The external Arc window,
ID 64831, produced 6014 × 3054 pixels. The laptop window, ID 63294, produced
4112 × 2390 pixels. Both selection panels closed after each click. Escape also
closed both panels without another capture. The signed build retained its
existing Screen Recording permission.

Evidence is in `artifacts/window-capture-displays-2026-10-05/results.json`, native
logs, window bounds, and screenshots. The executable SHA-256 is
`dee9ad12f3efb267c80e1e40efc1063e4410c009db914223c6731e7686bfa8d6`.
These checks used toolbar commands. Global shortcut delivery, physical
mixed-scale displays, Windows, and Linux remain untested in this run.

## Live area capture, October 5, 2026

On macOS, area capture now opens transparent selection panels before it reads
any screen pixels. Mouse release captures only the selected parts of each
display and combines them in desktop coordinates. Repeat uses the same region.
Pixels reflect the desktop at capture time, so content can change during a drag.
Windows and Linux retain the existing screenshot-based area selector.

Capture commands keep the editor visible. The Mac region backend excludes only
the selector window IDs. Window capture can select Sniplet itself. Active-window
capture uses the editor's native ID when Sniplet is active, because xcap marks
every window of the active Mac process as focused, including a small system
capture-status window.

Native checks covered both displays, a 600 × 600 pixel capture across their
boundary, Repeat, Escape, a visible Sniplet area, and Sniplet window capture.
The final active-window check captured the 2560 × 1702 pixel editor instead of
the 132 × 40 pixel status window. Screen capture under the external-display
pointer returned 6016 × 3384 pixels. Rebuilds kept Screen Recording permission.

Selector frame callbacks measured 54 to 126 ms in the final builds, compared
with 434 to 490 ms before this change. These callbacks measure UI scheduling,
not physical shortcut delivery or hardware presentation. Pixel readback now
happens after selection. Release-to-editor callbacks ranged from 129 to 568 ms
across the recorded captures. Debug optimization reduced the external-screen
editor-load sample from 711 ms to 184 ms; these are local samples, not a benchmark.

All 68 app UI tests and five platform capture tests passed. The final area tests,
strict app/platform Clippy, formatting, locked build, and bundle signature check
also passed. Evidence is in `artifacts/capture-speed-2026-10-05/results.json`
and its native logs. The record identifies the base commit and final executable.
Physical global shortcuts, native mixed-scale displays, Windows, and Linux
remain untested in this run.

## October 5, 2026: Mac area-drag pauses

On macOS 26.5 (25F71), the native profiler found the UI thread waiting in
`SMAppService.status`. The runtime checked Launch at Startup once a second
from the same task that processed shortcuts and menu commands. Each synchronous
system-service request could stop drawing and input processing.

The periodic status check now runs in its own task, with the system request on
a background worker. The checkbox update stays on the UI thread. The worker uses
an Objective-C autorelease pool. Windows retains its existing tray-theme check.

Both native profiles ran for 12 seconds with a 2 ms sampling interval and
included the same large area drag. Before the change, 2,404 of 4,484 main-thread
samples (53.6%) were in `SMAppService.status`. After the change, none of 4,706
main-thread samples were in that call. The second profile found the status
requests on background workers. This measures the removal of that UI block;
it does not measure display frame rate or establish a bound on all input delays.

The rebuilt, locally signed app completed the area capture. Escape closed a
second selection. Seven area tests, three runtime tests, strict app Clippy with
UI tests enabled, formatting, the locked build, and the bundle signature check
passed. The native profiles and counts are in
`artifacts/area-drag-2026-10-05/`. Windows and Linux were not run for this change.

## October 6, 2026: Editor and export responsiveness

The checkout was updated with `git pull --rebase --autostash origin main` before
these changes. It advanced to `f4ac199`, and the local area-drag fix was restored
without conflicts.

Large preview rendering and RGBA-to-BGRA conversion now run on a worker. One
preview render runs at a time. Pointer input updates the pending state, and each
completed render can appear during a continuous drag. A result from another
document is discarded. Previews of at most 262,144 canvas pixels keep the small
synchronous path. Opening and cancelling a file dialog does not cancel the
current document's preview.

Automatic and manual image copying, file decoding, Save rendering and encoding,
QR/OCR preparation, large pasted-image encoding, large measurement-source
preparation, and manual scroll stitching now use workers. Clipboard writes are
ordered so an old render cannot replace a later successful copy. A failed text
scan does not cancel a pending automatic image copy. Source pixels are shared
with workers instead of copied on the UI thread. Settings saves keep one write
in progress and then save the latest values. The app waits for pending settings
saves during normal shutdown.

The new settings window also queried `SMAppService.status` on each draw. It now
uses cached startup state. Startup checks and registration changes run on
workers with Objective-C autorelease pools.

On macOS 26.5 (25F71), the old build's 12-second capture/copy profile contained
150 main-thread samples in image clipboard writes and 17 in raster rendering.
The updated build's 15-second capture/copy profile contained zero main-thread
samples in either call. It found both calls on background workers. A separate
20-second drawing/Save profile also contained zero main-thread samples in raster
rendering, PNG encoding, image clipboard writes, or startup-status checks. All
profiles used a 1 ms sampling interval. These checks show where the work runs;
they do not measure frame rate or set a maximum input delay.

Native screen capture, Copy, arrow drawing, and Save passed. The PNG export is
6016 × 3384 pixels. Area capture produced a 500 × 275 point image, and Escape
cancelled a second area selection. No new Screen Recording approval was needed.

All 83 app tests, five focused preview tests after the final document guard
change, two focused stitch tests, strict app Clippy with UI tests enabled,
formatting, the locked Mac build, and strict bundle-signature verification
passed. Windows and Linux native checks were not run. Native scrolling stitch
success was not retested; the overlap matcher is unchanged. Drag-out file export
still prepares its file synchronously when the native drag starts.

The signed debug app is running. Its executable SHA-256 is
`2bbc904ccd6c418e45dcf04d9801e9cf06008132d75cc9cf74ae077a5712d57e`.
Profiles, counts, and the native PNG export are under
`artifacts/responsiveness-2026-10-06/`.

## OS-specific release gates

### macOS

- Test first grant, denial, later grant, and revocation of Screen Recording permission.
- Test standard and Retina displays, Spaces/fullscreen behavior, menu-bar lifecycle, and Command-based editor shortcuts.
- Verify window shadow/transparency and pasteboard file/image representations in at least two native applications.

### Windows

- Test per-monitor DPI at 100%, 125%, 150%, and 200% across mixed displays where possible.
- Test Windows Graphics Capture/security boundaries, taskbar notification-area behavior, reserved shortcut conflicts, and clipboard ownership after Sniplet exits.
- Verify always-on-top and focus behavior across virtual desktops and a fullscreen application.

### Linux

- Record X11 and Wayland as separate targets; passing one does not cover the other.
- On X11, test monitor/window enumeration, global shortcuts, tray host behavior, clipboard persistence, and scroll injection.
- On Wayland, record the compositor and portal versions. Exercise portal selection/permission, window capture, global shortcut availability, tray implementation, and topmost/pinning behavior. Unsupported compositor capabilities must be surfaced to the user rather than silently reported as success.
- Confirm a working Vulkan driver in the graphical session. The Vulkan loader installed for compilation is not evidence that GPUI can present a window.

## Release evidence rule

A parity row may be marked **Verified** only with an automated test name or a desktop test record tied to a commit and environment. **Blocked** means an external platform capability prevented the test and includes the observed reason. **Unsupported** is an intentional product result with a user-visible explanation. Missing evidence remains **Not run**.
