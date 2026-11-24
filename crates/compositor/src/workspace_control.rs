//! Bounded, user-local workspace commands (no arbitrary executable or shell input).
use anyhow::{Context, Result};
use crate::state::Atelier;
use smithay::{input::pointer::MotionEvent, utils::SERIAL_COUNTER};
use std::{fs::{self, File, OpenOptions}, io::{Seek, SeekFrom, Write}, os::unix::{fs::{OpenOptionsExt, PermissionsExt}, net::UnixDatagram}, path::PathBuf};

pub struct WorkspaceControl {
    socket: UnixDatagram,
    socket_path: PathBuf,
    state_path: PathBuf,
    state: File,
    published: Option<usize>,
}

impl WorkspaceControl {
    pub fn bind(display: &str) -> Result<Self> {
        let (socket_path, state_path) = atelier_config::workspace_paths(display)?;
        let socket = UnixDatagram::bind(&socket_path).context("bind workspace command socket")?;
        let opened = (|| -> Result<File> {
            fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
            socket.set_nonblocking(true)?;
            Ok(OpenOptions::new().write(true).create_new(true).mode(0o600).open(&state_path)?)
        })();
        match opened {
            Ok(state) => Ok(Self { socket, socket_path, state_path, state, published: None }),
            Err(error) => { let _ = fs::remove_file(&socket_path); Err(error) }
        }
    }

    pub fn poll(&mut self, atelier: &mut Atelier, time: u32) -> Result<()> {
        for _ in 0..32 {
            let mut data = [0u8; 2];
            match self.socket.recv(&mut data) {
                Ok(1) if (1..=4).contains(&data[0]) => {
                    let serial = SERIAL_COUNTER.next_serial();
                    atelier.select_workspace(usize::from(data[0] - 1), false, serial);
                    let focus = atelier.surface_under_pointer();
                    let location = atelier.pointer_position;
                    if let Some(pointer) = atelier.seat.get_pointer() {
                        if !pointer.is_grabbed() {
                            pointer.motion(atelier, focus, &MotionEvent { location, serial, time });
                            pointer.frame(atelier);
                        }
                    }
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        let active = atelier.active_workspace();
        if self.published != Some(active) {
            self.state.seek(SeekFrom::Start(0))?;
            self.state.write_all(&[(active + 1) as u8])?;
            self.published = Some(active);
        }
        Ok(())
    }
}

impl Drop for WorkspaceControl {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
        let _ = fs::remove_file(&self.state_path);
    }
}
