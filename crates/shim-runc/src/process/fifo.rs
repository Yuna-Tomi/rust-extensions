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

use futures::pin_mut;
use log::error;
use nix::fcntl::OFlag;
use nix::sys::stat::{self, Mode};
use nix::unistd;
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::prelude::{AsRawFd, RawFd, OpenOptionsExt};
use std::path::Path;
use std::pin::Pin;
use std::task::Poll;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::oneshot::{self, Receiver};

use crate::dbg::*;

#[derive(Debug)]
pub struct Fifo {
    flag: OFlag,
    opened: Receiver<()>,
    closed: Receiver<()>,
    closing: Receiver<()>,
    // FIXME: it should be Option to delay real creation of file
    file: tokio::fs::File,
    handle: Handler,
}

impl Fifo {
    /// perm is FileMode
    /// OpenFifo opens a fifo. Returns io.ReadWriteCloser.
    /// Context can be used to cancel this function until open(2) has not returned.
    /// Accepted flags:
    /// - OFlags.O_CREAT - create new fifo if one doesn't exist
    /// - OFlags.O_RDONLY - open fifo only from reader side
    /// - OFlags.O_WRONLY - open fifo only from writer side
    /// - OFlags.O_RDWR - open fifo from both sides, never block on syscall level
    /// - OFlags.O_NONBLOCK - return Fifo even if other side of the
    ///     fifo isn't open. read/write will be connected after the actual fifo is
    ///     open or after fifo is closed.
    pub async fn open<P>(path: P, mut flag: OFlag, perm: u32) -> std::io::Result<Self>
    where
        P: AsRef<Path>,
    {
        debug_log!("Fifo::open: path={:?}, flag(base8)={:o}, perm={:o}", path.as_ref(), flag.bits(), perm);

        if let Err(e) = fs::metadata(&path) {
            if e.kind() == std::io::ErrorKind::NotFound && flag & OFlag::O_CREAT != OFlag::empty() {
                debug_log!("no fifo, then creating...");
                let perm = Mode::from_bits_truncate(perm & 0o777);
                unistd::mkfifo(path.as_ref(), perm)?;
            } else {
                return Err(e);
            }
        };
        let (opened_tx, opened) = oneshot::channel::<()>();
        let (closing_tx, closing) = oneshot::channel::<()>();
        let (closed_tcx, closed) = oneshot::channel::<()>();

        let block =
            flag & OFlag::O_NONBLOCK == OFlag::empty() || flag & OFlag::O_RDWR != OFlag::empty();

        flag &= !OFlag::O_CREAT;
        flag &= !OFlag::O_NONBLOCK;

        debug_log!("Create Hander...");
        let handle = Handler::new(&path).await?;

        // ugly hack: have to concurrently prepare files
        let file = OpenOptions::new().mode(flag.bits() as u32).open("").await?;

        let mut fifo = Self {
            flag,
            file,
            opened,
            closing,
            closed,
            handle,
        };

        let close_task = async {};
        tokio::task::spawn(async {}).await?;

        // let file = tokio::task::spawn(async {
        //     // FIXME
        //     // let path = handle.path();
        //     let path = "";
        //     let opts = OpenOptions::new()
        //         .mode(flag as u32);
        //     match opts.open(path).await {
        //         Ok(f) => {
        //             // FIXME
        //             // if let Ok(_) = closing.try_recv() {
        //             //     // alreadly closing..
        //             //     Ok(None)
        //             // } else {
        //             //     Ok(Some(f))
        //             // }
        //             Ok(Some(f))
        //         }
        //         Err(e) => Err(e)
        //     }
        // }).await?;
        if block {}

        debug_log!("Fifo created");
        Ok(fifo)
    }

    // pub fn read(&mut self) -> std::io::Result<usize> {
    //     let f = self.file.as_mut().unwrap();
    //     Ok(1)
    // }
}

impl AsyncWrite for Fifo {
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().file).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Pin::new(&mut self.get_mut().file).poll_shutdown(cx)
    }

    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().file).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        Pin::new(&mut self.get_mut().file).poll_write_vectored(cx, bufs)
    }
}

impl AsyncRead for Fifo {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().file).poll_read(cx, buf)
    }
}

pub fn is_fifo<P>(path: P) -> std::io::Result<bool>
where
    P: AsRef<Path>,
{
    match fs::metadata(path) {
        Ok(m) if m.file_type().is_fifo() => Ok(true),
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[derive(Debug)]
pub struct Handler {
    file: tokio::fs::File,
    fd: RawFd,
    dev: u64,
    ino: u64,
    name: String,
}

const O_PATH: u32 = OFlag::O_PATH.bits() as u32;
impl Handler {
    pub async fn new<P>(path: P) -> std::io::Result<Self>
    where
        P: AsRef<Path>,
    {
        debug_log!("Handler file open: {:?}", path.as_ref());
        // Note that O_PATH file open can block if path is invalid (locked file)
        // ugly hack: its not good to wait
        let f = std::fs::OpenOptions::new().mode(O_PATH).open(&path)?;
        let file = tokio::fs::File::from(f);
        // let file = OpenOptions::new().mode(O_PATH).open(&path).await?;
        debug_log!("Have read handler file!");
        let fd = file.as_raw_fd();
        let stat = stat::fstat(fd)?;
        let handler = Handler {
            file,
            dev: stat.st_dev,
            ino: stat.st_ino,
            fd,
            name: path.as_ref().to_string_lossy().parse::<String>().unwrap(),
        };

        // check /proc just in case: follow the Go's implementation

        debug_log!("check /proc just in case: follow the Go's implementation...");
        let _ = stat::stat(handler.proc_path().as_str())?;
        Ok(handler)
    }
}

impl Handler {
    pub fn proc_path(&self) -> String {
        let mut s = "/proc/self/fd/".to_string();
        s.push_str(&self.fd.to_string());
        s
    }
}
