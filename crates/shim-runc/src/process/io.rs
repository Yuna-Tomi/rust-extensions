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

use std::ffi::OsStr;
use std::fs::DirBuilder;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;

use nix::fcntl::OFlag;
use runc::io::{IOOption, Io, NullIo, PipedIo};
use tokio::io::{AsyncWrite, BufReader, BufWriter};
use url::{ParseError, Url};

use super::config::StdioConfig;
use super::fifo::{self, Fifo};

use crate::dbg::*;

#[derive(Debug, Clone, Default)]
pub struct ProcessIO {
    pub io: Option<Arc<dyn Io>>,
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
        // Only NullIo is supported now.
        // return Ok(Self {
        //     io: Some(Arc::new(NullIo::new()?)),
        //     copy: false,
        //     stdio,
        //     ..Default::default()
        // });

        // FIXME: Appropriate IO settings...
        if stdio.is_null() {
            return Ok(Self {
                io: Some(Arc::new(NullIo::new()?)),
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
                let io = Arc::new(PipedIo::new(
                    io_uid as u32,
                    io_gid as u32,
                    conditional_io_options(&stdio),
                )?);
                Ok(Self {
                    io: Some(io as Arc<dyn Io>),
                    uri: Some(u),
                    copy: true,
                    stdio,
                })
            }
            "binary" => {
                // FIXME: appropriate binary io
                unimplemented!()
                // Ok(Self {
                //     io: Some(Box::new(BinaryIO::new("dummy")?) as Box<dyn Io>),
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
                let io = Arc::new(PipedIo::new(
                    io_uid as u32,
                    io_gid as u32,
                    conditional_io_options(&stdio),
                )?);
                Ok(Self {
                    io: Some(io as Arc<dyn Io>),
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
    pub fn io(&self /* , cmd: &mut std::process::Command */) -> Option<Arc<dyn Io>> {
        if let Some(io) = &self.io {
            Some(io.clone())
        } else {
            None
        }
    }

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
    // out: Pipe,
}

// FIXME: suspended for difficulties.
impl Io for BinaryIO {
    fn stdin(&self) -> Option<std::fs::File> {
        unimplemented!()
    }

    fn stderr(&self) -> Option<std::fs::File> {
        unimplemented!()
    }

    fn stdout(&self) -> Option<std::fs::File> {
        unimplemented!()
    }

    fn set(&self, _cmd: &mut Command) -> std::io::Result<()> {
        unimplemented!()
    }

    fn set_tk(&self, _cmd: &mut tokio::process::Command) -> std::io::Result<()> {
        unimplemented!()
    }

    fn close_after_start(&self) {
        unimplemented!()
    }
}

impl BinaryIO {
    pub fn new(path: impl AsRef<OsStr>) -> std::io::Result<Self> {
        Ok(Self {
            cmd: Some(Arc::new(Command::new(path))),
            // out: Pipe::new()?,
        })
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

// In this function, each spawened tasks are expected to be lived
// until related process will be deleted. Then this function doesn't "join"
// Each "copy" on task will continuously copy data between
// pipe that containered arranged and processIO that connected to runc process
async fn copy_pipes(io: Arc<dyn Io>, stdio: &StdioConfig) -> std::io::Result<()> {
    let io_files = vec![io.stdout(), io.stderr()];

    // debug_log!("io files: {:?}", io_files);
    let out_err = vec![stdio.stdout.clone(), stdio.stderr.clone()];
    let mut same_file = None;
    for (ix, (rd, path)) in io_files.into_iter().zip(out_err.into_iter()).enumerate() {
        // Note that each io_file (stdout/stderr) have to std::mem::forget, in order not to close pipe.
        // Also, third argument corresponds to "not forget writer" for twice use of Fifo, in case of stdout==stderr.
        let dest = |mut writer: Pin<Box<dyn AsyncWrite + Unpin + Send>>,
                    reader: Option<std::fs::File>,
                    closer: Option<Fifo>,
                    ix: usize| async move {
            match reader {
                Some(f) => {
                    let f = tokio::fs::File::from_std(f);
                    let mut reader = BufReader::new(f);
                    use std::panic::set_hook;
                    set_hook(Box::new(|e| log::error!("panic on copy pipe: {}", e)));
                    debug_log!("{} reader: {:?}\ncloser: {:?}", FIFO[ix], reader, closer);
                    let x = tokio::io::copy(&mut reader, &mut *writer).await?;
                    debug_log!("{} copy: {} bytes", FIFO[ix], x);
                    // Note that "closer" will drop at the end of this task and fd will be closed.
                    // here, explicitly drop just for easy to understand
                    drop(closer);
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
            let _t = tokio::task::spawn(async move {
                let w_fifo = Fifo::open(&path, OFlag::O_WRONLY, 0).await.map_err(|e| {
                    debug_log!("error in make w_fifo {}", e);
                    e
                })?;

                let r_fifo = Fifo::open(&path, OFlag::O_RDONLY, 0).await.map_err(|e| {
                    debug_log!("error in make w_fifo {}", e);
                    e
                })?;
                let wr = Box::pin(w_fifo);
                let cl = Some(r_fifo);
                debug_log!(
                    "spawn task with fifo...\nread end: {:?}\nwrite end: {:?}",
                    wr,
                    rd
                );
                dest(wr, rd, cl, ix).await
            });
        } else if let Some(wr) = same_file.take() {
            debug_log!("pipe is not fifo -> use same file for task...");
            let _t = tokio::task::spawn(async move { dest(wr, rd, None, ix) });
            continue;
        } else {
            debug_log!("pipe is not fifo -> new file... {}", path.as_str());
            let drop_w = stdio.stdout == stdio.stderr;
            let f = tokio::fs::OpenOptions::new()
                .write(true)
                .append(true)
                .mode(0)
                .open(&path)
                .await?;
            if drop_w {
                let f = f.try_clone().await?;
                let _ = same_file.get_or_insert(Box::pin(f));
            }
            let wr = Box::pin(f);
            let _t = tokio::task::spawn(async move {
                use std::panic::set_hook;
                set_hook(Box::new(|e| log::error!("panic on stdin copy pipe: {}", e)));
                dest(wr, rd, None, ix).await
            });
        }
    }

    if stdio.stdin != "" {
        let io = io.clone();
        let stdin = stdio.stdin.clone();
        let copy_buf = async move {
            let f = Fifo::open(&stdin, OFlag::O_RDONLY | OFlag::O_NONBLOCK, 0).await?;
            let stdin = io.stdin().unwrap();
            let stdin = tokio::fs::File::from_std(stdin);
            debug_log!("stdin write end: {:?}\nstdin read end: {:?}", stdin, f);
            let mut writer = BufWriter::new(stdin);
            let mut reader = BufReader::new(f);
            debug_log!("stdin writer: {:?}\nstdin reader: {:?}", writer, reader);
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
        let _t = tokio::task::spawn(copy_buf);
    }
    Ok(())
}
