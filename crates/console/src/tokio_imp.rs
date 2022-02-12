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
    use super::*;
    use crate::Console;
    use crate::new_pty_pair;
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    #[tokio::test]
    async fn test() -> io::Result<()> {
        let (mut mst, slv) = new_pty_pair::<File>().expect("cannot allocat pty.");
        mst.set_raw().expect("sfailed to set raw.");

        let t1 = tokio::spawn(async move {
            let slv = slv.into_std().await;
            let chi = Command::new("echo")
                .arg("Hello, console!")
                .stdout(slv).spawn().expect("failed to spawn child.");
            let out = chi.wait_with_output().await.expect("error in child process.");
            dbg!(out);
        });

        let t2 = tokio::spawn(async move {
            let mut buf = String::new();
            mst.read_to_string(&mut buf).await.expect("failed to read.");
            buf
        });

        t1.await?;
        let buf = t2.await?;
        assert_eq!("Hello, console!\r\n", buf);
        Ok(())
    }

    #[tokio::test]
    async fn test2() -> io::Result<()> {
        let (mut mst, mut slv) = new_pty_pair::<File>().expect("cannot allocat pty.");

        let t1 = tokio::spawn(async move {
            let msg = b"Hello, console!\n";
            slv.write(msg).await.expect("cannot write.");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        });

        let t2 = tokio::spawn(async move {
            let mut buf = String::new();
            mst.read_to_string(&mut buf).await.expect("failed to read.");
            buf
        });

        t1.await?;
        let buf = t2.await?;
        assert_eq!("Hello, console!\r\n", buf);
        Ok(())
    }
}
