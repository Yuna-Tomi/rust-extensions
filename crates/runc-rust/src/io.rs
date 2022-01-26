use std::os::unix::prelude::RawFd;
/*
   copyright the containerd authors.

   licensed under the apache license, version 2.0 (the "license");
   you may not use this file except in compliance with the license.
   you may obtain a copy of the license at

       http://www.apache.org/licenses/license-2.0

   unless required by applicable law or agreed to in writing, software
   distributed under the license is distributed on an "as is" basis,
   without warranties or conditions of any kind, either express or implied.
   see the license for the specific language governing permissions and
   limitations under the license.
*/
use dyn_clone::DynClone;
use nix::unistd::{Gid, Uid};
use std::fmt::{self, Debug, Formatter};
use std::fs::File;
use std::os::unix::io::FromRawFd;
use std::process::{Command, Stdio};

use crate::dbg::*;

/// Users have to [`std::mem::forget()`] to prevent from closing fds when this return value drops.
/// Especially, in such situation, you have to [`std::mem::forget()`] the [`std::process::Command`] you passed to the [`set()`].
pub trait RuncIO: DynClone + Sync + Send {
    fn stdin(&self) -> Option<File>;
    fn stdout(&self) -> Option<File>;
    fn stderr(&self) -> Option<File>;
    fn close(&mut self);
    fn set(&self, cmd: &mut Command) ;
}

dyn_clone::clone_trait_object!(RuncIO);

impl Debug for dyn RuncIO {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // it's not good idea to call std~~() when debug.
        write!(
            f,
            "RuncIO",
        )
    }
}

#[derive(Debug, Clone)]
pub struct IOOption {
    pub open_stdin: bool,
    pub open_stdout: bool,
    pub open_stderr: bool,
}

impl Default for IOOption {
    fn default() -> Self {
        Self {
            open_stdin: true,
            open_stdout: true,
            open_stderr: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RuncPipedIO {
    stdin: Option<Pipe>,
    stdout: Option<Pipe>,
    stderr: Option<Pipe>,
}

impl RuncPipedIO {
    pub fn new(uid: isize, gid: isize, opts: IOOption) -> std::io::Result<Self> {
        let uid = Some(Uid::from_raw(uid as u32));
        let gid = Some(Gid::from_raw(gid as u32));
        let stdin = if opts.open_stdin {
            let pipe = Pipe::new()?;
            nix::unistd::fchown(pipe.read_fd, uid, gid)?;
            Some(pipe)
        } else {
            None
        };

        let stdout = if opts.open_stdout {
            let pipe = Pipe::new()?;
            nix::unistd::fchown(pipe.write_fd, uid, gid)?;
            Some(pipe)
        } else {
            None
        };

        let stderr = if opts.open_stderr {
            let pipe = Pipe::new()?;
            nix::unistd::fchown(pipe.write_fd, uid, gid)?;
            Some(pipe)
        } else {
            None
        };

        Ok(Self {
            stdin,
            stdout,
            stderr,
        })
    }
}

impl RuncIO for RuncPipedIO {
    fn stdin(&self) -> Option<File> {
        if let Some(stdin) = &self.stdin {
            Some(unsafe { File::from_raw_fd(stdin.write_fd) })
        } else {
            None
        }
    }

    fn stdout(&self) -> Option<File> {
        if let Some(stdout) = &self.stdout {
            Some(unsafe { File::from_raw_fd(stdout.read_fd) })
        } else {
            None
        }
    }

    fn stderr(&self) -> Option<File> {
        if let Some(stderr) = &self.stderr {
            Some(unsafe { File::from_raw_fd(stderr.read_fd) })
        } else {
            None
        }
    }

    fn close(&mut self) {
        if let Some(stdin) = &self.stdin {
            unsafe { stdin.close() };
        }
        if let Some(stdout) = &self.stdout {
            unsafe { stdout.close() };
        }
        if let Some(stderr) = &self.stderr {
            unsafe { stderr.close() };
        }
    }

    fn set(&self, cmd: &mut Command) {
        if let Some(stdin) = &self.stdin {
            let f = unsafe { File::from_raw_fd(stdin.read_fd) };
            cmd.stdin(f);
        }
        if let Some(stdout) = &self.stdout {
            let f = unsafe { File::from_raw_fd(stdout.write_fd) };
            cmd.stdout(f);
        }
        if let Some(stderr) = &self.stderr {
            let f = unsafe { File::from_raw_fd(stderr.write_fd) };
            cmd.stderr(f);
        }
    }
}


#[derive(Debug, Clone)]
pub struct Pipe {
    // might be ugly hack: use rawfd, insted of file to allow clone
    read_fd: RawFd,
    write_fd: RawFd,
}

impl Pipe {
    pub fn new() -> std::io::Result<Self> {
        let (read_fd, write_fd) = nix::unistd::pipe()?;
        unsafe {
            let fr = File::from_raw_fd(read_fd);
            let fw = File::from_raw_fd(write_fd);
            debug_log!("read end for pipe: {:?}", fr);
            debug_log!("write end for pipe: {:?}", fw);
            std::mem::forget(fr);
            std::mem::forget(fw);
            // std::mem::forget(File::from_raw_fd(read_fd));
            // std::mem::forget(File::from_raw_fd(write_fd));
        }
        Ok(Self { read_fd, write_fd })
    }

    pub fn read_fd(&self) -> RawFd {
        self.read_fd
    }

    pub fn write_fd(&self) -> RawFd {
        self.write_fd
    }

    pub unsafe fn close(&self) {
        let _ = File::from_raw_fd(self.read_fd);
        let _ = File::from_raw_fd(self.write_fd);
    }
}
