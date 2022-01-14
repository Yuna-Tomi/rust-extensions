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

//! A crate for consuming the runc binary in your Rust applications, similar to [go-runc](https://github.com/containerd/go-runc) for Go.

use crate::container::Container;
use crate::error::Error;
use crate::options::*;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub mod console;
pub mod container;
pub mod error;
pub mod events;
pub mod monitor;
pub mod options;
pub mod utils;

pub mod api {
    use crate::console::*;
    use crate::container::*;
    use crate::container::*;
    use crate::events::*;
    use crate::monitor::*;
}

const NONE: &str = "";
const JSON: &str = "json";
const TEXT: &str = "text";
const DEFAULT_COMMAND: &str = "runc";

#[derive(Debug, Clone)]
pub struct Version {
    pub runc_version: Option<String>,
    pub spec_version: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone)]
pub enum LogFormat {
    Json,
    Text,
}

/// Configuration for runc client.
///
/// This struct provide chaining interface like, for example, [`std::fs::OpenOptions`].
/// Note that you cannot access the members of RuncConfig directly.
///
/// # Example
///
/// ```no_run
/// use containerd_runc_rust::api as runc;
///
/// let config = runc::RuncConfig::new()
///     .root("./new_root")
///     .debug(false)
///     .log("/path/to/logfile.json")
///     .log_format(runc::LogFormat::Json)
///     .rootless(true);
/// let client = config.build();
/// ```
#[derive(Debug, Clone, Default)]
pub struct RuncConfig {
    /// This field is set to overrides the name of the runc binary. If [`None`], "runc" is used.
    command: Option<PathBuf>,
    /// Path to root directory of container rootfs.
    root: Option<PathBuf>,
    /// Debug logging. If true, debug level logs are emitted.
    debug: bool,
    /// Path to log file.
    log: Option<PathBuf>,
    /// Specifyng log format. Here, json or text is available. Default is "text" and interpreted as text if [`None`].
    log_format: Option<LogFormat>,
    systemd_cgroup: bool,
    /// Setting of whether using rootless mode or not. If [`None`], "auto" settings is used.
    rootless: Option<bool>,
}

impl RuncConfig {
    fn new() -> Self {
        Self::default()
    }

    fn command(&mut self, command: impl AsRef<Path>) -> &mut Self {
        self.command = Some(command.as_ref().to_path_buf());
        self
    }

    fn root(&mut self, root: impl AsRef<Path>) -> &mut Self {
        self.root = Some(root.as_ref().to_path_buf());
        self
    }

    fn debug(&mut self, debug: bool) -> &mut Self {
        self.debug = debug;
        self
    }

    fn log(&mut self, log: impl AsRef<Path>) -> &mut Self {
        self.log = Some(log.as_ref().to_path_buf());
        self
    }

    fn log_format_json(&mut self) -> &mut Self {
        self.log_format = Some(LogFormat::Json);
        self
    }

    fn log_format_text(&mut self) -> &mut Self {
        self.log_format = Some(LogFormat::Json);
        self
    }

    fn rootless(&mut self, rootless: bool) -> &mut Self {
        self.rootless = Some(rootless);
        self
    }

    fn rootless_auto(&mut self) -> &mut Self {
        let _ = self.rootless.take();
        self
    }

    fn build(self) -> Result<Runc, Error> {
        Ok(Runc {
            command: self.command,
            root: self.root,
            debug: self.debug,
            log: self.log,
            log_format: self.log_format.unwrap_or(LogFormat::Text),
            systemd_cgroup: self.systemd_cgroup,
            rootless: self.rootless,
        })
    }
}

/// Runc client to the runc cli
pub struct Runc {
    command: Option<PathBuf>,
    root: Option<PathBuf>,
    debug: bool,
    log: Option<PathBuf>,
    log_format: LogFormat,
    systemd_cgroup: bool,
    // FIXME: implementation of pdeath_signal is suspended due to difficulties, maybe it's favorable to use signal-hook crate.
    // pdeath_signal: ,
    rootless: Option<bool>,
    // FIXME: implementation of extra_args is suspended due to difficulties.
    // extra_args: Vec<String>,
}

impl Runc {
    #[cfg(target_os = "linux")]
    pub async fn command(&self, args: &[String]) -> Result<(), Error> {
        Err(Error::UnimplementedError("command".to_string()))
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn command(&self, args: &[String]) -> Result<(), Error> {
        Err(Error::UnimplementedError("command".to_string()))
    }

    pub async fn checkpoint(&self) -> Result<(), Error> {
        Err(Error::UnimplementedError("checkpoint".to_string()))
    }

    pub async fn create(
        &self,
        id: &str,
        bundle: &PathBuf,
        opts: Option<&CreateOpts>,
    ) -> Result<(), Error> {
        Err(Error::UnimplementedError("create".to_string()))
    }

    pub async fn delete(&self, id: &str, opts: Option<&DeleteOpts>) -> Result<(), Error> {
        Err(Error::UnimplementedError("delete".to_string()))
    }

    pub async fn events(&self, id: &str, interval: &Duration) -> Result<(), Error> {
        Err(Error::UnimplementedError("events".to_string()))
    }

    pub async fn exec(&self, id: &str, opts: Option<&ExecOpts>) -> Result<(), Error> {
        Err(Error::UnimplementedError("exec".to_string()))
    }

    pub async fn kill(&self, id: &str, opts: Option<&KillOpts>) -> Result<(), Error> {
        Err(Error::UnimplementedError("kill".to_string()))
    }

    pub async fn list(&self) -> Result<Vec<Container>, Error> {
        Err(Error::UnimplementedError("list".to_string()))
    }

    pub async fn pause(&self) -> Result<Vec<usize>, Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    /// List all the processes inside the container, returning their pids
    pub async fn ps(&self) -> Result<Vec<usize>, Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn restore(&self) -> Result<(), Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn resume(&self) -> Result<(), Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn run(&self) -> Result<Vec<usize>, Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn spec(&self) -> Result<Vec<usize>, Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn start(&self) -> Result<Vec<usize>, Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn state(&self) -> Result<Vec<usize>, Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn stats(&self) -> Result<(), Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }

    pub async fn update(&self) -> Result<Vec<usize>, Error> {
        Err(Error::UnimplementedError("ps".to_string()))
    }
}

fn filter_env(input: &[String], names: &[String]) -> Vec<String> {
    let mut envs: Vec<String> = vec![];
    'loop0: for v in input {
        for k in names {
            if v.starts_with(k.as_str()) {
                continue 'loop0;
            }
        }
        envs.push(v.clone());
    }
    envs
}
