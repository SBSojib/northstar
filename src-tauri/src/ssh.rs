use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use crate::{
    database::Database,
    error::{AppError, AppResult},
    models::{Credential, TerminalOutput, TerminalStatus},
};
use ssh2::Session;
use tauri::{AppHandle, Emitter};

enum SessionCommand {
    Input(Vec<u8>),
    Resize(u32, u32),
    Disconnect,
}

pub struct SshManager {
    sessions: Arc<Mutex<HashMap<String, mpsc::Sender<SessionCommand>>>>,
}

impl SshManager {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn connect(
        &self,
        app: AppHandle,
        database: &Database,
        host_id: i64,
        session_id: String,
        cols: u32,
        rows: u32,
    ) -> AppResult<String> {
        let (host, credential) = database.host_with_credential(host_id)?;
        let (sender, receiver) = mpsc::channel();
        self.sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?
            .insert(session_id.clone(), sender);

        let thread_session_id = session_id.clone();
        let sessions = Arc::clone(&self.sessions);
        thread::spawn(move || {
            emit_status(&app, &thread_session_id, "connecting", None);
            let result = run_session(
                &app,
                &thread_session_id,
                &host.address,
                host.port,
                &host.username,
                credential,
                cols,
                rows,
                receiver,
            );
            match result {
                Ok(()) => emit_status(&app, &thread_session_id, "closed", None),
                Err(error) => {
                    emit_status(&app, &thread_session_id, "error", Some(error.to_string()))
                }
            }
            if let Ok(mut sessions) = sessions.lock() {
                sessions.remove(&thread_session_id);
            }
        });

        Ok(session_id)
    }

    pub fn input(&self, session_id: &str, data: Vec<u8>) -> AppResult<()> {
        self.send(session_id, SessionCommand::Input(data))
    }

    pub fn resize(&self, session_id: &str, cols: u32, rows: u32) -> AppResult<()> {
        self.send(session_id, SessionCommand::Resize(cols, rows))
    }

    pub fn disconnect(&self, session_id: &str) -> AppResult<()> {
        let sender = self
            .sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?
            .remove(session_id)
            .ok_or(AppError::SessionNotFound)?;
        sender
            .send(SessionCommand::Disconnect)
            .map_err(|_| AppError::SessionNotFound)
    }

    fn send(&self, session_id: &str, command: SessionCommand) -> AppResult<()> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::Internal("Session lock was poisoned".into()))?;
        let result = sessions
            .get(session_id)
            .ok_or(AppError::SessionNotFound)?
            .send(command);
        if result.is_err() {
            sessions.remove(session_id);
            return Err(AppError::SessionNotFound);
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn run_session(
    app: &AppHandle,
    session_id: &str,
    address: &str,
    port: u16,
    username: &str,
    credential: Credential,
    cols: u32,
    rows: u32,
    receiver: mpsc::Receiver<SessionCommand>,
) -> AppResult<()> {
    let socket_address = (address, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| AppError::InvalidInput("Could not resolve host address".into()))?;
    let tcp = TcpStream::connect_timeout(&socket_address, Duration::from_secs(15))?;
    tcp.set_read_timeout(Some(Duration::from_secs(15)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(15)))?;

    let mut ssh = Session::new()?;
    ssh.set_tcp_stream(tcp);
    ssh.handshake()?;
    match credential {
        Credential::Password { password } => ssh.userauth_password(username, &password)?,
        Credential::PrivateKey {
            private_key,
            passphrase,
        } => ssh.userauth_pubkey_memory(username, None, &private_key, passphrase.as_deref())?,
    }
    if !ssh.authenticated() {
        return Err(AppError::InvalidInput("Authentication failed".into()));
    }

    let mut channel = ssh.channel_session()?;
    channel.request_pty("xterm-256color", None, Some((cols, rows, 0, 0)))?;
    channel.shell()?;
    ssh.set_blocking(false);
    emit_status(app, session_id, "connected", None);

    let mut buffer = [0_u8; 16 * 1024];
    loop {
        while let Ok(command) = receiver.try_recv() {
            match command {
                SessionCommand::Input(data) => write_nonblocking(&mut channel, &data)?,
                SessionCommand::Resize(cols, rows) => {
                    channel.request_pty_size(cols, rows, None, None)?
                }
                SessionCommand::Disconnect => {
                    let _ = channel.close();
                    return Ok(());
                }
            }
        }

        match channel.read(&mut buffer) {
            Ok(0) if channel.eof() => break,
            Ok(0) => {}
            Ok(count) => emit_output(app, session_id, &buffer[..count]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }

        match channel.stderr().read(&mut buffer) {
            Ok(0) => {}
            Ok(count) => emit_output(app, session_id, &buffer[..count]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }

        if channel.eof() {
            break;
        }
        thread::sleep(Duration::from_millis(8));
    }

    ssh.set_blocking(true);
    channel.wait_close()?;
    Ok(())
}

fn write_nonblocking(channel: &mut ssh2::Channel, mut data: &[u8]) -> AppResult<()> {
    while !data.is_empty() {
        match channel.write(data) {
            Ok(0) => thread::sleep(Duration::from_millis(4)),
            Ok(count) => data = &data[count..],
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(4))
            }
            Err(error) => return Err(error.into()),
        }
    }
    channel.flush()?;
    Ok(())
}

fn emit_output(app: &AppHandle, session_id: &str, data: &[u8]) {
    let _ = app.emit(
        "terminal-output",
        TerminalOutput {
            session_id: session_id.to_owned(),
            data: data.to_vec(),
        },
    );
}

fn emit_status(app: &AppHandle, session_id: &str, status: &str, message: Option<String>) {
    let _ = app.emit(
        "terminal-status",
        TerminalStatus {
            session_id: session_id.to_owned(),
            status: status.to_owned(),
            message,
        },
    );
}
