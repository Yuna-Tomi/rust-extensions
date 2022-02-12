/*
   Copyright The containerd Authors.

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
*/

use std::io;
use std::os::unix::prelude::AsRawFd;
use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncWrite};

use crate::Master;

impl<F: AsRawFd + AsyncRead + Unpin> AsyncRead for Master<F> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<F: AsRawFd + AsyncWrite + Unpin> AsyncWrite for Master<F> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
}

#[cfg(test)]
mod tests {

    use std::ffi::CString;

    use nix::errno::Errno;
    use nix::libc;
    use nix::unistd;
    use nix::unistd::ForkResult;
    use tokio::fs::{File, OpenOptions};
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    use super::*;
    use crate::Console;
    use crate::{new_pty_pair, new_pty_pair2};

    const ETTY: &str = "cannot allocate pty.";
    const ERR_R: &str = "cannot read.";
    const ERR_W: &str = "cannot write.";
    const EJOIN_R: &str = "failed to join read task.";
    const EJOIN_W: &str = "failed to join write task.";
    const ESPA: &str = "failed to spawn child.";
    const ECHI: &str = "error in child process.";

    // FIXME: this test fails on Linux with Errno 5, while succeeds on macOS.
    #[tokio::test]
    async fn test() {
        let (mut mst, mut slv) = new_pty_pair::<File>().expect(ETTY);

        let read = async move {
            let mut buf = String::new();
            mst.read_to_string(&mut buf).await.expect(ERR_R);
            buf
        };
        let write = async move { slv.write(b"Hello, console!\n").await.expect(ERR_W) };

        let (msg, wbytes) = tokio::join!(read, write);

        assert_eq!(wbytes, 16);
        assert_eq!("Hello, console!\r\n", msg);
    }

    // FIXME: this test fails on Linux with Errno 5, while succeeds on macOS.
    #[tokio::test]
    async fn test_parallel() {
        let (mut mst, mut slv) = new_pty_pair::<File>().expect(ETTY);

        let read = tokio::spawn(async move {
            let mut buf = String::new();
            mst.read_to_string(&mut buf).await.expect(ERR_R);
            buf
        });

        let write =
            tokio::spawn(async move { slv.write(b"Hello, console!\n").await.expect(ERR_W) });

        let (read, write) = tokio::join!(read, write);
        let msg = read.expect(EJOIN_R);
        let wbytes = write.expect(EJOIN_W);

        assert_eq!(wbytes, 16);
        assert_eq!("Hello, console!\r\n", msg);
    }

    // FIXME: this test fails on Linux with Errno 5, while succeeds on macOS.
    #[tokio::test]
    async fn test_command() {
        let (mut mst, slv) = new_pty_pair::<File>().expect(ETTY);

        let write = async move {
            let slv = slv.into_std().await;
            let mut cmd = Command::new("echo");
            cmd.arg("Hello, console!").stdout(slv);
            let chi = cmd.spawn().expect(ESPA);
            let _ = chi.wait_with_output().await.expect(ECHI);
        };

        let mut msg = String::new();
        let read = mst.read_to_string(&mut msg);

        let (read, _) = tokio::join!(read, write);
        read.expect(ERR_R);
        assert_eq!("Hello, console!\r\n", msg);
    }

    // FIXME: this test fails on Linux with Errno 5, while succeeds on macOS.
    #[tokio::test]
    async fn test_manually_fork() -> io::Result<()> {
        let (mut mst, slv) = new_pty_pair::<File>().expect(ETTY);
        mst.set_raw().expect("failed to set raw.");

        unsafe {
            match unistd::fork()? {
                ForkResult::Parent { .. } => {
                    drop(slv);
                }
                ForkResult::Child => {
                    mst.reset().expect("failed to reset termios");
                    drop(mst);
                    let res = libc::login_tty(slv.as_raw_fd());
                    Errno::result(res).expect("failed to login_tty");
                    let cmd = [
                        CString::new("echo").unwrap(),
                        CString::new("Hello, console!").unwrap(),
                    ];
                    // destructors never run in this child, then we don't have to forget slv
                    // even it has been closed at libc::login_tty above.
                    unistd::execvp(&cmd[0], &cmd).expect("failed to exec command");
                    unreachable!()
                }
            }
        };

        let mut msg = String::new();
        mst.read_to_string(&mut msg).await.expect(ERR_R);
        assert_eq!("Hello, console!\r\n", msg);
        Ok(())
    }

    // FIXME: this test fails on Linux with Errno 5 and fails on macOS with unexpected ENOTTY(see comment in new_pty_pair2)
    #[tokio::test]
    async fn test_manually_fork2() -> io::Result<()> {
        let (mut mst, slv) = new_pty_pair2::<File>().expect("cannot allocate pty.");
        mst.set_raw().expect("failed to set raw.");

        unsafe {
            match unistd::fork()? {
                ForkResult::Parent { .. } => {
                    drop(slv);
                }
                ForkResult::Child => {
                    mst.reset().expect("failed to reset termios");
                    drop(mst);
                    let slv = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .mode(0)
                        .open(&slv)
                        .await
                        .expect("failed to open slave.");
                    let res = libc::login_tty(slv.as_raw_fd());
                    Errno::result(res).expect("failed to login_tty");
                    let cmd = [
                        CString::new("echo").unwrap(),
                        CString::new("Hello, console!").unwrap(),
                    ];
                    // destructors never run in this child, then we don't have to forget slv
                    // even it has been closed at libc::login_tty above.
                    unistd::execvp(&cmd[0], &cmd).expect("failed to exec command");
                    unreachable!()
                }
            }
        };

        let mut msg = String::new();
        mst.read_to_string(&mut msg).await.expect(ERR_R);
        assert_eq!("Hello, console!\r\n", msg);
        Ok(())
    }
}
