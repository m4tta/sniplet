# Agent instructions

## Build, run, and test on boat

In a T3 Code thread (`T3CODE_THREAD_ID` is set), edit code on this machine, but build,
test, and run Sniplet on the thread's own boat.dev VM with `scripts/boat.py`, unless the
user names another machine for the work. Do not compile on this machine. Read
[BOAT.md](BOAT.md) before you build, test, or run the app.

## Website

The getsniplet.app website lives in `site/`. It is static HTML with no build step. Railway
deploys it from `main` when `site/` changes. It does not need boat, and CI and nightlies skip
site-only changes.
