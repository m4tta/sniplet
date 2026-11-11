//! One Sniplet per user. The first launch listens on a local socket: a named pipe on Windows,
//! a Unix socket in a per-user folder elsewhere. A later launch sends its arguments there,
//! waits for the first instance to take them, and exits. The socket is also the lock, so a
//! crashed instance leaves nothing that blocks the next launch.
use std::{
    io::{self, BufRead, BufReader, Write},
    time::Duration,
};

use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream, prelude::*};

pub enum Claim {
    /// This process is the only Sniplet. Pass the listener to `serve`.
    First(Listener),
    /// A running Sniplet took the arguments. This process must exit.
    Forwarded,
}

/// Become the running Sniplet, or hand `args` to the one that runs.
pub fn claim(args: &[String]) -> io::Result<Claim> {
    claim_name(name()?, args)
}

/// Accept arguments from later launches on a background thread, in the order they arrive.
pub fn serve(listener: Listener, mut on_args: impl FnMut(Vec<String>) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().filter_map(Result::ok) {
            if let Ok(args) = receive(stream) {
                on_args(args);
            }
        }
    });
}

fn claim_name(name: Name<'static>, args: &[String]) -> io::Result<Claim> {
    let mut last_error = None;
    // Two launches at the same time can both miss a listener and race to create one. The loser
    // gets an error, retries, and connects to the winner.
    for attempt in 0..10 {
        if let Ok(stream) = Stream::connect(name.borrow()) {
            send(stream, args)?;
            return Ok(Claim::Forwarded);
        }
        match ListenerOptions::new().name(name.borrow()).create_sync() {
            Ok(listener) => return Ok(Claim::First(listener)),
            Err(error) => {
                // A socket file that refuses connections twice is left over from a crash.
                #[cfg(unix)]
                if attempt == 1 && error.kind() == io::ErrorKind::AddrInUse {
                    let _ = std::fs::remove_file(socket_path());
                }
                let _ = attempt;
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| io::Error::other("no instance lock")))
}

fn send(stream: Stream, args: &[String]) -> io::Result<()> {
    // The running instance comes to the front for this launch, so let it take focus.
    #[cfg(target_os = "windows")]
    // SAFETY: plain Win32 call with no pointers.
    let _ = unsafe {
        windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(
            windows::Win32::UI::WindowsAndMessaging::ASFW_ANY,
        )
    };
    // The running instance has another working folder, so send absolute file paths.
    let args: Vec<String> = args
        .iter()
        .map(|arg| {
            if arg.starts_with('-') {
                return arg.clone();
            }
            std::path::absolute(arg)
                .map_or_else(|_| arg.clone(), |path| path.to_string_lossy().into_owned())
        })
        .collect();
    let mut stream = BufReader::new(stream);
    let mut line = serde_json::to_string(&args)?;
    line.push('\n');
    stream.get_mut().write_all(line.as_bytes())?;
    // Wait for the reply, so this process exits only after the running instance has the work.
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

fn user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

/// Named pipes live in one machine-wide namespace, so the name carries the user.
#[cfg(target_os = "windows")]
fn name() -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericNamespaced;
    format!("sniplet-{}", user()).to_ns_name::<GenericNamespaced>()
}

/// A file in a folder only this user can write: the runtime folder on Linux, the per-user
/// temporary folder on macOS.
#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
        .join(format!("sniplet-{}.sock", user()))
}

#[cfg(unix)]
fn name() -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericFilePath;
    socket_path().to_fs_name::<GenericFilePath>()
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn a_second_launch_hands_its_arguments_to_the_first() {
        let name = || test_name(&format!("sniplet-test-{}", std::process::id()));
        let Claim::First(listener) = claim_name(name(), &[]).unwrap() else {
            panic!("the first claim must own the lock");
        };
        let (sender, received) = mpsc::channel();
        serve(listener, move |args| sender.send(args).unwrap());

        let args = vec!["--settings".to_owned(), "shot.png".to_owned()];
        assert!(matches!(
            claim_name(name(), &args).unwrap(),
            Claim::Forwarded
        ));
        let got = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got[0], "--settings");
        assert!(std::path::Path::new(&got[1]).is_absolute());
        assert!(got[1].ends_with("shot.png"));
    }

    #[cfg(target_os = "windows")]
    fn test_name(id: &str) -> Name<'static> {
        use interprocess::local_socket::GenericNamespaced;
        id.to_owned().to_ns_name::<GenericNamespaced>().unwrap()
    }

    #[cfg(unix)]
    fn test_name(id: &str) -> Name<'static> {
        use interprocess::local_socket::GenericFilePath;
        std::env::temp_dir()
            .join(format!("{id}.sock"))
            .to_fs_name::<GenericFilePath>()
            .unwrap()
    }
}
