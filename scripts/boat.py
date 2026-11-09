#!/usr/bin/env python3
"""Build, run and test Sniplet on this T3 thread's own boat.dev VM.

Every T3 thread gets exactly one VM, named after T3CODE_THREAD_ID. A thread never
touches another thread's VM. BOAT.md describes the workflow:

    python scripts/boat.py sync                  # push this worktree, build, start the app
    python scripts/boat.py shot                  # screenshot -> artifacts/boat/*.png
    python scripts/boat.py click 640 400         # screenshot coordinates
    python scripts/boat.py exec -- bash scripts/verify.sh all
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SHOT_DIR = ROOT / "artifacts" / "boat"

# Every thread VM starts from this saved snapshot. `base` rebuilds it from origin/main.
BASE_SNAPSHOT = "sniplet-base"
BASE_BUILDER = "sniplet-base-build"
VM_TYPE = os.environ.get("SNIPLET_BOAT_TYPE", "large")  # 8 vCPU, 16 GB
TTL_SECONDS = 2 * 60 * 60  # An idle VM stops 2 hours after the last `up`, `push` or `sync`.

REPO = "/home/user/sniplet"  # The same path on every VM keeps the base snapshot's build warm.
STAGE = "/home/user/.sniplet-push"
APP_LOG = "/home/user/sniplet.log"
REMOTE_SHOT = "/tmp/sniplet-shot.png"
# Screenshots are scaled to this width, and every pointer command takes coordinates in
# that scaled image. One coordinate space means a model never clicks in the wrong scale.
SHOT_WIDTH = 1280

ACTIVE_STATES = {"provisioned", "cloning", "ready", "idle", "running"}

# Every VM is created with --no-env. Otherwise boat puts the account's secrets, including
# a GitHub token, into the VM, where the app and every dependency's build script can read
# them. The VMs need no secrets.

# Shell setup for every VM command, in /etc/profile.d/sniplet.sh. The repo's .cargo/config.toml
# caps builds at 2 jobs for desktops; a VM uses every CPU. ~/.cache is not in snapshots, so
# cargo-xwin keeps the Windows SDK in another folder.
PROFILE = 'export CARGO_BUILD_JOBS="$(nproc)"\nexport XWIN_CACHE_DIR=/home/user/.xwin-cache\n'

# Compile-and-lint checks for platforms the VM cannot run. They mirror the clippy step in
# .github/workflows/ci.yml. Nothing is linked or run.
CARGO_XWIN_VERSION = "0.23.1"
CROSS_TARGETS = ["x86_64-pc-windows-msvc"]
CROSS_CHECKS = {
    "windows": "cargo xwin clippy --target x86_64-pc-windows-msvc "
    "--workspace --all-targets --all-features --locked -- -D warnings",
}

# Keep in sync with the Linux packages in .github/workflows/ci.yml.
APT_PACKAGES = (
    "gcc g++ clang cmake pkg-config libclang-dev libfontconfig-dev libwayland-dev "
    "libwebkit2gtk-4.1-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev libxrandr-dev "
    "libdbus-1-dev libpipewire-0.3-dev libegl-dev libgbm-dev libssl-dev libzstd-dev "
    "vulkan-validationlayers libvulkan1 mesa-vulkan-drivers vulkan-tools xvfb xauth rsync"
)


def die(message: str, code: int = 1) -> None:
    print(f"boat.py: {message}", file=sys.stderr)
    sys.exit(code)


def boat_exe() -> str:
    found = shutil.which("boat")
    if found:
        return found
    fallback = Path.home() / ".ascii" / "bin" / ("boat.exe" if os.name == "nt" else "boat")
    if fallback.exists():
        return str(fallback)
    die("the boat CLI is not installed. See 'Set up a machine' in BOAT.md.")
    raise AssertionError


def json_lines(text: str) -> list[dict]:
    frames = []
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                frames.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return frames


def boat(*args: str, check: bool = True, cwd: Path | None = None) -> list[dict]:
    """Runs `boat <args>` with JSON output and returns its JSON lines."""
    command = [boat_exe(), args[0], "--json", *args[1:]]
    proc = subprocess.run(
        command, capture_output=True, text=True, encoding="utf-8", errors="replace", cwd=cwd
    )
    frames = json_lines(proc.stdout)
    if check and proc.returncode != 0:
        error = next((f.get("error") for f in frames if f.get("event") == "error"), None)
        die(f"boat {args[0]} failed: {error or proc.stderr.strip() or proc.stdout.strip()}")
    return frames


def run(vm: str, command: str, timeout: int = 60) -> tuple[int, str, str]:
    """Runs a short shell command on the VM and returns (exit code, stdout, stderr)."""
    frames = boat("exec", vm, "--timeout", str(timeout), "--", command, check=False)
    result = next((f for f in frames if "exitCode" in f), None)
    if result is None:
        error = next((f.get("error") for f in frames if f.get("error")), "no result")
        die(f"remote command failed: {error}")
    return result["exitCode"], result.get("stdout", ""), result.get("stderr", "")


def run_checked(vm: str, command: str, timeout: int = 60) -> str:
    code, out, err = run(vm, command, timeout)
    if code != 0:
        die(f"remote command exited {code}: {command}\n{out}{err}".rstrip())
    return out


def stream(vm: str, command: str) -> int:
    """Runs a long shell command on the VM, prints its output live, and returns its exit code."""
    proc = subprocess.Popen(
        [boat_exe(), "exec", "--json", "--stream", "--timeout", "0", vm, "--", command],
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    assert proc.stdout is not None
    code = None
    for line in proc.stdout:
        for frame in json_lines(line):
            if frame.get("type") == "stdout":
                sys.stdout.write(frame.get("data", ""))
                sys.stdout.flush()
            elif frame.get("type") == "stderr":
                sys.stderr.write(frame.get("data", ""))
                sys.stderr.flush()
            elif frame.get("type") == "exit":
                code = frame.get("exitCode")
            elif frame.get("event") == "error":
                print(f"boat exec: {frame.get('error')}", file=sys.stderr)
    proc.wait()
    return code if code is not None else (proc.returncode or 1)


# --- This thread's VM -----------------------------------------------------------------


def vm_name() -> str:
    thread = os.environ.get("T3CODE_THREAD_ID")
    if not thread:
        die("T3CODE_THREAD_ID is not set. Run this from a T3 Code thread; each thread owns one VM.")
    return f"sniplet-{thread}"


def find(name: str) -> dict | None:
    sandboxes = [
        s
        for frame in boat("list", "--all")
        for s in frame.get("sandboxes", [])
        if s.get("name") == name
    ]
    return max(sandboxes, key=lambda s: s.get("createdAt", ""), default=None)


def state_of(vm: str) -> str:
    for frame in boat("list", "--all"):
        for s in frame.get("sandboxes", []):
            if s.get("id") == vm:
                return s.get("state", "unknown")
    return "missing"


def wait_until_stopped(vm: str) -> None:
    for _ in range(200):
        if state_of(vm) not in ACTIVE_STATES | {"stopping"}:
            return
        time.sleep(3)
    die(f"{vm} is still stopping after 10 minutes.")


def ready_id(frames: list[dict]) -> str:
    vm = next((f["id"] for f in frames if f.get("event") == "ready"), None)
    if not vm:
        die(f"the VM did not become ready: {frames[-1] if frames else 'no output'}")
    return vm


def up() -> str:
    """Starts this thread's VM: creates it from the base snapshot, or resumes it."""
    name = vm_name()
    vm = find(name)
    if vm is None:
        print(f"Creating {name} from the {BASE_SNAPSHOT} snapshot...", flush=True)
        frames = boat(
            "new", "--from", BASE_SNAPSHOT, "--type", VM_TYPE, "--no-env",
            "--ttl", str(TTL_SECONDS), "--name", name,
        )
        return ready_id(frames)
    if vm["state"] == "stopping":
        wait_until_stopped(vm["id"])
        vm["state"] = "stopped"
    if vm["state"] in ACTIVE_STATES:
        boat("extend", vm["id"], "--ttl", str(TTL_SECONDS))
    else:
        print(f"Resuming {vm['id']} ({vm['state']})...", flush=True)
        boat("resume", vm["id"], "--ttl", str(TTL_SECONDS))
    return vm["id"]


def running_vm() -> str:
    """Returns this thread's VM ID if it is running. It never starts a VM by itself."""
    vm = find(vm_name())
    if vm is None:
        die("this thread has no VM yet. Run: python scripts/boat.py sync")
    if vm["state"] not in ACTIVE_STATES:
        die(f"this thread's VM is {vm['state']}. Run: python scripts/boat.py sync")
    return vm["id"]


def push(vm: str) -> None:
    """Copies this worktree to the VM. Only files whose content changed are rewritten,
    so cargo rebuilds only what changed. Files deleted here are deleted there."""
    print("Pushing the worktree...", flush=True)
    run_checked(vm, f"rm -rf {STAGE}")
    boat("push", vm, str(ROOT), "--to", STAGE)
    run_checked(
        vm,
        f"rsync -rl --checksum --delete --exclude=/target/ --exclude=/artifacts/ --exclude=/.git "
        f"{STAGE}/ {REPO}/ && rm -rf {STAGE}",
        timeout=300,
    )


def start_app(vm: str, app_args: list[str]) -> None:
    args = shlex.join(app_args)
    out = run_checked(
        vm,
        f"pkill -x sniplet; for i in $(seq 50); do pgrep -x sniplet >/dev/null || break; sleep 0.1; done; "
        f"cd {REPO} && (setsid nohup target/debug/sniplet {args} >{APP_LOG} 2>&1 </dev/null &); "
        f"sleep 3; if pgrep -x sniplet >/dev/null; then echo 'Sniplet is running.'; "
        f"else echo 'Sniplet exited. Log:'; tail -n 40 {APP_LOG}; exit 1; fi",
    )
    print(out.rstrip())


# --- Commands -------------------------------------------------------------------------


def cmd_up(_: argparse.Namespace) -> None:
    print(f"VM {up()} is running.")


def cmd_push(_: argparse.Namespace) -> None:
    vm = up()
    push(vm)
    print(f"Pushed to {vm}:{REPO}")


def cmd_sync(args: argparse.Namespace) -> None:
    vm = up()
    push(vm)
    print("Building sniplet-app...", flush=True)
    code = stream(vm, f"cd {REPO} && cargo build -p sniplet-app --locked")
    if code != 0:
        die("the build failed.", code)
    start_app(vm, args.app_args)


def cmd_run(args: argparse.Namespace) -> None:
    start_app(running_vm(), args.app_args)


def cmd_shot(args: argparse.Namespace) -> None:
    vm = running_vm()
    if args.region:
        # A full-resolution crop for reading detail. Pointer commands still use full-shot coordinates.
        x, y, w, h = args.region
        capture = (
            f"read W H < <(xdotool getdisplaygeometry); s() {{ echo $(( $1 * W / {SHOT_WIDTH} )); }}; "
            f"import -window root -crop $(s {w})x$(s {h})+$(s {x})+$(s {y}) +repage {REMOTE_SHOT}"
        )
    else:
        capture = f"import -window root -resize {SHOT_WIDTH}x {REMOTE_SHOT}"
    # Base64 over exec takes about 1 s; `boat scp` takes about 10 s.
    png = base64.b64decode(run_checked(vm, f"{capture} && base64 -w0 {REMOTE_SHOT}"))
    SHOT_DIR.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S") + f"-{int(time.time() * 1000) % 1000:03d}"
    path = SHOT_DIR / f"shot-{stamp}.png"
    path.write_bytes(png)
    print(path)


def pointer(vm: str, script: str) -> None:
    """Runs xdotool commands. `p X Y` converts screenshot coordinates to screen pixels."""
    run_checked(
        vm,
        f"read W H < <(xdotool getdisplaygeometry); "
        f"p() {{ echo $(( $1 * W / {SHOT_WIDTH} )) $(( $2 * W / {SHOT_WIDTH} )); }}; {script}",
    )


def cmd_click(args: argparse.Namespace) -> None:
    button = 3 if args.right else 1
    repeat = "--repeat 2 --delay 80 " if args.double else ""
    pointer(running_vm(), f"xdotool mousemove $(p {args.x} {args.y}) click {repeat}{button}")


def cmd_move(args: argparse.Namespace) -> None:
    pointer(running_vm(), f"xdotool mousemove $(p {args.x} {args.y})")


def cmd_drag(args: argparse.Namespace) -> None:
    x1, y1, x2, y2 = args.x1, args.y1, args.x2, args.y2
    steps = 20
    moves = " ".join(
        f"mousemove $(p {x1 + (x2 - x1) * i // steps} {y1 + (y2 - y1) * i // steps}) sleep 0.02"
        for i in range(1, steps + 1)
    )
    pointer(running_vm(), f"xdotool mousemove $(p {x1} {y1}) mousedown 1 sleep 0.05 {moves} mouseup 1")


def cmd_type(args: argparse.Namespace) -> None:
    run_checked(running_vm(), f"xdotool type --delay 12 -- {shlex.quote(args.text)}")


def cmd_key(args: argparse.Namespace) -> None:
    run_checked(running_vm(), f"xdotool key --delay 60 -- {shlex.join(args.keys)}")


def cmd_logs(args: argparse.Namespace) -> None:
    print(run_checked(running_vm(), f"tail -n {args.lines} {APP_LOG}").rstrip())


def cmd_exec(args: argparse.Namespace) -> None:
    if not args.command:
        die("give a command after --, for example: exec -- cargo test -p sniplet-core")
    # One argument is a shell command line; several are quoted as separate words.
    command = args.command[0] if len(args.command) == 1 else shlex.join(args.command)
    sys.exit(stream(running_vm(), f"cd {REPO} && {command}"))


def cmd_check(args: argparse.Namespace) -> None:
    vm = up()
    push(vm)
    print(f"Checking the {args.platform} build (compile and clippy only)...", flush=True)
    sys.exit(stream(vm, f"cd {REPO} && {CROSS_CHECKS[args.platform]}"))


def cmd_desktop(_: argparse.Namespace) -> None:
    frames = boat("desktop", running_vm())
    print(next((f["viewerUrl"] for f in frames if f.get("viewerUrl")), frames))


def cmd_status(_: argparse.Namespace) -> None:
    name = vm_name()
    vm = find(name)
    if vm is None:
        print(f"{name}: no VM yet")
        return
    print(
        f"{name}: {vm['id']} {vm['state']}, {vm.get('type')} "
        f"({vm.get('vcpu')} vCPU, {vm.get('memoryGB')} GB), stops after {vm.get('archiveAfter')}"
    )


def cmd_down(_: argparse.Namespace) -> None:
    vm = find(vm_name())
    if vm is None or vm["state"] not in ACTIVE_STATES:
        print("This thread's VM is not running.")
        return
    boat("stop", vm["id"])
    print(f"Stopping {vm['id']}. Its disk is kept for 14 days; `sync` resumes it.")


def cmd_base(args: argparse.Namespace) -> None:
    """Rebuilds the sniplet-base snapshot from a commit (origin/main by default).
    Existing thread VMs keep their disks."""
    subprocess.run(["git", "fetch", "-q", "origin", "main"], cwd=ROOT, check=True)
    rev = subprocess.run(
        ["git", "rev-parse", "--short", args.ref], cwd=ROOT, check=True,
        capture_output=True, text=True,
    ).stdout.strip()
    print(f"Building {BASE_SNAPSHOT} from {args.ref} ({rev})...", flush=True)
    vm = ready_id(boat(
        "new", "--type", VM_TYPE, "--no-env", "--ttl", str(TTL_SECONDS), "--name", BASE_BUILDER,
    ))

    def step(command: str) -> None:
        if stream(vm, command) != 0:
            die(f"a base step failed. Builder VM {vm} is kept for debugging and stops by itself.")

    step(
        "export DEBIAN_FRONTEND=noninteractive; sudo apt-get update -qq && "
        f"sudo -E apt-get install -y -qq --no-install-recommends {APT_PACKAGES} >/tmp/apt.log 2>&1 "
        "|| { tail -n 40 /tmp/apt.log; exit 1; }"
    )
    step(f"printf %s {shlex.quote(PROFILE)} | sudo tee /etc/profile.d/sniplet.sh >/dev/null")
    # cargo-xwin downloads Microsoft's CRT and Windows SDK into XWIN_CACHE_DIR on first use.
    step(
        f"rustup target add {' '.join(CROSS_TARGETS)} && "
        f"cargo install --locked cargo-xwin@{CARGO_XWIN_VERSION}"
    )
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run(
            ["git", "archive", "--format=tar.gz", "-o", str(Path(tmp) / "src.tgz"), args.ref],
            cwd=ROOT, check=True,
        )
        boat("scp", "src.tgz", f"{vm}:/home/user/src.tgz", cwd=Path(tmp))
    step(f"rm -rf {REPO} && mkdir -p {REPO} && tar xzf ~/src.tgz -C {REPO} && rm ~/src.tgz")
    # Warm every build that threads use: the app, the full verify suite, a native smoke run,
    # and every cross-platform check.
    step(f"cd {REPO} && cargo build -p sniplet-app --locked && bash scripts/verify.sh all --native")
    for command in CROSS_CHECKS.values():
        step(f"cd {REPO} && {command}")
    step(f"rm -rf {REPO}/artifacts")
    # Stop first, so the snapshot holds every write.
    boat("stop", vm)
    wait_until_stopped(vm)
    boat("snapshot", vm, BASE_SNAPSHOT)  # Replaces the old snapshot with this name.
    boat("delete", vm, "--yes")
    print(f"Saved {BASE_SNAPSHOT} from {args.ref} ({rev}).")


def main() -> None:
    for out in (sys.stdout, sys.stderr):
        out.reconfigure(errors="replace")  # Windows consoles cannot print every character.

    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="subcommand", required=True)

    def add(name: str, func, help_text: str) -> argparse.ArgumentParser:
        p = sub.add_parser(name, help=help_text)
        p.set_defaults(func=func)
        return p

    add("up", cmd_up, "start this thread's VM (create it from the base, or resume it)")
    add("push", cmd_push, "start the VM and copy this worktree to it, with no build")
    p = add("sync", cmd_sync, "push, build sniplet-app, and restart the app")
    p.add_argument("app_args", nargs=argparse.REMAINDER, help="app arguments, after --")
    p = add("run", cmd_run, "restart the app with other arguments, with no push or build")
    p.add_argument("app_args", nargs=argparse.REMAINDER, help="app arguments, after --")
    p = add("shot", cmd_shot, f"screenshot, {SHOT_WIDTH} px wide, saved under artifacts/boat/")
    p.add_argument("--region", nargs=4, type=int, metavar=("X", "Y", "W", "H"),
                   help="save a full-resolution crop of this region (screenshot coordinates)")
    p = add("click", cmd_click, "click at screenshot coordinates")
    p.add_argument("x", type=int)
    p.add_argument("y", type=int)
    p.add_argument("--right", action="store_true")
    p.add_argument("--double", action="store_true")
    p = add("move", cmd_move, "move the pointer to screenshot coordinates")
    p.add_argument("x", type=int)
    p.add_argument("y", type=int)
    p = add("drag", cmd_drag, "press, move, and release the left button")
    for axis in ("x1", "y1", "x2", "y2"):
        p.add_argument(axis, type=int)
    p = add("type", cmd_type, "type text")
    p.add_argument("text")
    p = add("key", cmd_key, "press keys, for example ctrl+z or ctrl+shift+1 Escape")
    p.add_argument("keys", nargs="+")
    p = add("logs", cmd_logs, "show the end of the app log")
    p.add_argument("-n", "--lines", type=int, default=60)
    p = add("exec", cmd_exec, "run a command in the VM's repo folder, with its exit code")
    p.add_argument("command", nargs=argparse.REMAINDER)
    p = add("check", cmd_check, "push, then compile and lint for a platform the VM cannot run")
    p.add_argument("platform", choices=sorted(CROSS_CHECKS))
    add("desktop", cmd_desktop, "print a live desktop link for a person to watch")
    add("status", cmd_status, "show this thread's VM")
    add("down", cmd_down, "stop this thread's VM (the disk is kept for 14 days)")
    p = add("base", cmd_base, "rebuild the shared sniplet-base snapshot from origin/main")
    p.add_argument("--ref", default="origin/main", help="commit to build from (default: origin/main)")

    args = parser.parse_args()
    for attr in ("app_args", "command"):
        value = getattr(args, attr, None)
        if isinstance(value, list) and value[:1] == ["--"]:
            setattr(args, attr, value[1:])
    args.func(args)


if __name__ == "__main__":
    main()
