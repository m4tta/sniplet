//! One Sniplet per user. The running Sniplet holds an OS lock on a file in a per-user folder,
//! which the OS releases when the process ends, even in a crash. It also listens on a local
//! socket: a named pipe on Windows, a Unix socket in the same folder elsewhere. A later launch
//! sends its arguments there, waits for the running Sniplet to take them, and exits.
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::Duration,
};

use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream, prelude::*};

/// How long a later launch waits for the running Sniplet to answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

pub enum Claim {
    /// This process is the only Sniplet. Pass it to `serve`.
    First(Instance),
    /// A running Sniplet took the arguments. This process must exit.
    Forwarded,
}

/// The lock and the socket of the running Sniplet. Both last as long as the process.
pub struct Instance {
    lock: File,
    listener: Listener,
}

/// Become the running Sniplet, or hand `args` to the one that runs.
pub fn claim(args: &[String]) -> io::Result<Claim> {
    claim_in(&folder()?, args)
}

/// Accept arguments from later launches on background threads. A connection that never sends
/// holds only its own thread.
pub fn serve(instance: Instance, on_args: impl Fn(Vec<String>) + Send + Sync + 'static) {
    let on_args = Arc::new(on_args);
    std::thread::spawn(move || {
        let _lock = instance.lock;
        for stream in instance.listener.incoming().filter_map(Result::ok) {
            let on_args = on_args.clone();
            std::thread::spawn(move || {
                if let Ok(args) = receive(stream) {
                    on_args(args);
                }
            });
        }
    });
}

fn claim_in(folder: &Path, args: &[String]) -> io::Result<Claim> {
    let lock = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(folder.join("sniplet.lock"))?;
    let args = absolute(args);
    // The lock holder may still be starting its listener, so keep trying for a short time.
    // If it quits meanwhile, its lock is free on the next try.
    for _ in 0..40 {
        if lock.try_lock().is_ok() {
            // Only the lock holder gets here, so a socket file left now is from a crash.
            #[cfg(unix)]
            let _ = std::fs::remove_file(folder.join("sniplet.sock"));
            let listener = ListenerOptions::new().name(name(folder)?).create_sync()?;
            return Ok(Claim::First(Instance { lock, listener }));
        }
        if let Ok(stream) = Stream::connect(name(folder)?) {
            match send(stream, args.clone()) {
                Ok(()) => return Ok(Claim::Forwarded),
                // The running Sniplet is alive but does not answer. Do not wait for it again.
                Err(error) if error.kind() == io::ErrorKind::TimedOut => return Err(error),
                Err(_) => {}
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(io::Error::other("the running Sniplet did not answer"))
}

/// The running Sniplet has another working folder, so send absolute file paths.
fn absolute(args: &[String]) -> Vec<String> {
    args.iter()
        .map(|arg| {
            if arg.starts_with('-') {
                return arg.clone();
            }
            std::path::absolute(arg)
                .map_or_else(|_| arg.clone(), |path| path.to_string_lossy().into_owned())
        })
        .collect()
}

fn send(stream: Stream, args: Vec<String>) -> io::Result<()> {
    // The running instance comes to the front for this launch, so let it take focus.
    #[cfg(target_os = "windows")]
    // SAFETY: plain Win32 call with no pointers.
    let _ = unsafe {
        windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
            windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
        )
    };
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(exchange(stream, &args));
    });
    result
        .recv_timeout(REPLY_TIMEOUT)
        .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()))
}

/// Send one JSON line, and wait for "ok", so this process exits only after the running
/// instance has the work.
fn exchange(stream: Stream, args: &[String]) -> io::Result<()> {
    let mut stream = BufReader::new(stream);
    let mut line = serde_json::to_string(args)?;
    line.push('\n');
    stream.get_mut().write_all(line.as_bytes())?;
    let mut reply = String::new();
    stream.read_line(&mut reply)?;
    match reply.trim() {
        "ok" => Ok(()),
        _ => Err(io::Error::other("the running Sniplet did not answer")),
    }
}

fn receive(stream: Stream) -> io::Result<Vec<String>> {
    let mut stream = BufReader::new(stream);
    let mut line = String::new();
    stream.read_line(&mut line)?;
    let args = serde_json::from_str(&line)?;
    stream.get_mut().write_all(b"ok\n")?;
    Ok(args)
}

/// Named pipes live in one machine-wide namespace. The per-user folder makes the name unique.
#[cfg(target_os = "windows")]
fn name(folder: &Path) -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericNamespaced;
    let folder = folder.to_string_lossy().replace(['\\', '/', ':'], "-");
    format!("sniplet{folder}").to_ns_name::<GenericNamespaced>()
}

#[cfg(unix)]
fn name(folder: &Path) -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericFilePath;
    folder.join("sniplet.sock").to_fs_name::<GenericFilePath>()
}

/// A folder that only this user can write: the user's temporary folder on Windows and macOS,
/// and the session's runtime folder on Linux.
fn folder() -> io::Result<PathBuf> {
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(runtime));
    }
    if cfg!(any(target_os = "windows", target_os = "macos")) {
        return Ok(std::env::temp_dir());
    }
    // Linux without a runtime folder: `/tmp` is shared, so use a private folder in the cache.
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .ok_or_else(|| io::Error::other("no home folder"))?;
    let folder = cache.join("sniplet");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(&folder)?;
    Ok(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_hands_its_arguments_to_the_first() {
        let folder = tempfile::tempdir().unwrap();
        let Claim::First(instance) = claim_in(folder.path(), &[]).unwrap() else {
            panic!("the first claim must own the lock");
        };
        let (sender, received) = mpsc::channel();
        serve(instance, move |args| sender.send(args).unwrap());

        let args = vec!["--settings".to_owned(), "shot.png".to_owned()];
        assert!(matches!(
            claim_in(folder.path(), &args).unwrap(),
            Claim::Forwarded
        ));
        let got = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got[0], "--settings");
        assert!(Path::new(&got[1]).is_absolute());
        assert!(got[1].ends_with("shot.png"));
    }

    #[test]
    fn a_silent_connection_does_not_block_later_launches() {
        let folder = tempfile::tempdir().unwrap();
        let Claim::First(instance) = claim_in(folder.path(), &[]).unwrap() else {
            panic!("the first claim must own the lock");
        };
        let (sender, received) = mpsc::channel();
        serve(instance, move |args| sender.send(args).unwrap());

        let _silent = Stream::connect(name(folder.path()).unwrap()).unwrap();
        let args = vec!["--demo".to_owned()];
        assert!(matches!(
            claim_in(folder.path(), &args).unwrap(),
            Claim::Forwarded
        ));
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).unwrap(), args);
    }
}
