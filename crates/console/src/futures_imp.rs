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

use futures::{AsyncRead, AsyncWrite};

use crate::Master;

impl<F: AsRawFd + AsyncRead + Unpin> AsyncRead for Master<F> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<F: AsRawFd + AsyncWrite + Unpin> AsyncWrite for Master<F> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_close(cx)
    }
}

#[cfg(test)]
mod tests {

    use std::ffi::CString;
    use std::os::unix::prelude::FromRawFd;
    use std::os::unix::prelude::IntoRawFd;

    use async_std::fs::File;
    use async_std::process::Command;
    use futures::{AsyncReadExt, AsyncWriteExt};
    use nix::errno::Errno;
    use nix::libc;
    use nix::unistd;
    use nix::unistd::ForkResult;

    use super::*;
    use crate::new_pty_pair;
    use crate::Console;

    const ETTY: &str = "cannot allocate pty.";
    const ERR_R: &str = "cannot read.";
    const ERR_W: &str = "cannot write.";
    const ESPA: &str = "failed to spawn child.";
    const ECHI: &str = "error in child process.";

    // FIXME: this test fails on Linux with Errno 5, while succeeds on macOS.
    #[futures_test::test]
    async fn test() {
        let (mut mst, mut slv) = new_pty_pair::<File>().expect("cannot allocate pty.");

        let read = async move {
            let mut buf = String::new();
            mst.read_to_string(&mut buf).await.expect(ERR_R);
            buf
        };

        let write = async move { slv.write(b"Hello, console!\n").await.expect(ERR_W) };

        let (msg, wbytes) = futures::join!(read, write);

        assert_eq!(wbytes, 16);
        assert_eq!("Hello, console!\r\n", msg);
    }

    // FIXME: this test fails on Linux with Errno 5, while succeeds on macOS.
    #[tokio::test]
    async fn test_command() {
        let (mut mst, slv) = new_pty_pair::<File>().expect(ETTY);
        let slv = unsafe { std::fs::File::from_raw_fd(slv.into_raw_fd()) };

        let read = async move {
            let mut buf = String::new();
            mst.read_to_string(&mut buf).await.expect(ERR_R);
            buf
        };

        let write = async move {
            let mut cmd = Command::new("echo");
            cmd.arg("Hello, console!").stdout(slv);
            let chi = cmd.spawn().expect(ESPA);
            let _ = chi.output().await.expect(ECHI);
        };

        let (_, msg) = futures::join!(write, read);

        assert_eq!("Hello, console!\r\n", msg);
    }

    // FIXME: this test fails on Linux with Errno 5, while succeeds on macOS.
    #[futures_test::test]
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
}
