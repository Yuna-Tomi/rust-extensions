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

// NOTE: Go references
// https://github.com/containerd/fifo/blob/main/fifo.go

use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::prelude::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;
use std::pin::Pin;
use std::task::Poll;

use nix::fcntl::{self, OFlag};
use nix::sys::stat::{self, Mode};
use nix::unistd;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot::error::TryRecvError;
use tokio::sync::oneshot::{self, Receiver};

use crate::dbg::*;

#[derive(Debug)]
pub struct Fifo {
    flag: OFlag,
    file: Option<tokio::fs::File>,
    handle: Handler,
    opened: Option<Receiver<tokio::fs::File>>,
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
    pub fn open<P>(path: P, mut flag: OFlag, perm: u32) -> std::io::Result<Self>
    where
        P: AsRef<Path>,
    {
        if let Err(e) = fs::metadata(&path) {
            if e.kind() == std::io::ErrorKind::NotFound && flag & OFlag::O_CREAT != OFlag::empty() {
                let perm = Mode::from_bits_truncate(perm & 0o777);
                unistd::mkfifo(path.as_ref(), perm)?;
            } else {
                return Err(e);
            }
        };

        let block =
            flag & OFlag::O_NONBLOCK == OFlag::empty() || flag & OFlag::O_RDWR != OFlag::empty();

        flag &= !OFlag::O_CREAT;
        flag &= !OFlag::O_NONBLOCK;

        let handle = Handler::new(&path)?;
        let path = handle.path()?;
        let mut opts = tokio::fs::OpenOptions::new();
        match flag & OFlag::O_ACCMODE {
            OFlag::O_RDONLY => { opts.read(true); }
            OFlag::O_WRONLY => { opts.write(true); }
            OFlag::O_RDWR   => { opts.read(true).write(true); }
            _ => {}
        }
        opts.mode(0).custom_flags(flag.bits());

        // FIXME:
        // following Go's implementation, we have to prepare file on other thread.
        let (tx, open_rx) = oneshot::channel::<tokio::fs::File>();
        tokio::spawn(async move {
            let f = opts.open(&path).await.map_err(|e| 
                debug_log!("error in fifo setting: {}", e)
            ).unwrap();
            tx.send(f).unwrap();
        });
        // FIXME:
        if block {}
        Ok(Self {
            flag,
            file: None,
            opened: Some(open_rx),
            // closing,
            // closed,
            handle,
        })
    }

    pub fn close(&self) -> std::io::Result<()> {
        self.handle.close()
    }
}

impl AsyncWrite for Fifo {
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        if let Some(f) = &mut this.file {
            Pin::new(f).poll_flush(cx)
        } else {
            let f = match this.opened.as_mut().unwrap().try_recv() {
                Ok(f) => f,
                Err(TryRecvError::Empty) => {
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                Err(TryRecvError::Closed) => panic!("channel closed."),
            };
            this.file.get_or_insert(f);
            Pin::new(&mut this.file.as_mut().unwrap()).poll_flush(cx)
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        let this = self.get_mut();
        if let Some(f) = &mut this.file {
            Pin::new(f).poll_shutdown(cx)
        } else {
            let f = match this.opened.as_mut().unwrap().try_recv() {
                Ok(f) => f,
                Err(TryRecvError::Empty) => {
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                Err(TryRecvError::Closed) => panic!("channel closed."),
            };
            this.file.get_or_insert(f);
            Pin::new(&mut this.file.as_mut().unwrap()).poll_shutdown(cx)
        }
    }

    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        if let Some(f) = &mut this.file {
            Pin::new(f).poll_write(cx, buf)
        } else {
            let f = match this.opened.as_mut().unwrap().try_recv() {
                Ok(f) => f,
                Err(TryRecvError::Empty) => {
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                Err(TryRecvError::Closed) => panic!("channel closed."),
            };
            this.file.get_or_insert(f);
            Pin::new(&mut this.file.as_mut().unwrap()).poll_write(cx, buf)
        }
    }

    fn poll_write_vectored(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        let this = self.get_mut();
        if let Some(f) = &mut this.file {
            Pin::new(f).poll_write_vectored(cx, bufs)
        } else {
            let f = match this.opened.as_mut().unwrap().try_recv() {
                Ok(f) => f,
                Err(TryRecvError::Empty) => {
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                Err(TryRecvError::Closed) => panic!("channel closed."),
            };
            this.file.get_or_insert(f);
            Pin::new(&mut this.file.as_mut().unwrap()).poll_write_vectored(cx, bufs)
        }
    }
}

impl AsyncRead for Fifo {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if let Some(f) = &mut this.file {
            Pin::new(f).poll_read(cx, buf)
        } else {
            let f = match this.opened.as_mut().unwrap().try_recv() {
                Ok(f) => f,
                Err(TryRecvError::Empty) => {
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                Err(TryRecvError::Closed) => panic!("channel closed."),
            };
            this.file.get_or_insert(f);
            Pin::new(&mut this.file.as_mut().unwrap()).poll_read(cx, buf)
        }
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

/// File manager at fd level for Fifo.
impl Handler {
    pub fn new<P>(path: P) -> std::io::Result<Self>
    where
        P: AsRef<Path>,
    {
        // here, we use fcntl directly because O_PATH is not suitable for OpenOptions
        // see https://rust-lang.github.io/rfcs/1252-open-options.html#no-access-mode-set
        let fd = fcntl::open(path.as_ref(), OFlag::O_PATH, Mode::empty())?;
        let file = unsafe { tokio::fs::File::from_raw_fd(fd) };
        let stat = stat::fstat(fd)?;
        let handler = Handler {
            file,
            dev: stat.st_dev,
            ino: stat.st_ino,
            fd,
            name: path.as_ref().to_string_lossy().parse::<String>().unwrap(),
        };

        // check /proc just in case: follow the Go's implementation
        let _ = stat::stat(handler.proc_path().as_str())?;
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
