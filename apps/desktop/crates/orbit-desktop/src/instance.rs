//! One app per person: a second start brings the running window to the front and ends, so there
//! is never a second window or tray icon.
//!
//! The first start holds a lock on `<data dir>/orbit-desktop/instance.lock` (the OS frees it when
//! the process ends, also on a crash) and listens on a loopback port it writes to `instance.port`.
//! A later start that cannot take the lock sends `show` to that port. A `--hidden` start (the
//! autostart entry) sends nothing: the app already runs, and autostart must not open the window.
//! Loopback only, and the only request shows the window, so the port needs no secret.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SHOW: &str = "show";
/// How long a second start waits for the first one to write its port (it may be starting too).
const FORWARD_WAIT: Duration = Duration::from_secs(5);

pub enum Start {
    /// This is the only app: keep the instance (dropping it frees the lock) and serve it.
    First(Instance),
    /// Another app runs; it was asked to show its window unless this start is hidden.
    Running,
}

pub struct Instance {
    _lock: File,
    listener: TcpListener,
}

pub fn dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("orbit-desktop"))
}

/// Takes the lock in `dir`, or hands this start to the app that has it.
pub fn start(dir: &Path, hidden: bool) -> io::Result<Start> {
    fs::create_dir_all(dir)?;
    let lock = OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("instance.lock"))?;
    match lock.try_lock() {
        Ok(()) => {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            fs::write(port_path(dir), listener.local_addr()?.port().to_string())?;
            Ok(Start::First(Instance { _lock: lock, listener }))
        }
        Err(TryLockError::WouldBlock) => {
            if !hidden {
                forward(dir)?;
            }
            Ok(Start::Running)
        }
        Err(TryLockError::Error(e)) => Err(e),
    }
}

impl Instance {
    /// Calls `show` on a background thread for each `show` request from a later start.
    pub fn serve(&self, show: impl Fn() + Send + 'static) -> io::Result<()> {
        let listener = self.listener.try_clone()?;
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if read_request(stream) {
                    show();
                }
            }
        });
        Ok(())
    }
}

fn port_path(dir: &Path) -> PathBuf {
    dir.join("instance.port")
}

/// Sends `show` to the running app, waiting while it is still starting.
fn forward(dir: &Path) -> io::Result<()> {
    // The running app may bring its window to the front: this start came from the person.
    #[cfg(windows)]
    // SAFETY: a plain Win32 call without pointers.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(windows_sys::Win32::UI::WindowsAndMessaging::ASFW_ANY);
    }
    let deadline = Instant::now() + FORWARD_WAIT;
    loop {
        let sent = fs::read_to_string(port_path(dir))
            .ok()
            .and_then(|text| text.trim().parse::<u16>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no port yet"))
            .and_then(|port| TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(1)))
            .and_then(|mut stream| stream.write_all(format!("{SHOW}\n").as_bytes()));
        match sent {
            Ok(()) => return Ok(()),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// One short line; anything else (or nothing within a second) is not a request.
fn read_request(stream: TcpStream) -> bool {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let mut line = String::new();
    let _ = BufReader::new(stream.take(64)).read_line(&mut line);
    line.trim_end() == SHOW
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn first(dir: &Path) -> (Instance, mpsc::Receiver<()>) {
        let Ok(Start::First(instance)) = start(dir, false) else { panic!("the first start holds the lock") };
        let (tx, rx) = mpsc::channel();
        instance.serve(move || tx.send(()).expect("the test waits")).expect("serves");
        (instance, rx)
    }

    #[test]
    fn a_second_start_shows_the_running_window_and_a_hidden_one_does_not() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (instance, shown) = first(dir.path());

        assert!(matches!(start(dir.path(), false), Ok(Start::Running)));
        shown.recv_timeout(Duration::from_secs(5)).expect("the running app is asked to show its window");

        assert!(matches!(start(dir.path(), true), Ok(Start::Running)), "a hidden start never opens a second app");
        assert!(shown.recv_timeout(Duration::from_millis(300)).is_err(), "autostart does not bring the window up");

        drop(instance);
        let (_again, _) = first(dir.path());
    }

    #[test]
    fn only_a_show_line_counts() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (instance, shown) = first(dir.path());
        let port = instance.listener.local_addr().expect("bound").port();
        for junk in ["", "hide\n", "show me\n", &"show".repeat(40)] {
            let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
            stream.write_all(junk.as_bytes()).expect("writes");
            drop(stream);
        }
        assert!(shown.recv_timeout(Duration::from_millis(300)).is_err());
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).expect("connects");
        stream.write_all(b"show\r\n").expect("writes");
        shown.recv_timeout(Duration::from_secs(5)).expect("a show line counts");
    }

    #[test]
    fn a_second_start_waits_for_the_port_of_a_starting_app() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let (instance, shown) = first(dir.path());
        let port = fs::read_to_string(port_path(dir.path())).expect("written");
        fs::remove_file(port_path(dir.path())).expect("removed");
        let later = {
            let path = port_path(dir.path());
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                fs::write(path, port).expect("written");
            })
        };
        assert!(matches!(start(dir.path(), false), Ok(Start::Running)));
        shown.recv_timeout(Duration::from_secs(5)).expect("forwarded once the port is there");
        later.join().expect("joined");
        drop(instance);
    }
}
