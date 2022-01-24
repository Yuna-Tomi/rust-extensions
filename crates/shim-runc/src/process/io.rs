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
use containerd_runc_rust as runc;
use runc::io::{
    RuncIO, RuncPipedIO, IOOption,
};
use url::{Url, ParseError};
use std::{sync::{Arc, RwLock}, process::Command, os::unix::{fs::DirBuilderExt, prelude::RawFd}, fs::{OpenOptions, File}, ffi::OsStr};
use std::path::Path;
use std::fs::DirBuilder;

use crate::dbg::*;

#[derive(Debug, Clone, Default)]
pub struct StdioConfig {
    pub stdin: String,
    pub stdout: String,
    pub stderr: String,
    pub terminal: bool,
}

impl StdioConfig {
    pub fn is_null(&self) -> bool  {
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
    pub fn new(id: &str, io_uid: isize, io_gid: isize, stdio: StdioConfig)  -> std::io::Result<Self> {
        if stdio.is_null() {
            return Ok(Self {
                copy: false, stdio, ..Default::default()
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
                        io_uid, io_gid, conditional_io_options(&stdio)
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
                    io_uid, io_gid, conditional_io_options(&stdio)
                )?);
                Ok(Self {
                    io: Some(io as Box<dyn RuncIO>),
                    uri: Some(u),
                    copy: true,
                    stdio,
                })
            }
            _ => Err(std::io::Error::from(std::io::ErrorKind::NotFound))
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
}

#[derive(Clone)]
pub struct BinaryIO {
    cmd: Option<Arc<Command>>,
    out: Pipe,
}

// FIXME: suspended
impl RuncIO for BinaryIO {
    fn stdin(&self) -> Option<File> {
        panic!("unimplemented");
    }

    fn stderr(&self) -> Option<File> {
        panic!("unimplemented")
    }

    fn stdout(&self) -> Option<File> {
        panic!("unimplemented")
    }

    fn close(&mut self) {
        panic!("unimplemented")
    }

    fn set(&self, cmd: &mut Command) {
        panic!("unimplemented")
    }

}

impl BinaryIO {
    pub fn new(path: impl AsRef<OsStr>) -> std::io::Result<Self> {
        Ok(
            Self {
                cmd: Some(Arc::new(Command::new(path))),
                out: Pipe::new()?,
            }
        )
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