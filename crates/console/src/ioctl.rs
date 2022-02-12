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

#![cfg(target_os = "linux")]
//! This module provides wrappers of ioctl

use nix::errno::Errno;
use nix::libc;
use std::mem::MaybeUninit;
use std::os::unix::prelude::RawFd;

use crate::{Result, WinSize};

pub fn get_winsize(fd: RawFd) -> Result<nix::pty::Winsize> {
    let mut size = MaybeUninit::<nix::pty::Winsize>::uninit();
    unsafe {
        let res = libc::ioctl(fd, libc::TIOCGWINSZ, size.as_mut_ptr() as *mut u8);
        Errno::result(res)?;
        Ok(size.assume_init())
    }
}

pub fn set_winsize(fd: RawFd, size: &nix::pty::Winsize) -> Result<()> {
    let ptr = size as *const nix::pty::Winsize;
    unsafe {
        let res = libc::ioctl(fd, libc::TIOCSWINSZ, ptr as *const u8);
        Errno::result(res)?;
        Ok(())
    }
}
