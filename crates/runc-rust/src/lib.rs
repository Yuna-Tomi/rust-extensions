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

//! A crate for consuming the runc binary in your Rust applications, similar to [go-runc](https://github.com/containerd/go-runc) for Go.

use crate::container::Container;
use crate::error::Error;
use crate::events::{Event, Stats};
use crate::options::*;
use crate::specs::{LinuxResources, Process};
use crate::utils::{
    DEBUG, DEFAULT_COMMAND, JSON, LOG, LOG_FORMAT, ROOT, ROOTLESS, SYSTEMD_CGROUP, TEXT,
};
use log::warn;
use std::fmt::{self, Display};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::process::Command;
use tokio::time;

pub mod console;
pub mod container;
pub mod error;
pub mod events;
pub mod monitor;
pub mod options;
pub mod specs;
mod stream;
mod utils;

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

impl Display for LogFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogFormat::Json => write!(f, "{}", JSON),
            LogFormat::Text => write!(f, "{}", TEXT),
        }
    }
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
    // FIXME: implementation of pdeath_signal is suspended due to difficulties, maybe it's favorable to use signal-hook crate.
    // pdeath_signal: XXX,
    /// Using systemd cgroup.
    systemd_cgroup: bool,
    /// Setting process group ID(gpid).
    set_pgid: bool,
    // FIXME: implementation of extra_args is suspended due to difficulties.
    // criu: String,
    /// Setting of whether using rootless mode or not. If [`None`], "auto" settings is used. Note that "auto" is different from explicit "true" or "false".
    rootless: Option<bool>,
    // FIXME: implementation of extra_args is suspended due to difficulties.
    // extra_args: Vec<String>,
    /// Timeout settings for runc command. Default is 5 seconds.
    timeout: Option<Duration>,
}

impl RuncConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn command(mut self, command: impl AsRef<Path>) -> Self {
        self.command = Some(command.as_ref().to_path_buf());
        self
    }

    pub fn root(mut self, root: impl AsRef<Path>) -> Self {
        self.root = Some(root.as_ref().to_path_buf());
        self
    }

    pub fn debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }

    pub fn log(mut self, log: impl AsRef<Path>) -> Self {
        self.log = Some(log.as_ref().to_path_buf());
        self
    }

    pub fn log_format(mut self, log_format: LogFormat) -> Self {
        self.log_format = Some(log_format);
        self
    }

    pub fn log_format_json(mut self) -> Self {
        self.log_format = Some(LogFormat::Json);
        self
    }

    pub fn log_format_text(mut self) -> Self {
        self.log_format = Some(LogFormat::Text);
        self
    }

    pub fn systemd_cgroup(mut self, systemd_cgroup: bool) -> Self {
        self.systemd_cgroup = systemd_cgroup;
        self
    }

    // FIXME: criu is not supported now
    // pub fn criu(mut self, criu: bool) -> Self {
    //     self.criu = criu;
    // }

    pub fn rootless(mut self, rootless: bool) -> Self {
        self.rootless = Some(rootless);
        self
    }

    pub fn set_pgid(mut self, set_pgid: bool) -> Self {
        self.set_pgid = set_pgid;
        self
    }

    pub fn rootless_auto(mut self) -> Self {
        let _ = self.rootless.take();
        self
    }

    pub fn timeout(mut self, millis: u64) -> Self {
        self.timeout = Some(Duration::from_millis(millis));
        self
    }

    pub fn build(self) -> Result<Runc, Error> {
        let command = utils::binary_path(self.command.unwrap_or(PathBuf::from(DEFAULT_COMMAND)))
            .ok_or(Error::NotFoundError)?;
        Ok(Runc {
            command,
            root: self.root,
            debug: self.debug,
            log: self.log,
            log_format: self.log_format.unwrap_or(LogFormat::Text),
            // self.pdeath_signal: self.pdeath_signal,
            systemd_cgroup: self.systemd_cgroup,
            set_pgid: self.set_pgid,
            // criu: self.criu,
            rootless: self.rootless,
            // extra_args: self.extra_args,
            timeout: self.timeout.unwrap_or(Duration::from_millis(5000)),
        })
    }
}

/// Runc client to the runc cli
pub struct Runc {
    command: PathBuf,
    root: Option<PathBuf>,
    debug: bool,
    log: Option<PathBuf>,
    log_format: LogFormat,
    // pdeath_signal: XXX,
    set_pgid: bool,
    // criu: bool,
    systemd_cgroup: bool,
    rootless: Option<bool>,
    // extra_args: Vec<String>,
    timeout: Duration,
}

impl Args for Runc {
    type Output = Result<Vec<String>, Error>;
    fn args(&self) -> Self::Output {
        let mut args: Vec<String> = vec![];
        if let Some(root) = &self.root {
            args.push(ROOT.to_string());
            args.push(utils::abs_string(root)?);
        }
        if self.debug {
            args.push(DEBUG.to_string());
        }
        if let Some(log_path) = &self.log {
            args.push(LOG.to_string());
            args.push(utils::abs_string(log_path)?);
        }
        args.push(LOG_FORMAT.to_string());
        args.push(self.log_format.to_string());
        // if self.criu {
        //     args.push(CRIU.to_string());
        // }
        if self.systemd_cgroup {
            args.push(SYSTEMD_CGROUP.to_string());
        }
        if let Some(rootless) = self.rootless {
            let arg = format!("{}={}", ROOTLESS, rootless);
            args.push(arg);
        }
        // if self.extra_args.len() > 0 {
        //     args.append(mut self.extra_args.clone())
        // }
        Ok(args)
    }
}

impl Runc {
    /// Create a new runc client from the supplied configuration
    pub fn from_config(config: RuncConfig) -> Result<Self, Error> {
        config.build()
    }

    #[cfg(target_os = "linux")]
    pub async fn command(&self, args: &[String], combined_output: bool) -> Result<String, Error> {
        let args = [&self.args()?, args].concat();
        let proc = Command::new(&self.command)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(Error::ProcessSpawnError)?;

        let result = time::timeout(self.timeout, proc.wait_with_output())
            .await
            .map_err(Error::CommandTimeoutError)?
            .map_err(Error::CommandError)?;

        let status = result.status;
        let stdout = String::from_utf8(result.stdout).unwrap();
        let stderr = String::from_utf8(result.stderr).unwrap();

        if status.success() {
            Ok(if combined_output {
                stdout + stderr.as_str()
            } else {
                stdout
            })
        } else {
            Err(Error::CommandFaliedError {
                status,
                stdout,
                stderr,
            })
        }
    }

    #[cfg(not(target_os = "linux"))]
    pub async fn command(&self, args: &[String]) -> Result<(), Error> {
        Err(Error::UnimplementedError("command".to_string()))
    }

    pub async fn checkpoint(&self) -> Result<(), Error> {
        Err(Error::UnimplementedError("checkpoint".to_string()))
    }

    /// Create a new container
    pub async fn create(
        &self,
        id: &str,
        bundle: impl AsRef<Path>,
        opts: Option<&CreateOpts>,
    ) -> Result<(), Error> {
        let mut args = vec![
            "create".to_string(),
            "--bundle".to_string(),
            utils::abs_string(bundle)?,
        ];
        if let Some(opts) = opts {
            args.append(&mut opts.args()?);
        }
        args.push(id.to_string());
        self.command(&args, true).await?;
        Ok(())
    }

    /// Delete a container
    pub async fn delete(&self, id: &str, opts: Option<&DeleteOpts>) -> Result<(), Error> {
        let mut args = vec!["delete".to_string()];
        if let Some(opts) = opts {
            args.append(&mut opts.args());
        }
        args.push(id.to_string());
        self.command(&args, true).await?;
        Ok(())
    }

    /// Return an event stream of container notifications
    pub async fn events(&self, id: &str, interval: &Duration) -> Result<(), Error> {
        Err(Error::UnimplementedError("events".to_string()))
    }

    /// Execute an additional process inside the container
    pub async fn exec(
        &self,
        id: &str,
        spec: &Process,
        opts: Option<&ExecOpts>,
    ) -> Result<(), Error> {
        let (mut temp_file, file_name): (NamedTempFile, String) =
            utils::make_temp_file_in_runtime_dir()?;
        {
            let f = temp_file.as_file_mut();
            let spec_json = serde_json::to_string(spec).map_err(Error::JsonDeserializationError)?;
            f.write(spec_json.as_bytes())
                .map_err(Error::SpecFileCreationError)?;
            f.flush().map_err(Error::SpecFileCreationError)?;
        }
        let mut args = vec!["exec".to_string(), "process".to_string(), file_name];
        if let Some(opts) = opts {
            args.append(&mut opts.args()?);
        }
        args.push(id.to_string());
        self.command(&args, true).await?;
        Ok(())
    }

    /// Send the specified signal to processes inside the container
    pub async fn kill(&self, id: &str, sig: i32, opts: Option<&DeleteOpts>) -> Result<(), Error> {
        let mut args = vec!["kill".to_string()];
        if let Some(opts) = opts {
            args.append(&mut opts.args());
        }
        args.push(id.to_string());
        args.push(sig.to_string());
        self.command(&args, true).await?;
        Ok(())
    }

    /// List all containers associated with this runc instance
    pub async fn list(&self) -> Result<Vec<Container>, Error> {
        let args = ["list".to_string(), "--format-json".to_string()];
        let output = self.command(&args, false).await?;
        let output = output.trim();
        // Ugly hack to work around golang
        Ok(if output == "null" {
            Vec::new()
        } else {
            serde_json::from_str(output).map_err(Error::JsonDeserializationError)?
        })
    }

    /// Pause a container
    pub async fn pause(&self, id: &str) -> Result<(), Error> {
        let args = ["pause".to_string(), id.to_string()];
        self.command(&args, true).await?;
        Ok(())
    }

    /// List all the processes inside the container, returning their pids
    pub async fn ps(&self, id: &str) -> Result<Vec<usize>, Error> {
        let args = [
            "ps".to_string(),
            "--format-json".to_string(),
            id.to_string(),
        ];
        let output = self.command(&args, false).await?;
        let output = output.trim();
        // Ugly hack to work around golang
        Ok(if output == "null" {
            Vec::new()
        } else {
            serde_json::from_str(output).map_err(Error::JsonDeserializationError)?
        })
    }

    pub async fn restore(&self) -> Result<(), Error> {
        Err(Error::UnimplementedError("restore".to_string()))
    }

    /// Resume a container
    pub async fn resume(&self, id: &str) -> Result<(), Error> {
        let args = ["pause".to_string(), id.to_string()];
        self.command(&args, true).await?;
        Ok(())
    }

    /// Run the create, start, delete lifecycle of the container and return its exit status
    pub async fn run(
        &self,
        id: &str,
        bundle: impl AsRef<Path>,
        opts: Option<&CreateOpts>,
    ) -> Result<(), Error> {
        let mut args = vec!["run".to_string(), "--bundle".to_string()];
        if let Some(opts) = opts {
            args.append(&mut opts.args()?);
        }
        args.push(utils::abs_string(bundle)?);
        args.push(id.to_string());
        self.command(&args, true).await?;
        Ok(())
    }

    /// Start an already created container
    pub async fn start(&self, id: &str) -> Result<(), Error> {
        let args = ["start".to_string(), id.to_string()];
        self.command(&args, true).await?;
        Ok(())
    }

    /// Return the state of a container
    pub async fn state(&self, id: &str) -> Result<Vec<usize>, Error> {
        let args = ["state".to_string(), id.to_string()];
        let output = self.command(&args, true).await?;
        Ok(serde_json::from_str(&output).map_err(Error::JsonDeserializationError)?)
    }

    /// Return the latest statistics for a container
    pub async fn stats(&self, id: &str) -> Result<Stats, Error> {
        let args = ["events".to_string(), "--stats".to_string(), id.to_string()];
        let output = self.command(&args, true).await?;
        let event: Event =
            serde_json::from_str(&output).map_err(Error::JsonDeserializationError)?;
        if let Some(stats) = event.stats {
            Ok(stats)
        } else {
            Err(Error::MissingContainerStatsError)
        }
    }

    /// Update a container with the provided resource spec
    pub async fn update(&self, id: &str, resources: &LinuxResources) -> Result<(), Error> {
        let (mut temp_file, file_name): (NamedTempFile, String) =
            utils::make_temp_file_in_runtime_dir()?;
        {
            let f = temp_file.as_file_mut();
            let spec_json =
                serde_json::to_string(resources).map_err(Error::JsonDeserializationError)?;
            f.write(spec_json.as_bytes())
                .map_err(Error::SpecFileCreationError)?;
            f.flush().map_err(Error::SpecFileCreationError)?;
        }
        let args = [
            "update".to_string(),
            "--resources".to_string(),
            file_name,
            id.to_string(),
        ];
        self.command(&args, true).await?;
        Ok(())
    }
}
// Clean up: this Drop tries to remove runc binary and associated directory, then only for tests.
    impl Drop for Runc {
        fn drop(&mut self) {
            if let Some(root) = self.root.clone() {
                if let Err(e) = fs::remove_dir_all(&root) {
                    warn!("failed to cleanup root directory: {}", e);
                }
            }
            if let Some(system_runc) = utils::binary_path(&self.command) {
                if system_runc != self.command {
                    if let Err(e) = fs::remove_file(&self.command) {
                        warn!("failed to remove runc binary: {}", e);
                    }
                }
            } else if let Err(e) = fs::remove_file(&self.command) {
                warn!("failed to remove runc binary: {}", e);
            }
        }
    }

#[cfg(test)]
mod tests {
    use super::*;
    use log::warn;
    use std::fs;
    use std::fs::File;
    use std::{env, io};
    
    // following style of go-runc, use only true/false to test
    const CMD_TRUE: &str = "/bin/true";
    const CMD_FALSE: &str = "/bin/false";

    #[tokio::test]
    async fn test_command() {
        let ok_runc = RuncConfig::new().command(CMD_TRUE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            ok_runc.command(&[], true).await.expect("true failed.");
            eprintln!("ok_runc succeeded.");
        });

        let fail_runc = RuncConfig::new().command(CMD_FALSE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            match fail_runc.command(&[], true).await {
                Ok(_) => panic!("fail_runc returned exit status 0."),
                Err(Error::CommandFaliedError{
                    status, stdout, stderr,
                }) => {
                    if status.code().unwrap() == 1 && stdout.is_empty() && stderr.is_empty() {
                        eprintln!("fail_runc succeeded.");
                    } else {
                        panic!("unexpected outputs from fail_runc.")
                    }
                }
                _ => panic!("unexpected error from fail_runc.")
            }
        }).await.expect("tokio spawn falied.");
    }

    #[tokio::test]
    async fn test_create() {
        let opts = CreateOpts::new();
        let ok_runc = RuncConfig::new().command(CMD_TRUE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            ok_runc.create("fake-id", "fake-bundle", Some(&opts)).await.expect("true failed.");
            eprintln!("ok_runc succeeded.");
        });

        let opts = CreateOpts::new();
        let fail_runc = RuncConfig::new().command(CMD_FALSE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            match fail_runc.create("fake-id", "fake-bundle", Some(&opts)).await {
                Ok(_) => panic!("fail_runc returned exit status 0."),
                Err(Error::CommandFaliedError{
                    status, stdout, stderr,
                }) => {
                    if status.code().unwrap() == 1 && stdout.is_empty() && stderr.is_empty() {
                        eprintln!("fail_runc succeeded.");
                    } else {
                        panic!("unexpected outputs from fail_runc.")
                    }
                }
                _ => panic!("unexpected error from fail_runc.")
            }
        }).await.expect("tokio spawn falied.");
    }

    #[tokio::test]
    async fn test_run() {
        let opts = CreateOpts::new();
        let ok_runc = RuncConfig::new().command(CMD_TRUE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            ok_runc.create("fake-id", "fake-bundle", Some(&opts)).await.expect("true failed.");
            eprintln!("ok_runc succeeded.");
        });

        let opts = CreateOpts::new();
        let fail_runc = RuncConfig::new().command(CMD_FALSE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            match fail_runc.create("fake-id", "fake-bundle", Some(&opts)).await {
                Ok(_) => panic!("fail_runc returned exit status 0."),
                Err(Error::CommandFaliedError{
                    status, stdout, stderr,
                }) => {
                    if status.code().unwrap() == 1 && stdout.is_empty() && stderr.is_empty() {
                        eprintln!("fail_runc succeeded.");
                    } else {
                        panic!("unexpected outputs from fail_runc.")
                    }
                }
                _ => panic!("unexpected error from fail_runc.")
            }
        }).await.expect("tokio spawn falied.");
    }

    #[tokio::test]
    async fn test_exec() {
        let opts = ExecOpts::new();
        let proc = dummy_process();
        let ok_runc = RuncConfig::new().command(CMD_TRUE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            ok_runc.exec("fake-id", &proc, Some(&opts)).await.expect("true failed.");
            eprintln!("ok_runc succeeded.");
        });

        let opts = ExecOpts::new();
        let proc = dummy_process();
        let fail_runc = RuncConfig::new().command(CMD_FALSE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            match fail_runc.exec("fake-id", &proc, Some(&opts)).await {
                Ok(_) => panic!("fail_runc returned exit status 0."),
                Err(Error::CommandFaliedError{
                    status, stdout, stderr,
                }) => {
                    if status.code().unwrap() == 1 && stdout.is_empty() && stderr.is_empty() {
                        eprintln!("fail_runc succeeded.");
                    } else {
                        panic!("unexpected outputs from fail_runc.")
                    }
                }
                _ => panic!("unexpected error from fail_runc.")
            }
        }).await.expect("tokio spawn falied.");
    }

    #[tokio::test]
    async fn test_delete() {
        let opts = DeleteOpts::new();
        let ok_runc = RuncConfig::new().command(CMD_TRUE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            ok_runc.delete("fake-id",Some(&opts)).await.expect("true failed.");
            eprintln!("ok_runc succeeded.");
        });

        let opts = DeleteOpts::new();
        let fail_runc = RuncConfig::new().command(CMD_FALSE).build().expect("unable to create runc instance");
        tokio::spawn(async move {
            match fail_runc.delete("fake-id", Some(&opts)).await {
                Ok(_) => panic!("fail_runc returned exit status 0."),
                Err(Error::CommandFaliedError{
                    status, stdout, stderr,
                }) => {
                    if status.code().unwrap() == 1 && stdout.is_empty() && stderr.is_empty() {
                        eprintln!("fail_runc succeeded.");
                    } else {
                        panic!("unexpected outputs from fail_runc.")
                    }
                }
                _ => panic!("unexpected error from fail_runc.")
            }
        }).await.expect("tokio spawn falied.");
    }

    fn dummy_process() -> Process {
        serde_json::from_str("{}").unwrap()
    }
}

