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
use super::fifo_noasync::{self, Fifo};
use containerd_runc_rust as runc;
use futures::executor;
use nix::fcntl::{self, OFlag};
use nix::sys::stat::Mode;
use runc::io::{IOOption, RuncIO, RuncPipedIO};
use std::os::unix::prelude::FromRawFd;
use std::path::Path;
use std::pin::Pin;
use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    os::unix::{fs::DirBuilderExt, prelude::RawFd},
    process::Command,
    sync::{Arc, RwLock},
};
use std::{fs::DirBuilder, os::unix::prelude::AsRawFd};
// use tokio::io::{AsyncRead, AsyncWrite, BufReader, BufWriter};
use std::io::{BufReader, BufWriter};
use url::{ParseError, Url};

use crate::dbg::*;

#[derive(Debug, Clone, Default)]
pub struct StdioConfig {
    pub stdin: String,
    pub stdout: String,
    pub stderr: String,
    pub terminal: bool,
}

impl StdioConfig {
    pub fn is_null(&self) -> bool {
        self.stdin == "" && self.stdout == "" && self.stderr == ""
    }
}

#[derive(Debug, Clone, Default)]
pub struct ProcessIO {
    // io: runc::IO,
    io: Option<Box<dyn RuncIO>>,
    uri: Option<Url>,
    copy: bool,
    stdio: StdioConfig,
}

impl ProcessIO {
    pub fn new(
        id: &str,
        io_uid: isize,
        io_gid: isize,
        stdio: StdioConfig,
    ) -> std::io::Result<Self> {
        if stdio.is_null() {
            return Ok(Self {
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
                Ok(Self {
                    // FIXME: appropriate binary io
                    io: Some(Box::new(BinaryIO::new("dummy")?) as Box<dyn RuncIO>),
                    uri: Some(u),
                    copy: false,
                    stdio,
                })
            }
            "file" => {
                let path = Path::new(u.path());
                DirBuilder::new()
                    .recursive(true)
                    .mode(0o755)
                    .create(path.parent().unwrap())?; // don't pass root
                let _ = OpenOptions::new()
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
    pub fn copy_pipes(&self) -> std::io::Result<()> {
        if !self.copy {
            return Ok(());
        }
        copy_pipes(self.io().expect("runc io not should be set before copying pipes."), &self.stdio)
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

fn copy_pipes(io: Box<dyn RuncIO>, stdio: &StdioConfig) -> std::io::Result<()> {
    let io_files = vec![io.stdout(), io.stderr()];
    std::mem::forget(io);
    debug_log!("io files: {:?}", io_files);
    let out_err = vec![&stdio.stdout, &stdio.stderr];
    let mut same_file = None;
    for (ix, (io_file, path)) in io_files.into_iter().zip(out_err.into_iter()).enumerate() {
        // Note that each io_file (stdout/stderr) have to std::mem::forget, in order not to close pipe.
        // Also, third argument corresponds to "not forget writer" for twice use of Fifo, in case of stdout==stderr.
        let dest = |mut writer: Pin<Box<dyn std::io::Write + Unpin + Send>>,
                    r: Option<Fifo>,
                    drop_w: bool| {
            match io_file {
                Some(f) => {
                    debug_log!("readfile: {:?}", f);
                    debug_log!("fifo: {:?}", r);
                    let f = unsafe {std::fs::File::from_raw_fd(f.as_raw_fd()) };
                    let mut reader = BufReader::new(f);
                    // let _ = tokio::io::copy(&mut reader, &mut *writer)?;
                    let _ = std::io::copy(&mut reader, &mut *writer)?;
                    std::mem::forget(reader);
                    drop(r);
                    if !drop_w {
                        std::mem::forget(writer);
                    }
                    debug_log!("finished copy!");
                    Ok(())
                }
                None => {
                    log::error!("{}", FIFO_ERR_MSG[ix]);
                    Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
                }
            }
        };
        // might be ugly hack
        if fifo_noasync::is_fifo(path)? {
            let mut w_fifo = Fifo::open(path, OFlag::O_WRONLY, 0)?;
            let r_fifo = Fifo::open(path, OFlag::O_RDONLY, 0)?;

            debug_log!("spawn task with fifo...");
            debug_log!("read end: {:?}", r_fifo);
            debug_log!("write end: {:?}", w_fifo);
            let w = Box::pin(w_fifo);
            let r = Some(r_fifo);
            dest(w, r, true)?;
        } else if let Some(w) = same_file.take() {
            debug_log!("pipe is not fifo -> use same file for task...");
            dest(w, None, true)?;
            continue;
        } else {
            debug_log!("pipe is not fifo -> new file... {}", path.as_str());
            let fd = fcntl::open(path.as_str(), OFlag::O_WRONLY | OFlag::O_APPEND, Mode::empty())?;
            // ugly hack
            let f = unsafe { std::fs::File::from_raw_fd(fd) };
            // let f = tokio::fs::OpenOptions::new()
            //     .write(true)
            //     .append(true)
            //     .open(&path)
            //     ?;
            let w = Box::pin(f);
            let drop_w = if stdio.stdout == stdio.stderr {
                // might be ugly hack
                let f = unsafe { std::fs::File::from_raw_fd(w.as_raw_fd()) };
                let _ = same_file.get_or_insert(Box::pin(f));
                false
            } else {
                true
            };
            dest(w, None, drop_w)?;
        }
    }
    if stdio.stdin == "" {
        return Ok(());
    }
    let f = Fifo::open(&stdio.stdin, OFlag::O_RDONLY | OFlag::O_NONBLOCK, 0)?;
    // let copy_buf = async move {
    //     let stdin = tokio::fs::File::from(io.stdin().unwrap());
    //     let mut writer = BufWriter::new(stdin);
    //     let mut reader = BufReader::new(f);
    //     match tokio::io::copy(&mut reader, &mut writer) {
    //         Ok(x) => Ok(()),
    //         Err(e) => {
    //             log::error!("{}", e);
    //             Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
    //         }
    //     }
    //     // don't have to forget these reader/writer
    // };
    debug_log!("task spawn");
    // tokio::task::spawn(copy_buf)??;
    debug_log!("task completed");
    Ok(())
}
