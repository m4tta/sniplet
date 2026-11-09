# Testing on boat

Agents edit Sniplet on the local machine. Builds, tests, and the running app go to a
[boat.dev](https://docs.boat.dev) VM. Each T3 thread has its own VM, and
`scripts/boat.py` is the only way to use it.

## Rules

1. **Default to the VM.** Build, test, and run the app on this thread's VM. This covers
   `cargo build`, `cargo test`, `cargo check`, `cargo clippy`, `scripts/verify.sh`,
   `--self-test`, and all computer use on the app. Do not compile on the local machine.
   Local `git` and `cargo fmt` are fine.
2. **Another machine only when the user names it.** If the user says where to test (for
   example "test on my PC" or "use yarr-dev"), do that and do not start a VM.
3. **One VM per thread, one agent per VM.** The tool names the VM
   `sniplet-<T3CODE_THREAD_ID>` and touches only that VM. Never use `boat` on another
   thread's VM. Never stop, delete, or change a VM that this thread did not create.
4. **Only the thread's main agent runs `boat.py`.** Subagents in the same thread have the
   same thread ID, so they would share the VM. If you need a second copy of the app, start
   a new T3 thread. It gets its own VM.
5. **Stop the VM when the work is done:** `python scripts/boat.py down`.

## Commands

Run from the worktree root: `python scripts/boat.py <command>`. Use `python3` where
`python` does not exist.

| Command | What it does |
| --- | --- |
| `up` | Creates this thread's VM from the base snapshot, or resumes it. `push` and `sync` do this too. |
| `sync [-- ARGS]` | Starts the VM, pushes the worktree, builds `sniplet-app`, and restarts the app with `ARGS`. |
| `push` | Starts the VM and pushes the worktree, with no build. Use it before `exec` when you do not need the app. |
| `run [-- ARGS]` | Restarts the app with other arguments, with no push or build. |
| `shot [--region X Y W H]` | Saves a screenshot under `artifacts/boat/` and prints its path. |
| `click X Y [--right] [--double]` | Clicks at screenshot coordinates. |
| `drag X1 Y1 X2 Y2` | Presses the left button at the first point, moves, and releases at the second. |
| `move X Y` | Moves the pointer. |
| `type "TEXT"` | Types text into the focused control. |
| `key KEY...` | Presses keys in xdotool syntax: `ctrl+z`, `ctrl+shift+2`, `Escape`, `Return`. |
| `exec -- CMD` | Runs a shell command in the VM's repo folder (`/home/user/sniplet`). Prints its output and exits with its exit code. |
| `logs [-n N]` | Shows the end of the app log. |
| `desktop` | Prints a live desktop link for a person to watch. |
| `status` | Shows this thread's VM and its state. |
| `down` | Stops the VM. The disk and the warm build are kept for 14 days. |
| `base` | Rebuilds the shared base snapshot. See [The base snapshot](#the-base-snapshot). |

## Test the app

1. Run `sync`. The first time, it creates the VM from the base snapshot. Each time, it
   pushes the worktree (only files with changed content are rewritten, and files you
   deleted are deleted), builds the debug app, and restarts it. Run `sync` after every
   change that you want to see.
2. Run `shot` and read the PNG at the path it prints.
3. Act with `click`, `drag`, `type`, or `key`, with coordinates from the latest
   screenshot.
4. Run `shot` again and check the result. Repeat steps 3 and 4.
5. In your report, give the paths of the screenshots that prove the result.

**Coordinates.** The VM screen is 1920 × 1080. `shot` saves a 1280 × 720 image, and every
pointer command takes coordinates in that image. The tool scales them to the screen. To
read small text, use `shot --region X Y W H` (same coordinates) for a full-resolution
crop. Do not take click coordinates from a crop.

**Delays.** Some actions take time. For example, an area capture shows its overlay about
1 second after the hotkey. Wait before the next screenshot: `exec -- sleep 1`.

**Windows.** Starting a capture minimizes the editor, and a cancelled capture keeps it
minimized. This is intended behavior. To bring the editor back, run
`exec -- wmctrl -a Sniplet`. To list windows, run `exec -- wmctrl -l`.

**Hotkeys.** `key` sends keys through the X server like a real keyboard, so global
hotkeys work. `key ctrl+shift+2` starts an area capture.

**App arguments.** `sync -- --demo` or `run -- --settings` start the app with those
arguments.

**Speed (measured on 2026-11-09).** A screenshot takes about 1 second, and a pointer or
key command about 0.8 seconds. `sync` with no changes takes about 12 seconds, and so does
`sync` on a stopped VM. The first `sync` on a new VM takes 15 seconds plus the build. The
second `sync` on a new VM rebuilds some crates one time (about 30 seconds), because the
`sniplet-app` build script declares no inputs on Linux. This is expected.

## Run the automated checks

```sh
python scripts/boat.py push
python scripts/boat.py exec -- bash scripts/verify.sh all
python scripts/boat.py exec -- cargo test -p sniplet-core --locked
```

`verify.sh` accepts `core`, `workspace`, `ui`, and `all`. Add `--native` for the smoke run
and `--self-test`, which captures the real screen. Those runs start their own copies of
the app, so stop the running copy first: `exec -- pkill -x sniplet`.

## What the VM proves

The VM runs Ubuntu 24.04 with 8 vCPUs and 16 GB (the `large` size), Xorg on display `:0`
at 1920 × 1080, and openbox. It has no GPU. The app renders through Mesa's llvmpipe
software Vulkan driver.

On 2026-11-09 these passed on the VM: `verify.sh all`, the smoke run, `--self-test` with a
real 1920 × 1080 capture, pointer and keyboard input, and the global capture hotkey.

The VM does not prove Windows or macOS behavior, Wayland, mixed DPI, or GPU rendering.
Code under `cfg(windows)` or `cfg(target_os = "macos")` does not even compile there. The
tray was not checked. If your change touches any of these, say so in your report and ask
the user if they want a run on a machine that they name.

## Secrets

`boat.py` creates every VM with `--no-env`. Without that flag, boat puts the account's
secrets, including a GitHub token, into the VM, where the app and every dependency's build
script can read them. The VMs need no secrets. Do not create VMs for this work in another
way, and do not copy secrets into a VM.

## Live view for the user

`desktop` prints a link that contains an access token. Give it to the user when they want
to watch the app. Do not put it in commits, pull requests, or public comments.

## VM lifecycle and cost

- `up`, `push`, and `sync` create or resume the VM, and reset its auto-stop timer to
  2 hours. The other commands do not reset the timer.
- If a command says that the VM is stopped, run `sync`. A resume keeps the disk and the
  warm build, but not running processes, so the app must start again.
- A `large` VM costs $0.072 per hour while it runs. The $20 plan includes the hours.
- The plan allows 100 VMs at once, 60 starts per hour, and 12 starts per minute. If
  `boat` reports a rate limit, wait one minute and try again. Never stop another thread's
  VM to make room.

## The base snapshot

Every new thread VM starts from the saved snapshot `sniplet-base`. It contains the CI
Linux packages, the Mesa Vulkan driver, `rsync`, `CARGO_BUILD_JOBS=$(nproc)` in
`/etc/profile.d/sniplet.sh`, and the repo at `/home/user/sniplet` with a warm build of
`origin/main`: the debug app, the full verify suite, and a native smoke run.

If a thread's `Cargo.lock` is different from the base's, its first `sync` compiles the
changed dependencies. That takes minutes instead of seconds.

Run `python scripts/boat.py base` when `Cargo.lock` or the CI Linux packages change on
`main`, or when the user asks. It takes 5 to 8 minutes. It creates a temporary
`sniplet-base-build` VM, replaces the snapshot, and deletes the temporary VM. If a step
fails, it keeps the temporary VM for debugging, and that VM stops by itself after
2 hours. Existing thread VMs keep their own disks. Only one agent at a time may run
`base`.

The package list is `APT_PACKAGES` in `scripts/boat.py`. Keep it the same as the Linux
packages in `.github/workflows/ci.yml`.

## Set up a machine

1. Install the boat CLI.
   - Windows: download
     `https://boat.dev/api/boat/cli/download?platform=windows-x64&channel=prod` to
     `%USERPROFILE%\.ascii\bin\boat.exe`, and add that folder to the user `PATH`. The
     official installer does the same, but it also changes the PowerShell profile and
     starts a browser sign-in.
   - macOS and Linux: follow the [quickstart](https://docs.boat.dev/quickstart).
2. Sign in with `BOAT_API_KEY` from The Vault (Infisical, `dev` environment):

   ```sh
   infisical secrets get BOAT_API_KEY --projectId <The Vault project ID> --env dev --plain --silent | boat login --key-stdin
   ```

3. Check the setup: `boat limits` shows the plan, and `python scripts/boat.py status`
   shows this thread's VM.

## Troubleshooting

| Problem | Fix |
| --- | --- |
| `T3CODE_THREAD_ID is not set` | You are not in a T3 thread. Ask the user which machine to use. |
| `this thread's VM is stopped` | Run `sync`. |
| The app is not on the screen | Run `logs`, then `exec -- wmctrl -l`, then `run`. |
| The build fails on the VM but not locally | Look for Linux-only code or a missing package. |
| `the boat CLI is not installed` | See [Set up a machine](#set-up-a-machine). The tool also looks in `~/.ascii/bin`. |
