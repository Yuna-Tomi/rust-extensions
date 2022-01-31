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

use super::config::StdioConfig;
use super::fifo::{self, Fifo};
use containerd_runc_rust as runc;
use nix::fcntl::{self, OFlag};
use nix::sys::stat::Mode;
use runc::io::{IOOption, NullIO, RuncIO, RuncPipedIO};
use std::os::unix::prelude::FromRawFd;
use std::path::Path;
use std::pin::Pin;
use std::{
    ffi::OsStr,
    os::unix::{fs::DirBuilderExt, prelude::RawFd},
    process::Command,
    sync::{Arc, RwLock},
};
use std::{fs::DirBuilder, os::unix::prelude::AsRawFd};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};
use url::{ParseError, Url};

use crate::dbg::*;

#[derive(Debug, Clone, Default)]
pub struct ProcessIO {
    pub io: Option<Box<dyn RuncIO>>,
    pub uri: Option<Url>,
    pub copy: bool,
    pub stdio: StdioConfig,
}

impl ProcessIO {
    pub fn new(
        id: &str,
        io_uid: isize,
        io_gid: isize,
        stdio: StdioConfig,
    ) -> std::io::Result<Self> {
        // Only NullIO is supported now.
        return Ok(Self {
            io: Some(Box::new(NullIO::new()?)),
            copy: false,
            stdio,
            ..Default::default()
        });

        if stdio.is_null() {
            return Ok(Self {
                io: Some(Box::new(NullIO::new()?)),
                copy: false,
                stdio,
                ..Default::default()
            });
        }

        let u = match Url::parse(&stdio.stdout) {
            Ok(u) => u,
            Err(ParseError::RelativeUrlWithoutBase) => {
                // ugry hack: parse twice...
                Url::parse(&format!("fifo:{}", stdio.stdout)).unwrap()
            }
            Err(e) => {
                return Err(std::io::ErrorKind::NotFound.into());
            }
        };

        match u.scheme() {
            "fifo" => {
                let io = Box::new(RuncPipedIO::new(
                    io_uid,
                    io_gid,
                    conditional_io_options(&stdio),
                )?);
                Ok(Self {
                    io: Some(io as Box<dyn RuncIO>),
                    uri: Some(u),
                    copy: true,
                    stdio,
                })
            }
            "binary" => {
                // FIXME: appropriate binary io
                panic!("unimplemented");
                // Ok(Self {
                //     io: Some(Box::new(BinaryIO::new("dummy")?) as Box<dyn RuncIO>),
                //     uri: Some(u),
                //     copy: false,
                //     stdio,
                // })
            }
            "file" => {
                let path = Path::new(u.path());
                DirBuilder::new()
                    .recursive(true)
                    .mode(0o755)
                    .create(path.parent().unwrap())?; // don't pass root
                let _ = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .write(true)
                    .open(path)?; // follow the implementation in Go, immediately close the file.
                let mut stdio = stdio;
                stdio.stdout = path.to_string_lossy().parse::<String>().unwrap();
                stdio.stderr = path.to_string_lossy().parse::<String>().unwrap();
                let io = Box::new(RuncPipedIO::new(
                    io_uid,
                    io_gid,
                    conditional_io_options(&stdio),
                )?);
                Ok(Self {
                    io: Some(io as Box<dyn RuncIO>),
                    uri: Some(u),
                    copy: true,
                    stdio,
                })
            }
            _ => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        }
    }
}

// FIXME: suspended
impl ProcessIO {
    // fn close(&mut self) -> std::io::Result<()> {
    //     let mut x = self.io.as_ref().unwrap());
    //     .close()
    // }

    pub fn io(&self /* , cmd: &mut std::process::Command */) -> Option<Box<dyn RuncIO>> {
        if let Some(io) = &self.io {
            Some(io.clone())
        } else {
            None
        }
    }

    // FIXME: approriate pipe copy
    pub async fn copy_pipes(&self) -> std::io::Result<()> {
        if !self.copy {
            return Ok(());
        } else {
            let io = self.io().expect("runc io should be set before copying.");
            copy_pipes(io, &self.stdio).await
        }
    }
}

#[derive(Clone)]
pub struct BinaryIO {
    cmd: Option<Arc<Command>>,
    out: Pipe,
}

// FIXME: suspended
impl RuncIO for BinaryIO {
    fn stdin(&self) -> Option<RawFd> {
        panic!("unimplemented");
    }

    fn stderr(&self) -> Option<RawFd> {
        panic!("unimplemented")
    }

    fn stdout(&self) -> Option<RawFd> {
        panic!("unimplemented")
    }

    fn close(&mut self) {
        panic!("unimplemented")
    }

    unsafe fn set(&self, cmd: &mut Command) {
        panic!("unimplemented")
    }
}

impl BinaryIO {
    pub fn new(path: impl AsRef<OsStr>) -> std::io::Result<Self> {
        Ok(Self {
            cmd: Some(Arc::new(Command::new(path))),
            out: Pipe::new()?,
        })
    }
}

#[derive(Clone)]
pub struct Pipe {
    read_fd: RawFd,
    write_fd: RawFd,
}

impl Pipe {
    pub fn new() -> Result<Self, nix::Error> {
        let (read_fd, write_fd) = nix::unistd::pipe()?;
        Ok(Self { read_fd, write_fd })
    }
}

fn conditional_io_options(stdio: &StdioConfig) -> IOOption {
    IOOption {
        open_stdin: stdio.stdin != "",
        open_stdout: stdio.stdout != "",
        open_stderr: stdio.stderr != "",
    }
}

const FIFO_ERR_MSG: [&str; 2] = ["error copying stdout", "error copying stderr"];
const FIFO: [&str; 2] = ["stdout", "stderr"];

async fn copy_pipes(io: Box<dyn RuncIO>, stdio: &StdioConfig) -> std::io::Result<()> {
    let io_files = vec![io.stdout(), io.stderr()];

    // debug_log!("io files: {:?}", io_files);
    let out_err = vec![stdio.stdout.clone(), stdio.stderr.clone()];
    let mut same_file = None;
    let mut tasks = vec![];
    for (ix, (reader_fd, path)) in io_files.into_iter().zip(out_err.into_iter()).enumerate() {
        // Note that each io_file (stdout/stderr) have to std::mem::forget, in order not to close pipe.
        // Also, third argument corresponds to "not forget writer" for twice use of Fifo, in case of stdout==stderr.
        let dest = |mut writer: Pin<Box<dyn AsyncWrite + Unpin + Send>>,
                    r: Option<Fifo>,
                    drop_w: bool,
                    ix: usize,
                    reader_fd: Option<RawFd>| async move {
            match reader_fd {
                Some(f) => {
                    let f = unsafe { tokio::fs::File::from_raw_fd(f) };
                    debug_log!("{}\nreadfile: {:?}\nfifo: {:?}", ix, f, r);
                    let mut reader = BufReader::new(f);
                    let x = tokio::io::copy(&mut reader, &mut *writer).await?;
                    debug_log!("{} copy: {} bytes", FIFO[ix], x);
                    std::mem::forget(reader);
                    drop(r);
                    if !drop_w {
                        std::mem::forget(writer);
                    }
                    Ok(())
                }
                None => {
                    debug_log!("{}", FIFO_ERR_MSG[ix]);
                    Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
                }
            }
        };
        // might be ugly hack
        if fifo::is_fifo(&path)? {
            let t = tokio::task::spawn(async move {
                let w_mkfifo = Fifo::open(&path, OFlag::O_WRONLY, 0);
                let r_mkfifo = Fifo::open(&path, OFlag::O_RDONLY, 0);
                let w_fifo = w_mkfifo.await.map_err(|e| {
                    // debug_log!("error in await w_fifo {}", e);
                    e
                })?;
                let r_fifo = r_mkfifo.await.map_err(|e| {
                    // debug_log!("error in await r_fifo {}", e);
                    e
                })?;
                // debug_log!("spawn task with fifo...");
                // debug_log!("read end: {:?}", r_fifo);
                // debug_log!("write end: {:?}", w_fifo);
                let w = Box::pin(w_fifo);
                let r = Some(r_fifo);
                dest(w, r, true, ix, reader_fd).await
            });
            tasks.push(t);
        } else if let Some(w) = same_file.take() {
            // debug_log!("pipe is not fifo -> use same file for task...");
            let t = tokio::task::spawn(dest(w, None, true, ix, reader_fd));
            tasks.push(t);
            // debug_log!("task completed");
            continue;
        } else {
            // debug_log!("pipe is not fifo -> new file... {}", path.as_str());
            let drop_w = stdio.stdout == stdio.stderr;
            let t = tokio::task::spawn(async move {
                let f = tokio::fs::OpenOptions::new()
                    .write(true)
                    .append(true)
                    .mode(0)
                    .open(&path)
                    .await?;
                let w = Box::pin(f);
                // if drop_w {
                //     // might be ugly hack
                //     let f = unsafe { tokio::fs::File::from_raw_fd(w.as_raw_fd()) };
                //     let _ = same_file.get_or_insert(Box::pin(f));
                // }
                dest(w, None, drop_w, ix, reader_fd).await
            });
            tasks.push(t);
        }
    }
    if stdio.stdin != "" {
        let f = Fifo::open(&stdio.stdin, OFlag::O_RDONLY | OFlag::O_NONBLOCK, 0).await?;
        let copy_buf = async move {
            let stdin = unsafe { tokio::fs::File::from_raw_fd(io.stdin().unwrap()) };
            debug_log!("stdin write end: {:?}\nstdin read end: {:?}", stdin, f);
            let mut writer = BufWriter::new(stdin);
            let mut reader = BufReader::new(f);
            debug_log!(
                "stdin writer buffer: {:?}\nstdin reader buffer: {:?}",
                writer.buffer(),
                reader.buffer(),
            );
            match tokio::io::copy(&mut reader, &mut writer).await {
                Ok(x) => {
                    debug_log!("stdin copy: {} bytes", x);
                    Ok(())
                }
                Err(e) => {
                    debug_log!("{}", e);
                    Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
                }
            }
            // don't have to forget these reader/writer
        };
        debug_log!("spawn task for stdin");
        let t = tokio::task::spawn(copy_buf);
        tasks.push(t);
    }

    for t in tasks {
        t.await??;
    }
    // debug_log!("task completed");
    Ok(())
}
