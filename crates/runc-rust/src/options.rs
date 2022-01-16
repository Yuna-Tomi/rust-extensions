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

// Forked from https://github.com/pwFoo/rust-runc/blob/master/src/lib.rs
/*
 * Copyright 2020 fsyncd, Berlin, Germany.
 * Additional material, copyright of the containerd authors.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use crate::error::Error;
use crate::utils::{self, ALL, CONSOLE_SOCKET, DETACH, FORCE, NO_NEW_KEYRING, NO_PIVOT, PID_FILE};
use std::path::{Path, PathBuf};

pub trait Args {
    type Output;
    fn args(&self) -> Self::Output;
}

#[derive(Debug, Clone, Default)]
pub struct CreateOpts {
    /// Path to where a pid file should be created.
    pub pid_file: Option<PathBuf>,
    /// Path to where a console socket should be created.
    pub console_socket: Option<PathBuf>,
    /// Detach from the container's process (only available for run)
    pub detach: bool,
    /// Don't use pivot_root to jail process inside rootfs.
    pub no_pivot: bool,
    /// A new session keyring for the container will not be created.
    pub no_new_keyring: bool,
}

impl Args for CreateOpts {
    type Output = Result<Vec<String>, Error>;
    fn args(&self) -> Self::Output {
        let mut args: Vec<String> = vec![];
        if let Some(pid_file) = &self.pid_file {
            args.push(PID_FILE.to_string());
            args.push(utils::abs_string(pid_file)?);
        }
        if let Some(console_socket) = &self.console_socket {
            args.push(CONSOLE_SOCKET.to_string());
            args.push(utils::abs_string(console_socket)?);
        }
        if self.no_pivot {
            args.push(NO_PIVOT.to_string());
        }
        if self.no_new_keyring {
            args.push(NO_NEW_KEYRING.to_string());
        }
        if self.detach {
            args.push(DETACH.to_string());
        }
        Ok(args)
    }
}

impl CreateOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pid_file(&mut self, pid_file: impl AsRef<Path>) -> &mut Self {
        self.pid_file = Some(pid_file.as_ref().to_path_buf());
        self
    }

    pub fn console_socket(&mut self, console_socket: impl AsRef<Path>) -> &mut Self {
        self.console_socket = Some(console_socket.as_ref().to_path_buf());
        self
    }

    pub fn detach(&mut self, detach: bool) -> &mut Self {
        self.detach = detach;
        self
    }

    pub fn no_pivot(&mut self, no_pivot: bool) -> &mut Self {
        self.no_pivot = no_pivot;
        self
    }

    pub fn no_new_keyring(&mut self, no_new_keyring: bool) -> &mut Self {
        self.no_new_keyring = no_new_keyring;
        self
    }
}

/// Container execution options
#[derive(Debug, Clone, Default)]
pub struct ExecOpts {
    /// Path to where a pid file should be created.
    pub pid_file: Option<PathBuf>,
    /// Path to where a console socket should be created.
    pub console_socket: Option<PathBuf>,
    /// Detach from the container's process (only available for run)
    pub detach: bool,
}

impl Args for ExecOpts {
    type Output = Result<Vec<String>, Error>;
    fn args(&self) -> Self::Output {
        let mut args: Vec<String> = vec![];
        if let Some(pid_file) = &self.pid_file {
            args.push(PID_FILE.to_string());
            args.push(utils::abs_string(pid_file)?);
        }
        if let Some(console_socket) = &self.console_socket {
            args.push(CONSOLE_SOCKET.to_string());
            args.push(utils::abs_string(console_socket)?);
        }
        if self.detach {
            args.push(DETACH.to_string());
        }
        Ok(args)
    }
}

impl ExecOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pid_file(&mut self, pid_file: impl AsRef<Path>) -> &mut Self {
        self.pid_file = Some(pid_file.as_ref().to_path_buf());
        self
    }

    pub fn console_socket(&mut self, console_socket: impl AsRef<Path>) -> &mut Self {
        self.console_socket = Some(console_socket.as_ref().to_path_buf());
        self
    }

    pub fn detach(&mut self, detach: bool) -> &mut Self {
        self.detach = detach;
        self
    }
}

/// Container deletion options
#[derive(Debug, Clone, Default)]
pub struct DeleteOpts {
    /// Forcibly delete the container if it is still running
    pub force: bool,
}

impl Args for DeleteOpts {
    type Output = Result<Vec<String>, Error>;
    fn args(&self) -> Self::Output {
        let mut args: Vec<String> = vec![];
        if self.force {
            args.push(FORCE.to_string());
        }
        Ok(args)
    }
}

impl DeleteOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn force(&mut self, force: bool) -> &mut Self {
        self.force = force;
        self
    }
}

/// Container killing options
#[derive(Debug, Clone, Default)]
pub struct KillOpts {
    /// Seng the kill signal to all the processes inside the container
    pub all: bool,
}

impl Args for KillOpts {
    type Output = Result<Vec<String>, Error>;
    fn args(&self) -> Self::Output {
        let mut args: Vec<String> = vec![];
        if self.all {
            args.push(ALL.to_string());
        }
        Ok(args)
    }
}

impl KillOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn all(&mut self, all: bool) -> &mut Self {
        self.all = all;
        self
    }
}
