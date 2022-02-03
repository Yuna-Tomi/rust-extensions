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
use std::fmt::{self, Debug, Formatter};
use std::fs::File;
use std::os::unix::io::FromRawFd;
use std::os::unix::prelude::{AsRawFd, RawFd};
use std::process::Command;
use std::sync::Mutex;

use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use nix::unistd::{Gid, Uid};

use crate::dbg::*;

/// Users have to [`std::mem::forget()`] to prevent from closing fds when this return value drops.
/// Especially, in such situation, you have to [`std::mem::forget()`] the [`std::process::Command`] you passed to the [`set()`].
pub trait RuncIO: Sync + Send {
    /// Return write side of stdin
    fn stdin(&self) -> Option<File> {
        None
    }

    /// Return read side of stdout
    fn stdout(&self) -> Option<File> {
        None
    }

    /// Return read side of stderr
    fn stderr(&self) -> Option<File> {
        None
    }

    fn close(&self) {
        debug_log!("close unimplemented!");
        panic!("close unimplemented!");
    }

    /// Set IO for passed command.
    /// Read side of stdin, write side of stdout and write side of stderr should be provided to command.
    fn set(&self, _cmd: &mut Command) -> std::io::Result<()> {
        debug_log!("set unimplemented!");
        panic!("set unimplemented!");
    }

    // tokio version of set()
    fn set_tk(&self, _cmd: &mut tokio::process::Command) -> std::io::Result<()> {
        debug_log!("set_tk unimplemented!");
        panic!("set_tk unimplemented!");
    }

    fn close_after_start(&self) {
        debug_log!("close_after_start unimplemented!");
        panic!("close_after_start unimplemented!");
    }
}

// dyn_clone::clone_trait_object!(RuncIO);

impl Debug for dyn RuncIO {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // it's not good idea to call std~~() when debug.
        write!(f, "RuncIO",)
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

/// This struct represents pipe that can be used to transfer
/// stdio inputs and outputs
/// when one side of closed, this struct represent it with [`None`]
#[derive(Debug)]
pub struct Pipe {
    // Might be ugly hack: using mutex in order to take rd/wr under immutable [`Pipe`]
    rd: Mutex<Option<File>>,
    wr: Mutex<Option<File>>,
}

impl Pipe {
    pub fn new() -> std::io::Result<Self> {
        let (r, w) = nix::unistd::pipe()?;
        let (rd, wr) = unsafe {
            (
                Mutex::new(Some(File::from_raw_fd(r))),
                Mutex::new(Some(File::from_raw_fd(w))),
            )
        };
        debug_log!("new pipe: rd={:?}, wr={:?}", rd, wr);
        Ok(Self { rd, wr })
    }

    pub fn take_read(&self) -> Option<File> {
        let mut m = self.rd.lock().unwrap();
        m.take()
    }

    pub fn take_write(&self) -> Option<File> {
        let mut m = self.wr.lock().unwrap();
        m.take()
    }

    pub fn close_read(&self) {
        let mut m = self.rd.lock().unwrap();
        let _ = m.take();
    }

    pub fn close_write(&self) {
        let mut m = self.wr.lock().unwrap();
        let _ = m.take();
    }

    pub fn close(&self) {
        self.close_read();
        self.close_write();
    }
}

#[derive(Debug)]
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
            {
                let m = pipe.rd.lock().unwrap();
                if let Some(f) = m.as_ref() {
                    nix::unistd::fchown(f.as_raw_fd(), uid, gid)?;
                }
            }
            Some(pipe)
        } else {
            None
        };

        let stdout = if opts.open_stdout {
            let pipe = Pipe::new()?;
            {
                let m = pipe.wr.lock().unwrap();
                if let Some(f) = m.as_ref() {
                    nix::unistd::fchown(f.as_raw_fd(), uid, gid)?;
                }
            }
            Some(pipe)
        } else {
            None
        };

        let stderr = if opts.open_stderr {
            let pipe = Pipe::new()?;
            {
                let m = pipe.wr.lock().unwrap();
                if let Some(f) = m.as_ref() {
                    nix::unistd::fchown(f.as_raw_fd(), uid, gid)?;
                }
            }
            Some(pipe)
        } else {
            None
        };

        debug_log!("new piped io:\nstdin={:#?}, stdout={:#?}, stderr={:#?}", stdin, stdout, stderr);
        debug_log!("now, fds={:#?}", check_fds!());
        Ok(Self {
            stdin,
            stdout,
            stderr,
        })
    }
}

impl RuncIO for RuncPipedIO {
    fn stdin(&self) -> Option<File> {
        if let Some(ref stdin) = self.stdin {
            stdin.take_write()
        } else {
            None
        }
    }

    fn stdout(&self) -> Option<File> {
        if let Some(ref stdout) = self.stdout {
            stdout.take_read()
        } else {
            None
        }
    }

    fn stderr(&self) -> Option<File> {
        if let Some(ref stderr) = self.stderr {
            stderr.take_read()
        } else {
            None
        }
    }

    fn close(&self) {
        if let Some(ref stdin) = self.stdin {
            stdin.close();
        }
        if let Some(ref stdout) = self.stdout {
            stdout.close();
        }
        if let Some(ref stderr) = self.stderr {
            stderr.close();
        }
    }

    /// Note that this internally use [`std::fs::File`]'s [`try_clone()`].
    /// Thus, the files passed to commands will be not closed after command exit.
    fn set(&self, cmd: &mut Command) -> std::io::Result<()> {
        if let Some(ref p) = self.stdin {
            let m = p.rd.lock().unwrap();
            if let Some(stdin) = &*m {
                let f = stdin.try_clone()?;
                debug_log!("set read end for stdin: {:?}", f);
                cmd.stdin(f);
            }
        }

        if let Some(ref p) = self.stdout {
            let m = p.wr.lock().unwrap();
            if let Some(f) = &*m {
                let f = f.try_clone()?;
                debug_log!("set read end for stdout: {:?}", f);
                cmd.stdout(f);
            }
        }

        if let Some(ref p) = self.stderr {
            let m = p.wr.lock().unwrap();
            if let Some(f) = &*m {
                let f = f.try_clone()?;
                debug_log!("set read end for stderr: {:?}", f);
                cmd.stderr(f);
            }
        }
        debug_log!("fds: {:#?}", check_fds!());

        Ok(())
    }

    fn set_tk(&self, cmd: &mut tokio::process::Command) -> std::io::Result<()> {
        if let Some(ref p) = self.stdin {
            let m = p.rd.lock().unwrap();
            if let Some(stdin) = &*m {
                let f = stdin.try_clone()?;
                // debug_log!("set read end for stdin: {:?}", f);
                cmd.stdin(f);
            }
        }

        if let Some(ref p) = self.stdout {
            let m = p.wr.lock().unwrap();
            if let Some(f) = &*m {
                let f = f.try_clone()?;
                // debug_log!("set read end for stdout: {:?}", f);
                cmd.stdin(f);
            }
        }

        if let Some(ref p) = self.stderr {
            let m = p.wr.lock().unwrap();
            if let Some(f) = &*m {
                let f = f.try_clone()?;
                // debug_log!("set read end for stderr: {:?}", f);
                cmd.stdin(f);
            }
        }
        debug_log!("fds={:#?}", check_fds!());
        Ok(())
    }

    /// closing only write side (should be stdout/err "from" runc process)
    fn close_after_start(&self) {
        if let Some(ref p) = self.stdout {
            p.close_write();
        }
        if let Some(ref p) = self.stderr {
            p.close_write();
        }
    }
}

// IO setup for /dev/null use with runc
#[derive(Debug)]
pub struct NullIO {
    dev_null: RawFd,
}

impl NullIO {
    pub fn new() -> std::io::Result<Self> {
        let fd = nix::fcntl::open("/dev/null", OFlag::O_RDONLY, Mode::empty())?;
        // let dev_null = unsafe { Some(std::fs::File::from_raw_fd(fd)) };
        // Ok(Self { dev_null })
        Ok(Self { dev_null: fd })
    }
}

impl RuncIO for NullIO {
    fn set(&self, cmd: &mut Command) -> std::io::Result<()> {
        let null = unsafe { std::fs::File::from_raw_fd(self.dev_null) };
        cmd.stdout(null.try_clone()?);
        cmd.stderr(null.try_clone()?);
        std::mem::forget(null);
        Ok(())
    }

    fn set_tk(&self, cmd: &mut tokio::process::Command) -> std::io::Result<()> {
        let null = unsafe { std::fs::File::from_raw_fd(self.dev_null) };
        cmd.stdout(null.try_clone()?);
        cmd.stderr(null.try_clone()?);
        std::mem::forget(null);
        Ok(())
    }

    fn close(&self) {
        let _ = unsafe { std::fs::File::from_raw_fd(self.dev_null) };
    }

    fn close_after_start(&self) {
        self.close();
    }
}
