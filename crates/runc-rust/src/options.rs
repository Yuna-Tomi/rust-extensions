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

use crate::error::Error;
use std::path::PathBuf;

type ConsoleSocketPath = Option<PathBuf>;

const ALL: &str = "--all";
const CONSOLE_SOCKET: &str = "--console-socket";
const DETACH: &str = "--detach";
const FORCE: &str = "--force";
const NO_NEW_KEYRING: &str = "--no-new-keyring";
const NO_PIVOT: &str = "--no-pivot";
const PID_FILE: &str = "--pid-file";

pub trait Args {
    type Output;
    fn args(&self) -> Self::Output;
}

#[derive(Debug, Clone)]
pub struct CreateOpts {
    /// Path to where a pid file should be created.
    pub pid_file: Option<PathBuf>,
    pub console_socket: ConsoleSocketPath,
    pub detach: bool,
    /// If [`true`], it doesn't use pivot_root to jail process inside rootfs.
    pub no_pivot: bool,
    /// If [`true`], a new session keyring for the container will not be created.
    pub no_new_keyring: bool,
}

impl Args for CreateOpts {
    type Output = Result<Vec<String>, Error>;
    fn args(&self) -> Self::Output {
        let mut args: Vec<String> = vec![];
        if let Some(pid_file) = &self.pid_file {
            args.push(PID_FILE.to_string());
            args.push(
                pid_file
                    .canonicalize()
                    .map_err(|e| Error::InvalidPathError(e))?
                    .to_string_lossy()
                    .parse::<String>()
                    .unwrap(),
            );
        }
        if let Some(console_socket) = &self.console_socket {
            args.push(CONSOLE_SOCKET.to_string());
            args.push(console_socket.to_string_lossy().parse::<String>().unwrap());
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

#[derive(Debug, Clone)]
pub struct ExecOpts {
    /// Path to where a pid file should be created.
    pub pid_file: Option<PathBuf>,
    pub console_socket: ConsoleSocketPath,
    pub detach: bool,
}

impl Args for ExecOpts {
    type Output = Result<Vec<String>, Error>;
    fn args(&self) -> Self::Output {
        let mut args: Vec<String> = vec![];
        if let Some(pid_file) = &self.pid_file {
            args.push(PID_FILE.to_string());
            args.push(
                pid_file
                    .canonicalize()
                    .map_err(|e| Error::InvalidPathError(e))?
                    .to_string_lossy()
                    .parse::<String>()
                    .unwrap(),
            );
        }
        if let Some(console_socket) = &self.console_socket {
            args.push(CONSOLE_SOCKET.to_string());
            args.push(console_socket.to_string_lossy().parse::<String>().unwrap());
        }
        if self.detach {
            args.push(DETACH.to_string());
        }
        Ok(args)
    }
}

#[derive(Debug, Clone)]
pub struct DeleteOpts {
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

#[derive(Debug, Clone)]
pub struct KillOpts {
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
