#!/usr/bin/env sh
set -eu

suite=all
suite_set=0
native=0
for argument in "$@"; do
    case "$argument" in
        --native)
            native=1
            ;;
        all|core|workspace|ui)
            if [ "$suite_set" -eq 1 ]; then
                echo "usage: $0 [all|core|workspace|ui] [--native]" >&2
                exit 2
            fi
            suite=$argument
            suite_set=1
            ;;
        *)
            echo "usage: $0 [all|core|workspace|ui] [--native]" >&2
            exit 2
            ;;
    esac
done

script_dir=$(dirname "$0")
workspace_root=$(CDPATH= cd "$script_dir/.." && pwd)
cd "$workspace_root"

run_cargo() {
    printf '\n> cargo'
    printf ' %s' "$@"
    printf '\n'
    cargo "$@"
}

bounded_run_count=0
run_bounded() {
    timeout_seconds=$1
    shift
    bounded_run_count=$((bounded_run_count + 1))
    timeout_marker="${TMPDIR:-/tmp}/sniplet-verify-timeout-$$-$bounded_run_count"
    rm -f "$timeout_marker"

    printf '\n>'
    printf ' %s' "$@"
    printf '\n'
    "$@" &
    command_pid=$!
    (
        sleep "$timeout_seconds"
        if kill -0 "$command_pid" 2>/dev/null; then
            : > "$timeout_marker"
            kill -TERM "$command_pid" 2>/dev/null || true
            sleep 2
            kill -KILL "$command_pid" 2>/dev/null || true
        fi
    ) &
    watchdog_pid=$!

    set +e
    wait "$command_pid"
    status=$?
    set -e
    kill "$watchdog_pid" 2>/dev/null || true
    wait "$watchdog_pid" 2>/dev/null || true

    if [ -f "$timeout_marker" ]; then
        rm -f "$timeout_marker"
        echo "command timed out after $timeout_seconds seconds" >&2
        return 124
    fi
    return "$status"
}

run_cargo fmt --all -- --check

case "$suite" in
    core)
        run_cargo check -p sniplet-core --all-targets --locked
        run_cargo test -p sniplet-core --all-targets --locked
        run_cargo clippy -p sniplet-core --all-targets --locked -- -D warnings
        ;;
    workspace)
        run_cargo check --workspace --all-targets --locked
        run_cargo test --workspace --all-targets --locked
        run_cargo clippy --workspace --all-targets --locked -- -D warnings
        ;;
    ui)
        run_cargo check -p sniplet-app --all-targets --features ui-tests --locked
        run_cargo test -p sniplet-app --all-targets --features ui-tests --locked
        run_cargo clippy -p sniplet-app --all-targets --features ui-tests --locked -- -D warnings
        ;;
    all)
        run_cargo check --workspace --all-targets --all-features --locked
        run_cargo build --workspace --all-targets --all-features --locked

        # Keep this separate so failures in the platform-free core have an
        # immediately reproducible command.
        run_cargo test -p sniplet-core --all-targets --locked
        run_cargo test --workspace --exclude sniplet-core --all-targets --locked
        run_cargo test -p sniplet-app --all-targets --features ui-tests --locked

        run_cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
        ;;
esac

if [ "$native" -eq 1 ]; then
    run_cargo build -p sniplet-app --locked
    target_dir=$(cargo metadata --format-version 1 --no-deps --locked \
        | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
    executable="$target_dir/debug/sniplet"
    if [ -z "$target_dir" ] || [ ! -x "$executable" ]; then
        echo "Sniplet executable not found at $executable" >&2
        exit 1
    fi

    run_bounded 20 "$executable" --demo --smoke --normal-window
    run_bounded 60 "$executable" --self-test "$workspace_root/artifacts/self-test"
fi
