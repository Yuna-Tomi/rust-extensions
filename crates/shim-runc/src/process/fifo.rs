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

use log::error;
use nix::fcntl::{self, OFlag};
use nix::sys::stat::{self, Mode};
use nix::unistd;
use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::prelude::{AsRawFd, FromRawFd, OpenOptionsExt, RawFd};
use std::path::Path;
use std::pin::Pin;
use std::task::Poll;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::oneshot::{self, Receiver};

use crate::dbg::*;

#[derive(Debug)]
pub struct Fifo {
    flag: OFlag,
    // opened: Receiver<()>,
    // closed: Receiver<()>,
    // closing: Receiver<()>,
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
    #[rustfmt::skip]
    pub async fn open<P>(path: P, mut flag: OFlag, perm: u32) -> std::io::Result<Self>
    where
        P: AsRef<Path>,
    {
        // debug_log!(
        //     "Fifo::open: path={:?}, flag(base8)={:o}, perm={:o}",
        //     path.as_ref(),
        //     flag.bits(),
        //     perm
        // );

        if let Err(e) = fs::metadata(&path) {
            debug_log!("oops: {}", e);
            if e.kind() == std::io::ErrorKind::NotFound && flag & OFlag::O_CREAT != OFlag::empty() {
                debug_log!("no fifo, then creating...");
                let perm = Mode::from_bits_truncate(perm & 0o777);
                unistd::mkfifo(path.as_ref(), perm)?;
            } else {
                return Err(e);
            }
        };
        // let (opened_tx, opened) = oneshot::channel::<()>();
        // let (closing_tx, closing) = oneshot::channel::<()>();
        // let (closed_tcx, closed) = oneshot::channel::<()>();

        let block =
            flag & OFlag::O_NONBLOCK == OFlag::empty() || flag & OFlag::O_RDWR != OFlag::empty();

        flag &= !OFlag::O_CREAT;
        flag &= !OFlag::O_NONBLOCK;

        debug_log!("Create Hander...");
        let handle = Handler::new(&path).await?;
        debug_log!("Handler created!");
        // ugly hack: have to concurrently prepare files
        let path = handle.path()?;
        let mut opts = tokio::fs::OpenOptions::new();
        match flag & OFlag::O_ACCMODE {
            OFlag::O_RDONLY => { opts.read(true); }
            OFlag::O_WRONLY => { opts.write(true); }
            OFlag::O_RDWR   => { opts.read(true).write(true); }
            _ => {}
        }
        opts.mode(0).custom_flags(flag.bits());
        // debug_log!("option set: {:?}", opts);

        /* DEBUG ------------------------------------------------------------------ */
        let _out = std::process::Command::new("ls")
            .arg("-l")
            .arg("/proc/self/fd")
            .output().map_err(|e| {
                debug_log!("{}", e);
                e
            })?;
        let _out = String::from_utf8(_out.stdout).unwrap();
        let _out = _out.split("\n").collect::<Vec<&str>>();
        debug_log!("Access fifo: path={:?}, flag={:?}", path, flag);
        debug_log!("fds: {:#?}", _out);
        /* DEBUG ------------------------------------------------------------------ */

        let file = opts.open(&path).await.map_err(|e| {
            debug_log!("fifo access open failed: {}", e);
            e
        })?;
        // let close_task = async {};
        // tokio::task::spawn(async {}).await?;

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
        // if block {}

        debug_log!("Fifo created");
        Ok(Self {
            flag,
            file,
            // opened,
            // closing,
            // closed,
            handle,
        })
    }

    // pub fn read(&mut self) -> std::io::Result<usize> {
    //     let f = self.file.as_mut().unwrap();
    //     Ok(1)
    // }

    pub fn close(&self) -> std::io::Result<()> {
        self.handle.close()
    }

    pub fn write(&mut self) -> std::io::Result<()> {
        let mut f = unsafe { std::fs::File::from_raw_fd(self.file.as_raw_fd()) };
        let msg = "debug";
        debug_log!("writing messege into fifo... msg={}, fifo={:?}", msg, f);
        f.write(msg.as_bytes())?;
        f.flush()?;
        std::mem::forget(f);
        Ok(())
    }
}

// impl futures::AsyncWrite for Fifo {
//     fn poll_write(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
//         Pin::new(&mut self.get_mut().file).poll_write(cx, buf)
//     }

//     fn poll_write_vectored(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>, bufs: &[std::io::IoSlice<'_>]) -> Poll<std::io::Result<usize>> {
//         Pin::new(&mut self.get_mut().file).poll_write_vectored(cx, bufs)
//     }

//     fn poll_close(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<std::io::Result<()>> {
//         Pin::new(&mut self.get_mut().file).poll_close(cx)
//     }

//     fn poll_flush(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<std::io::Result<()>> {
//         Pin::new(&mut self.get_mut().file).poll_flush(cx)
//     }
// }

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
    file: async_std::fs::File,
    fd: RawFd,
    dev: u64,
    ino: u64,
    name: String,
}

/// File manager at fd level for Fifo.
impl Handler {
    pub async fn new<P>(path: P) -> std::io::Result<Self>
    where
        P: AsRef<Path>,
    {
        debug_log!(
            "Handler file open: {:?}, mode={:o}",
            path.as_ref(),
            OFlag::O_PATH.bits()
        );
        // here, we use fcntl directly because O_PATH is not compatible for OpenOptions
        // see https://rust-lang.github.io/rfcs/1252-open-options.html#no-access-mode-set
        let fd = fcntl::open(path.as_ref(), OFlag::O_PATH, Mode::empty())?;
        let file = unsafe { async_std::fs::File::from_raw_fd(fd) };
        // let file = OpenOptions::new().mode(O_PATH).open(&path).await?;
        debug_log!("Have read handler file: {:?}", file);
        // let fd = file.as_raw_fd();
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
        debug_log!("checked stat.");
        Ok(handler)
    }
}

impl Handler {
    pub fn path(&self) -> std::io::Result<String> {
        let path = self.proc_path();
        let stat = stat::stat(path.as_str())?;
        if stat.st_dev != self.dev || stat.st_ino != self.ino {
            Err(std::io::Error::from(nix::Error::EBADFD))
        } else {
            Ok(path)
        }
    }

    pub fn proc_path(&self) -> String {
        let mut s = "/proc/self/fd/".to_string();
        s.push_str(&self.fd.to_string());
        s
    }

    pub fn close(&self) -> std::io::Result<()> {
        unistd::close(self.file.as_raw_fd())?;
        Ok(())
    }
}
