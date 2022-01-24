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

// NOTE: Go references
// https://github.com/containerd/containerd/blob/main/pkg/process/init.go
// https://github.com/containerd/containerd/blob/main/pkg/process/init_state.go

use crate::options::oci::Options;
use crate::utils;
use super::traits::{
    ContainerProcess, InitState, Process, 
};
use super::config::{
    CreateConfig,
    ExecConfig, StdioConfig,
};
use super::state::{
    ProcessState,
};
use chrono::{DateTime, Utc};
use containerd_runc_rust as runc;
use containerd_shim_protos as protos;
use nix::{
    sys::wait::{self, WaitPidFlag, WaitStatus},
    unistd::Pid,
};

use futures::{
    channel::oneshot::{self, Receiver},
    executor,
};
use runc::options::{DeleteOpts, KillOpts};
use runc::RuncClient;
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::Path;
use std::sync::RwLock;
use std::sync::{atomic::AtomicBool, Arc, Mutex};
use std::thread;

use crate::dbg::*;


// Might be ugly hack: in Rust, it is not good idea to hold Mutex inside InitProcess because it disables to clone.
/// Init process for a container
#[derive(Debug)]
pub struct InitProcess {
    pub mu: Arc<Mutex<()>>,

    // represents state transition
    pub state: ProcessState,

    wait_block: Option<Receiver<()>>,

    pub work_dir: String,
    pub id: String,
    pub bundle: String,
    // FIXME: suspended for difficulties
    // console: ???,
    // platform: ???,
    runtime: RuncClient,

    /// The pausing state
    pausing: bool, // here using primitive bool because InitProcess is designed to allow access only through Arc<Mutex<Self>>.
    status: isize,
    exited: Option<chrono::DateTime<Utc>>,
    pid: isize,
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
    ) -> io::Result<Self>
    where
        P: AsRef<Path>,
        W: AsRef<Path>,
        R: AsRef<Path>,
    {
        let runtime = utils::new_runc(
            opts.root,
            path,
            namespace,
            opts.binary_name,
            opts.systemd_cgroup,
        )
        .map_err(|_| io::Error::from(io::ErrorKind::NotFound))?;
        let stdio = StdioConfig {
            stdin: config.stdin,
            stdout: config.stdout,
            stderr: config.stderr,
            terminal: config.terminal,
        };

        Ok(Self {
            mu: Arc::default(),
            state: ProcessState::Unknown,
            wait_block: None,
            work_dir: work_dir
                .as_ref()
                .to_string_lossy()
                .parse::<String>()
                .unwrap(),
            id: config.id,
            bundle: config.bundle,
            runtime,
            stdio,
            pausing: false,
            status: 0,
            pid: 0, // NOTE: pid is not set when this struct is created
            exited: None,
            rootfs: rootfs.as_ref().to_string_lossy().parse::<String>().unwrap(),
            io_uid: opts.io_uid as isize,
            io_gid: opts.io_gid as isize,
            no_pivot_root: opts.no_pivot_root,
            no_new_keyring: opts.no_new_keyring,
        })
    }

    /// Create the process with the provided config
    pub fn create(&mut self, config: CreateConfig) -> io::Result<()> {
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
        debug_log!("call RuncClient::create:");
        debug_log!("    id={}, bundle={}", config.id, config.bundle);
        debug_log!("    opts={:?}", opts);
        let (tx, rx) = oneshot::channel::<()>();
        self.wait_block = Some(rx);
        let res = self
            .runtime
            .create(config.id.as_str(), &config.bundle, Some(&opts))
            .map_err(|e| {
                log::error!("{}", e);
                io::ErrorKind::Other
            })?;
        debug_log!("RuncClient::create succeeded");
        tx.send(()).unwrap(); // notify successfully created.

        if config.stdin != "" {
            // FIXME: have to open stdin
        }

        let mut pid_f = OpenOptions::new().read(true).open(&pid_file)?;
        let mut pid_str = String::new();
        pid_f.read_to_string(&mut pid_str)?;
        self.pid = pid_str.parse::<isize>().unwrap(); // content of init.pid is always a number
        self.state = ProcessState::Created;

        Ok(())
    }

    pub fn start(&mut self) -> io::Result<()> { InitState::start(self) }
    pub fn delete(&mut self) -> io::Result<()> { InitState::delete(self) }
    pub fn state(&mut self) -> io::Result<ProcessState> { InitState::state(self) }
    pub fn pause(&mut self) -> io::Result<()> { InitState::pause(self) }
    pub fn resume(&mut self) -> io::Result<()> { InitState::resume(self) }
    pub fn exec(&mut self, config: ExecConfig) -> io::Result<()> { InitState::exec(self, config) }
    pub fn kill(&mut self, sig: u32, all: bool) -> io::Result<()> { InitState::kill(self, sig, all) }
    pub fn set_exited(&mut self, status: isize) { InitState::set_exited(self, status) }
    pub fn update(&mut self, resource_config: Option<&dyn std::any::Any>) -> io::Result<()> {InitState::update(self, resource_config)}
    pub fn pid(&self) -> isize { Process::pid(self) }
    pub fn exit_status(&self) -> isize { Process::exit_status(self) }
    pub fn exited_at(&self) -> Option<chrono::DateTime<Utc>> { Process::exited_at(self) }
    pub fn stdio(&self) -> StdioConfig { Process::stdio(self) }
    pub fn wait(&mut self) -> io::Result<()> { Process::wait(self) }
}

impl ContainerProcess for InitProcess {}

impl InitState for InitProcess {
    fn start(&mut self) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        let (tx, rx) = oneshot::channel::<()>();
        // wait for wait() on creation process
        // while let Some(_) = self.wait_block {} // this produce deadlock because of Mutex of containers at Service
        // self.wait_block = Some(rx);
        debug_log!("RuncClient::create succeeded");
        // tx.send(()).unwrap(); // notify successfully started.
        
        let res = self.runtime.start(&self.id).map_err(|e| {
            log::error!("{}", e);
            io::ErrorKind::Other
        })?;
        debug_log!("started container: res={:?}", res);
        self.state = ProcessState::Running;
        Ok(())
    }

    fn delete(&mut self) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        self.runtime.delete(&self.id, None).map_err(|e| {
            log::error!("{}", e);
            io::ErrorKind::Other
        })?;
        self.state = ProcessState::Deleted;
        Ok(())
    }

    fn pause(&mut self) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn resume(&mut self) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn update(&mut self, resource_config: Option<&dyn std::any::Any>) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn exec(&self, config: ExecConfig) -> io::Result<()> {
        panic!("unimplemented!")
    }

    fn kill(&mut self, sig: u32, all: bool) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        let opts = KillOpts::new().all(all);
        self.runtime.kill(&self.id, sig, Some(&opts)).map_err(|e| {
            log::error!("{}", e);
            io::ErrorKind::Other
        })?;
        Ok(())
    }

    fn set_exited(&mut self, status: isize) {
        let _m = self.mu.lock().unwrap();
        let time = Utc::now();
        self.state = ProcessState::Stopped;
        self.exited = Some(time);
        self.status = status;
    }

    fn state(&self) -> io::Result<ProcessState> {
        let _m = self.mu.lock().unwrap();
        match self.state {
            ProcessState::Unknown => Err(io::Error::from(io::ErrorKind::NotFound)),
            _ => Ok(self.state),
        }
    }
}

/// Some of these implementation internally calls [`InitState`].
/// Note that in such case InitState will take Mutex and [`InitProcess`] should not take, avoiding dead lock.
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

    fn state(&self) -> io::Result<ProcessState> {
        InitState::state(self)
    }

    fn wait(&mut self) -> io::Result<()> {
        // FIXME: Might be ugly hack
        debug_log!("InitProcess::wait pid={}", self.pid);
        let rx = self
            .wait_block
            .take()
            .ok_or_else(|| io::ErrorKind::NotFound)?;
        let x = executor::block_on(async {
            // FIXME: need appropriate error handling
            rx.await.map_err(|e| io::ErrorKind::Other)
        })?;
        Ok(())
    }

    // fn wait(&mut self) -> io::Result<()> {
    //     // FIXME: Might be ugly hack
    //     debug_log!("InitProcess::wait pid={}", self.pid);
    //     loop {
    //         match wait::waitpid(Pid::from_raw(self.pid as i32), None) {
    //             Ok(WaitStatus::Exited(_, status)) => {
    //                 InitState::set_exited(self, status as isize);
    //                 return Ok(());
    //             }
    //             Err(e) => return Err(io::Error::from_raw_os_error(e as i32)),
    //             _ => {}
    //         }
    //     }
    // }

    fn start(&mut self) -> io::Result<()> {
        InitState::start(self)
    }

    fn delete(&mut self) -> io::Result<()> {
        InitState::delete(self)
    }

    fn kill(&mut self, sig: u32, all: bool) -> io::Result<()> {
        InitState::kill(self, sig, all)
    }

    fn set_exited(&mut self, status: isize) {
        InitState::set_exited(self, status)
    }
}
