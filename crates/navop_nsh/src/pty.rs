//! A small PTY bridge for `nsh-base`'s terminal component.
//!
//! The nsh terminal owns VT emulation, rendering, selection, scrollback and
//! keyboard input. This worker owns only the operating-system pseudo-terminal
//! and exposes a byte-stream command/message boundary.

use std::sync::mpsc::{Receiver, Sender};

use terminal::LocalConfig;

#[derive(Debug)]
pub(crate) enum PtyCommand {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

#[derive(Debug)]
pub(crate) enum PtyMessage {
    Data(Vec<u8>),
    Status(String),
}

#[cfg(unix)]
mod platform {
    use std::{
        env,
        fs::File,
        io::{ErrorKind, Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        },
        path::PathBuf,
        process::{Command, Stdio},
        sync::mpsc::TryRecvError,
        thread,
        time::Duration,
    };

    use super::{LocalConfig, PtyCommand, PtyMessage, Receiver, Sender};
    use anyhow::Context as _;
    use polling::{Event, Events, Poller};

    pub(crate) fn spawn(
        config: LocalConfig,
        commands: Receiver<PtyCommand>,
        output: Sender<PtyMessage>,
    ) {
        thread::spawn(move || {
            if let Err(error) = run(config, commands, &output) {
                let _ = output.send(PtyMessage::Status(format!("Terminal error: {error:#}")));
            }
        });
    }

    fn run(
        config: LocalConfig,
        commands: Receiver<PtyCommand>,
        output: &Sender<PtyMessage>,
    ) -> anyhow::Result<()> {
        let (master, slave) = open_pty(80, 24).context("open pseudo-terminal")?;
        let master_fd = master.as_raw_fd();
        let slave_fd = slave.as_raw_fd();
        set_nonblocking(master_fd).context("make PTY nonblocking")?;

        let shell = config
            .shell
            .filter(|shell| !shell.trim().is_empty())
            .or_else(|| env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/sh".to_owned());
        let mut command = Command::new(&shell);
        command.args(&config.args);
        if let Some(directory) = config.working_dir.as_deref() {
            command.current_dir(PathBuf::from(directory));
        }
        for (key, value) in config.env {
            command.env(key, value);
        }
        command
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // SAFETY: runs between fork and exec; only async-signal-safe libc calls
        // and the inherited descriptor are used.
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(slave_fd, libc::TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("spawn terminal shell {shell}"))?;
        let mut reader = master.try_clone().context("clone PTY reader")?;
        let mut writer = master;
        let poller = Poller::new().context("create PTY poller")?;
        // SAFETY: `reader` lives until the poller is dropped at the end of
        // this function.
        unsafe {
            poller
                .add(&reader, Event::readable(0))
                .context("register PTY with poller")?;
        }

        let _ = output.send(PtyMessage::Status("Terminal connected".to_owned()));
        let mut events = Events::new();
        let mut buffer = [0u8; 16 * 1024];

        loop {
            loop {
                match commands.try_recv() {
                    Ok(PtyCommand::Write(data)) => {
                        writer.write_all(&data).context("write terminal input")?;
                        writer.flush().context("flush terminal input")?;
                    }
                    Ok(PtyCommand::Resize { cols, rows }) => {
                        resize(master_fd, cols, rows).context("resize PTY")?;
                    }
                    Ok(PtyCommand::Shutdown) | Err(TryRecvError::Disconnected) => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Ok(());
                    }
                    Err(TryRecvError::Empty) => break,
                }
            }

            events.clear();
            match poller.wait(&mut events, Some(Duration::from_millis(16))) {
                Ok(_) => {}
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }

            let mut reached_eof = false;
            if events.iter().next().is_some() {
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => {
                            reached_eof = true;
                            break;
                        }
                        Ok(read) => {
                            let _ = output.send(PtyMessage::Data(buffer[..read].to_vec()));
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                        Err(error) => return Err(error.into()),
                    }
                }
            }

            if let Some(status) = child.try_wait().context("poll terminal process")? {
                if !reached_eof {
                    while let Ok(read) = reader.read(&mut buffer) {
                        if read == 0 {
                            break;
                        }
                        let _ = output.send(PtyMessage::Data(buffer[..read].to_vec()));
                    }
                }
                let _ = output.send(PtyMessage::Status(format!(
                    "Terminal process exited ({status})"
                )));
                return Ok(());
            }
        }
    }

    fn winsize(cols: u16, rows: u16) -> libc::winsize {
        libc::winsize {
            ws_row: rows.max(1),
            ws_col: cols.max(1),
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }

    fn resize(fd: i32, cols: u16, rows: u16) -> std::io::Result<()> {
        let size = winsize(cols, rows);
        if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size as *const _) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    fn open_pty(cols: u16, rows: u16) -> std::io::Result<(File, File)> {
        let mut master = -1;
        let mut slave = -1;
        let mut size = winsize(cols, rows);
        let result = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &mut size,
            )
        };
        if result == -1 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: a successful `openpty` returned two fresh owned descriptors.
        unsafe { Ok((File::from_raw_fd(master), File::from_raw_fd(slave))) }
    }

    fn set_nonblocking(fd: i32) -> std::io::Result<()> {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(unix))]
mod platform {
    use super::{LocalConfig, PtyMessage, Receiver, Sender};

    pub(crate) fn spawn(
        _config: LocalConfig,
        _commands: Receiver<super::PtyCommand>,
        output: Sender<PtyMessage>,
    ) {
        let _ = output.send(PtyMessage::Status(
            "Native PTY is not yet available on this platform".to_owned(),
        ));
    }
}

pub(crate) use platform::spawn;

#[cfg(all(test, unix))]
mod tests {
    use std::{
        sync::mpsc::channel,
        time::{Duration, Instant},
    };

    use super::*;

    #[test]
    fn pty_session_runs_a_shell_command_and_returns_its_output() {
        let (command_tx, command_rx) = channel();
        let (output_tx, output_rx) = channel();
        let config = LocalConfig {
            shell: Some("/bin/sh".to_owned()),
            args: vec!["-c".to_owned(), "printf nsh-pty-ok".to_owned()],
            working_dir: None,
            env: Vec::new(),
        };
        spawn(config, command_rx, output_tx);

        let mut seen = Vec::new();
        let mut statuses = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !seen.windows(10).any(|w| w == b"nsh-pty-ok") {
            match output_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(PtyMessage::Data(bytes)) => seen.extend(bytes),
                Ok(PtyMessage::Status(status)) => statuses.push(status),
                Err(_) => {}
            }
        }
        let _ = command_tx.send(PtyCommand::Shutdown);
        assert!(
            seen.windows(10).any(|window| window == b"nsh-pty-ok"),
            "PTY output was {:?}; statuses: {:?}",
            String::from_utf8_lossy(&seen),
            statuses
        );
    }
}
