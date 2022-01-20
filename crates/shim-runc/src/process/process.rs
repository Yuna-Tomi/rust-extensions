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

use crate::options::oci::Options;
use crate::process::config::{CreateConfig, ExecConfig};
use crate::utils;
use chrono::Utc;
use containerd_runc_rust as runc;
use containerd_shim_protos as protos;
use protos::shim::task::Status;
use runc::RuncClient;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::Path;
use std::sync::RwLock;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

pub trait InitState {
    fn start(&mut self) -> io::Result<()>;
    fn delete(&self) -> io::Result<()>;
    fn pause(&self) -> io::Result<()>;
    fn resume(&self) -> io::Result<()>;
    fn update(&self, resource_config: Option<&dyn std::any::Any>) -> io::Result<()>;
    // FIXME: suspended for difficulties
    // fn checkpoint(&self) -> io::Result<()>;
    fn exec(&self, config: ExecConfig) -> io::Result<()>; // FIXME: Result<dyn impl Process>
    fn kill(&self, sig: u32, all: bool) -> io::Result<()>;
    fn set_exited(&self, status: isize);
    fn status(&self) -> io::Result<String>;
}

pub trait Process {
    fn id(&self) -> String;
    fn pid(&self) -> isize;
    fn exit_status(&self) -> isize;
    fn exited_at(&self) -> Option<chrono::DateTime<Utc>>;
    // FIXME: suspended for difficulties
    // fn stdin(&self) -> ???;
    fn stdio(&self) -> StdioConfig;
    fn wait(&self);
    // FIXME: suspended for difficulties
    // fn resize(&self) -> io::Result<()>;
    fn start(&mut self) -> io::Result<()>;
    fn delete(&self) -> io::Result<()>;
    fn kill(&self, sig: u32, all: bool) -> io::Result<()>;
    fn set_exited(&self, status: isize);
    fn status(&self) -> io::Result<String>;
}

#[derive(Debug, Clone, Default)]
pub struct StdioConfig {
    pub stdin: String,
    pub stdout: String,
    pub stderr: String,
    terminal: bool,
}

// Might be ugly hack: in Rust, it is not good idea to hold Mutex inside InitProcess because it disables to clone.
/// Init process for a container
#[derive(Debug, Clone, Default)]
pub struct InitProcess {
    pub mu: Arc<Mutex<()>>,

    // represents state transition
    pub state: Status,

    pub work_dir: String,
    pub id: String,
    pub bundle: String,
    // FIXME: suspended for difficulties
    // console: ???,
    // platform: ???,
    pub runtime: Option<RuncClient>,

    /// The pausing state
    pub pausing: bool, // here using primitive bool because InitProcess is designed to allow access only through Arc<Mutex<Self>>.
    pub status: isize,
    pub exited: Option<chrono::DateTime<Utc>>,
    pub pid: isize,
    // FIXME: suspended for difficulties
    // closers: Vec<???>,
    // stdin: ???,
    pub stdio: StdioConfig,

    pub rootfs: String,
    pub io_uid: isize,
    pub io_gid: isize,
    pub no_pivot_root: bool,
    pub no_new_keyring: bool,
    // checkout is not supported now
    // pub criu_work_path: bool,
}

impl InitProcess {
    /// Mutex is required because used to ensure that [`InitProcess::start()`] and [`InitProcess::exit()`] calls return in
    /// the right order when invoked in separate threads.
    /// This is the case within the shim implementation as it makes use of
    /// the reaper interface.
    pub fn new<P, W, R>(
        path: P,
        work_dir: W,
        namespace: String,
        config: CreateConfig,
        opts: Options,
        rootfs: R,
    ) -> Self
    where
        P: AsRef<Path>,
        W: AsRef<Path>,
        R: AsRef<Path>,
    {
        let runtime = utils::new_runc(
            Some(opts.root),
            path,
            namespace,
            opts.binary_name,
            opts.systemd_cgroup,
        )
        .ok();
        let stdio = StdioConfig {
            stdin: config.stdin,
            stdout: config.stdout,
            stderr: config.stderr,
            terminal: config.terminal,
        };

        // NOTE: pid is not set when this struct is created
        Self {
            state: Status::CREATED,
            work_dir: work_dir
                .as_ref()
                .to_string_lossy()
                .parse::<String>()
                .unwrap(),
            id: config.id,
            bundle: config.bundle,
            runtime,
            stdio,
            status: 0,
            rootfs: rootfs.as_ref().to_string_lossy().parse::<String>().unwrap(),
            io_uid: opts.io_uid as isize,
            io_gid: opts.io_gid as isize,
            no_pivot_root: opts.no_pivot_root,
            no_new_keyring: opts.no_new_keyring,
            ..Default::default()
        }
    }

    /// Create the process with the provided config
    pub fn create(&mut self, config: CreateConfig) -> io::Result<()> {
        Err(io::ErrorKind::NotFound)?;
        let pid_file = Path::new(&self.bundle).join("init.pid");
        if config.terminal {
            // FIXME: using console is suspended for difficulties
        } else {
            // FIXME: have to prepare IO
        }

        let opts = runc::options::CreateOpts::new()
            .pid_file(&pid_file)
            .no_pivot(self.no_pivot_root);

        // FIXME: apply appropriate error
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| io::ErrorKind::NotFound)?;
        runtime
            .create(config.id.as_str(), &config.bundle, Some(&opts))
            .map_err(|e| {
                log::error!("{}", e);
                io::ErrorKind::Other
            })?;
        if config.stdin != "" {
            // FIXME: have to open stdin
        }

        // FIXME: appropriate error message
        // read pid from pid file (after container created)
        let mut pid_f = OpenOptions::new().read(true).open(&pid_file)?;
        let mut pid_str = String::new();
        pid_f.read_to_string(&mut pid_str)?;
        self.pid = pid_str.parse::<isize>().unwrap(); // content of init.pid is always a number

        panic!("unimplemented!")
    }
}

impl InitState for InitProcess {
    fn start(&mut self) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        let runtime = self
            .runtime
            .as_ref()
            .ok_or_else(|| io::ErrorKind::NotFound)?;
        runtime.start(&self.id).map_err(|e| {
            log::error!("{}", e);
            io::ErrorKind::Other
        })?;
        self.state = Status::RUNNING;
        Ok(())
    }

    fn delete(&self) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        panic!("unimplemented!")
    }

    fn pause(&self) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn resume(&self) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn update(&self, resource_config: Option<&dyn std::any::Any>) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn exec(&self, config: ExecConfig) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn kill(&self, sig: u32, all: bool) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        panic!("unimplemented!")
    }

    fn set_exited(&self, status: isize) {
        let _m = self.mu.lock().unwrap();
        panic!("unimplemented!")
    }

    fn status(&self) -> io::Result<String> {
        panic!("unimplemented!")
    }
}

impl Process for InitProcess {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn pid(&self) -> isize {
        self.pid
    }

    fn exit_status(&self) -> isize {
        let _m = self.mu.lock();
        self.status
    }

    fn exited_at(&self) -> Option<chrono::DateTime<Utc>> {
        let _m = self.mu.lock();
        self.exited
    }

    fn stdio(&self) -> StdioConfig {
        self.stdio.clone()
    }

    fn status(&self) -> io::Result<String> {
        let _m = self.mu.lock();
        if self.pausing {
            Ok("pausing".to_string())
        } else {
            InitState::status(self)
        }
    }

    // FIXME
    fn wait(&self) {
        panic!("unimplemented!")
    }

    fn start(&mut self) -> io::Result<()> {
        InitState::start(self)
    }

    fn delete(&self) -> io::Result<()> {
        InitState::delete(self)
    }

    fn kill(&self, sig: u32, all: bool) -> io::Result<()> {
        InitState::kill(self, sig, all)
    }

    fn set_exited(&self, status: isize) {
        InitState::set_exited(self, status)
    }
}
