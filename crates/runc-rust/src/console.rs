/*
   copyright the containerd authors.

   licensed under the apache license, version 2.0 (the "license");
   you may not use this file except in compliance with the license.
   you may obtain a copy of the license at

       http://www.apache.org/licenses/license-2.0

   unless required by applicable law or agreed to in writing, software
   distributed under the license is distributed on an "as is" basis,
   without warranties or conditions of any kind, either express or implied.
   see the license for the specific language governing permissions and
   limitations under the license.
*/

// Forked from https://github.com/pwFoo/rust-runc/blob/master/src/console.rs
/*
 * Copyright 2019 fsyncd, Berlin, Germany.
 * Additional material, copyright of the containerd authors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::error::Error;
use crate::utils;
use log::warn;
use mio::net::{SocketAddr, UnixListener};
use std::env;
use std::ffi::c_void;
use std::os::unix::io::AsRawFd;
use std::os::unix::io::FromRawFd;
use std::path::{Path, PathBuf};
use std::{fs, ptr};
use tempfile;
use tempfile::TempDir;
// use tokio::future::poll_fn;
use tokio::fs::File;
use tokio::io::unix::AsyncFd;
use uuid::Uuid;

/// Receive a PTY master over the provided unix socket
pub struct ReceivePtyMaster {
    pub console_socket: PathBuf,
    /// Temporal directory for pty.sock. This will be [`Some`] only if you make this struct with `new_with_temp_sock`
    /// If you use tempdir to bind socket, you should contain TempDir to guarantee the tempdir exits as long as pty master.
    temp_pty_dir: Option<TempDir>,
    listener: Option<UnixListener>,
    /// temporal socket should be cleaned, including its tempdir
    is_temp: bool,
}

// Looks to be a false positive
#[allow(clippy::cast_ptr_alignment)]
impl ReceivePtyMaster {
    /// Bind a unix domain socket to the provided path
    pub fn new(console_socket: PathBuf) -> Result<Self, Error> {
        let listener = UnixListener::bind(utils::abs_path_buf(&console_socket)?)
            .map_err(Error::UnixSocketConnectionError)?;
        Ok(Self {
            console_socket,
            temp_pty_dir: None,
            listener: Some(listener),
            is_temp: false,
        })
    }

    /// Creating temporal socket
    pub fn new_with_temp_sock() -> Result<Self, Error> {
        // it cannot be assumed that environment variable "XDG_RUNTIME_DIR" always exists.
        // let runtime_dir = env::var("XDG_RUNTIME_DIR").map_err(Error::EnvError)?;
        let pty_dir = tempfile::Builder::new()
            .prefix(&format!("pty{}", rand::random::<u32>()))
            .tempdir_in("/tmp")
            .map_err(Error::FileSystemError)?;
        let console_socket = utils::abs_path_buf(pty_dir.path().join("pty.sock"))?;
        let listener =
            UnixListener::bind(&console_socket).map_err(Error::UnixSocketConnectionError)?;
        Ok(Self {
            console_socket,
            temp_pty_dir: Some(pty_dir),
            listener: Some(listener),
            is_temp: true,
        })
    }

    /// Receive a master PTY file descriptor from the socket
    pub async fn receive(mut self) -> Result<File, Error> {
        // let io = AsyncFd::new(self.listener.unwrap()).map_err(|_| Error::UnixSocketReceiveMessageError)?;
        // poll_fn(|cx| io.poll_read_ready(cx))
        //     .await
        //     .unwrap();

        // let (console_stream, _) = io
        //     .get_ref()
        //     .accept()
        //     .map_err(|e| Error::UnixSocketConnectionError(e))?;

        // let console_stream = AsyncFd::new(console_stream).map_err(|e| Error::OtherError(e))?;
        Err(Error::UnimplementedError("PtyMaster.receive()".to_string()))

        // loop {
        //     poll_fn(|cx| console_stream.poll_read_ready(cx))
        //         .await
        //         .unwrap();

        //     {
        //         // 4096 is the max name length from the go-runc implementation
        //         let mut iov_base = [0u8; 4096];
        //         let mut message_buf = [0u8; 24];
        //         let mut io = libc::iovec {
        //             iov_len: iov_base.len(),
        //             iov_base: &mut iov_base as *mut _ as *mut c_void,
        //         };
        //         let mut msg = libc::msghdr {
        //             msg_name: ptr::null_mut(),
        //             msg_namelen: 0,
        //             msg_iov: &mut io,
        //             msg_iovlen: 1,
        //             msg_control: &mut message_buf as *mut _ as *mut c_void,
        //             msg_controllen: message_buf.len(),
        //             msg_flags: 0,
        //         };

        //         let console_stream_fd = console_stream.get_ref().as_raw_fd();
        //         let ret = unsafe { libc::recvmsg(console_stream_fd, &mut msg, 0) };
        //         ensure!(ret >= 0, UnixSocketReceiveMessageError {});
        //         unsafe {
        //             let cmsg = libc::CMSG_FIRSTHDR(&msg);
        //             if cmsg.is_null() {
        //                 continue;
        //             }
        //             let cmsg_data = libc::CMSG_DATA(cmsg);
        //             ensure!(!cmsg_data.is_null(), UnixSocketReceiveMessageError {});
        //             return Ok(File::from_std(std::fs::File::from_raw_fd(
        //                 ptr::read_unaligned(cmsg_data as *const i32),
        //             )));
        //         }
        //     }
        // }
    }
}

impl Drop for ReceivePtyMaster {
    fn drop(&mut self) {
        if self.is_temp {
            let dir_path = self.console_socket.parent().unwrap();
            if let Err(e) = fs::remove_dir_all(dir_path) {
                warn!("failed to clean up tempdir for socket: {}", e);
            }
        } else if let Err(e) = fs::remove_file(&self.console_socket) {
            warn!("failed to clean up console socket: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_sock() {
        match ReceivePtyMaster::new_with_temp_sock() {
            Ok(receiver) => {
                drop(receiver);
            }
            Err(e) => panic!("couldn't create temporal socket. {}", e),
        }
    }
}
