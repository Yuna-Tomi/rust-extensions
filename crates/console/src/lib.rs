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

mod ioctl;

#[cfg(feature = "tokio_imp")]
mod tokio_imp;

#[cfg(feature = "futures_imp")]
pub mod futures_imp;

use std::io::{self, Read, Write};
use std::os::unix::prelude::{AsRawFd, FromRawFd, IntoRawFd};
use std::sync::Arc;
use std::os::unix::prelude::RawFd;

use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::libc::c_ushort;
use nix::pty::{self, OpenptyResult};
use nix::sys::termios::{self, LocalFlags, SetArg, Termios};
use thiserror::Error;

type Result<T> = std::result::Result<T, Error>;

/// Manages master side of pseudo terminal
#[derive(Debug)]
pub struct Master<F: AsRawFd> {
    inner: F,
    /// reserving the original settings when instance of this struct generated
    original: Termios,
}

pub trait Console {
    fn resize(&self, size: WinSize) -> Result<()>;

    fn resize_from(&self, console: Arc<dyn Console>) -> Result<()> {
        let size = console.get_size()?;
        self.resize(size)
    }

    fn set_raw(&self) -> Result<()>;
    fn get_size(&self) -> Result<WinSize>;
    fn disable_echo(&self) -> Result<()>;
    fn reset(&self) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct WinSize {
    height: c_ushort,
    width: c_ushort,
    x: c_ushort,
    y: c_ushort,
}

impl From<nix::pty::Winsize> for WinSize {
    fn from(size: nix::pty::Winsize) -> Self {
        Self {
            height: size.ws_row,
            width: size.ws_col,
            x: size.ws_xpixel,
            y: size.ws_ypixel,
        }
    }
}

impl Into<nix::pty::Winsize> for WinSize {
    fn into(self) -> nix::pty::Winsize {
        nix::pty::Winsize {
            ws_row: self.height,
            ws_col: self.width,
            ws_xpixel: self.x,
            ws_ypixel: self.y,
        }
    }
}

impl<F: AsRawFd> Master<F> {
    pub fn new(inner: F) -> Result<Self> {
        let original = termios::tcgetattr(inner.as_raw_fd())?;
        Ok(Self { inner, original })
    }

    pub fn fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl<F: AsRawFd> Console for Master<F> {
    fn disable_echo(&self) -> Result<()> {
        let mut cur = termios::tcgetattr(self.fd())?;
        cur.local_flags &= !LocalFlags::ECHO;
        termios::tcsetattr(self.fd(), SetArg::TCSANOW, &cur)?;
        Ok(())
    }

    fn resize(&self, size: WinSize) -> Result<()> {
        ioctl::set_winsize(self.fd(), &size.into())
    }

    // #[cfg(not(any(target_os = "solaris", target_os = "illumos")))]
    // fn set_raw(&self) -> Result<()> {
    //     let mut cur = termios::tcgetattr(self.fd())?;
    //     termios::cfmakeraw(&mut cur);
    //     Ok(())
    // }

    // #[cfg(any(target_os = "solaris", target_os = "illumos"))]
    fn set_raw(&self) -> Result<()> {
        use nix::libc;
        use nix::sys::termios::{ControlFlags, InputFlags, OutputFlags};

        let mut cur = termios::tcgetattr(self.fd())?;
        cur.input_flags &= !(InputFlags::BRKINT
            | InputFlags::ICRNL
            | InputFlags::INLCR
            | InputFlags::IGNCR
            | InputFlags::INPCK
            | InputFlags::ISTRIP
            | InputFlags::IXON);
        cur.output_flags &= !OutputFlags::OPOST;
        cur.local_flags &= !(LocalFlags::ECHO
            | LocalFlags::ECHOE
            | LocalFlags::ECHONL
            | LocalFlags::ICANON
            | LocalFlags::IEXTEN
            | LocalFlags::ISIG);
        cur.control_flags &= !(ControlFlags::PARENB | ControlFlags::CSIZE);
        cur.control_flags |= ControlFlags::CS8;
        // VMIN/VTIME in nix cannot be used as index now, using ones in libc instead.
        cur.control_chars[libc::VMIN] = 1;
        cur.control_chars[libc::VTIME] = 0;
        termios::tcsetattr(self.fd(), SetArg::TCSANOW, &cur)?;
        Ok(())
    }

    fn get_size(&self) -> Result<WinSize> {
        Ok(ioctl::get_winsize(self.fd())?.into())
    }

    fn reset(&self) -> Result<()> {
        Ok(termios::tcsetattr(
            self.fd(),
            SetArg::TCSANOW,
            &self.original,
        )?)
    }
}

impl<F: AsRawFd + Read> Read for Master<F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<F: AsRawFd + Write> Write for Master<F> {
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }
}

impl<F: AsRawFd + FromRawFd> FromRawFd for Master<F> {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        match Master::new(F::from_raw_fd(fd)) {
            Ok(m) => m,
            Err(e) => panic!("failed to convert from fd: {}", e),
        }
    }
}

impl<F: AsRawFd + IntoRawFd> IntoRawFd for Master<F> {
    fn into_raw_fd(self) -> RawFd {
        self.inner.into_raw_fd()
    }
}

impl<F: AsRawFd> AsRawFd for Master<F> {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Nix(#[from] Errno),

    #[error(transparent)]
    Io(#[from] io::Error),
}

pub fn get_current<F: AsRawFd + FromRawFd>() -> Result<Master<F>> {
    // Usually all three streams (stdin, stdout, and stderr)
    // are open to the same console, but some might be redirected,
    // so try all three.
    for fd in [
        io::stdin().as_raw_fd(),
        io::stdout().as_raw_fd(),
        io::stderr().as_raw_fd(),
    ] {
        match termios::tcgetattr(fd) {
            Ok(original) => {
                let inner = unsafe { F::from_raw_fd(fd) };
                return Ok(Master { inner, original });
            }
            Err(_) => continue,
        }
    }
    Err(io::Error::from(io::ErrorKind::NotFound).into())
}

/// create new pty pair
/// Return value is [`Master`] that contains the master side and [`F`] of slave.
/// Note that this internally uses [`openpty(3)`] and slave end is already opened when this function returns (and has succeeded)
pub fn new_pty_pair<F: AsRawFd + FromRawFd>() -> Result<(Master<F>, F)> {
    // let mst = pty::posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY | OFlag::O_CLOEXEC )?;
    // pty::grantpt(&mst)?;
    // pty::unlockpt(&mst)?;
    // let slv = ptsname(&mst)?;
    // let mst = unsafe { File::from_raw_fd(mst.into_raw_fd()) };
    // let slv = OpenOptions::new()
    //     .read(true)
    //     .write(true)
    //     .mode(0)
    //     .open(&slv)?;
    let OpenptyResult { master, slave } = pty::openpty(None, None)?;
    let mst = unsafe { F::from_raw_fd(master) };
    let slv = unsafe { F::from_raw_fd(slave) };
    let mst = Master::new(mst)?;
    Ok((mst, slv))
}

#[cfg(not(target_os = "linux"))]
use {
    once_cell::sync::Lazy,
    std::sync::Mutex,
};
#[cfg(not(target_os = "linux"))]
static PTSNAME_MUTEX: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// This uses posix_openpt(3), grantpt(3), unlockpt(3) and ptsname(3) (on Linux, [`ptsname_r(3)`] instead.)
/// When this function returns, slave end is not opened ever.
pub fn new_pty_pair2<F: AsRawFd + FromRawFd>() -> Result<(Master<F>, String)> {
    let mst = pty::posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY | OFlag::O_CLOEXEC)?;
    pty::grantpt(&mst)?;
    pty::unlockpt(&mst)?;

    #[cfg(not(target_os = "linux"))]
    let slv = {
        // ptsname is MT-unsafe (mutates the file path placed in global field), then take mutex here
        let _m = PTSNAME_MUTEX.lock().unwrap();
        unsafe { pty::ptsname(&mst)? }
    };

    #[cfg(target_os = "linux")]
    let slv = pty::ptsname_r(&mst)?;

    let mst = unsafe { F::from_raw_fd(mst.into_raw_fd()) };
    // FIXME: internal tcgetattr(3) in this fails on macOS, while isattr(fd of mst) returns 1(true).
    let mst = Master::new(mst)?;
    Ok((mst, slv))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::{BufRead, BufReader};
    use std::thread;

    #[test]
    fn test() {
        let mst = get_current::<File>().expect("cannot extract master");
        let size = WinSize {
            height: 10,
            width: 10,
            x: 10,
            y: 10,
        };
        mst.resize(size).expect("cannot resize.");
        mst.disable_echo().expect("failed to disable echo.");
        assert_eq!(size, mst.get_size().expect("cannot get size."));
    }

    #[test]
    fn test2() {
        let (mst, mut slv) = new_pty_pair::<File>().expect("cannot allocat pty.");
        let t1 = thread::spawn(move || {
            let msg = "Hello, console!\n".to_string();
            let msg2 = "For containerd!\n".to_string();
            slv.write_all(msg.as_bytes())
                .expect("cannot write message.");
            slv.write_all(msg2.as_bytes())
                .expect("cannot write message.");
        });

        let t2 = thread::spawn(move || {
            let mut msg = String::new();
            let mut msg2 = String::new();
            let mut mst = BufReader::new(mst);
            // NOTE: if you sleep here for even 1 sec, it will lost data on macOS
            // but will not in Linux when sleep for even 5 sec.
            // The author didn't find reason now and is middle of survey...
            // pty is platform sensitive and these test have to make sure
            // this crate works well if user use it correctly
            mst.read_line(&mut msg).expect("cannot read message.");
            mst.read_line(&mut msg2).expect("cannot read message 2.");
            (msg, msg2)
        });

        t1.join().unwrap();
        let (msg, msg2) = t2.join().unwrap();
        assert_eq!("Hello, console!\r\n", msg);
        assert_eq!("For containerd!\r\n", msg2);
    }
}
