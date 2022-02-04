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

// NOTE: Go references
// https://github.com/containerd/containerd/blob/main/pkg/process/init.go
// https://github.com/containerd/containerd/blob/main/pkg/process/init_state.go

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::Path;
use std::sync::{Arc, Mutex};

use futures::executor;
use log::error;
use nix::fcntl::OFlag;
use runc::io::Io;
use runc::options::KillOpts;
use runc::AsyncClient;
use time::OffsetDateTime;

use super::config::{CreateConfig, ExecConfig, StdioConfig};
use super::fifo::Fifo;
use super::io::ProcessIO;
use super::state::ProcessState;
use super::traits::{ContainerProcess, InitState, Process};
use crate::options::oci::Options;
use crate::utils;

use crate::dbg::*;

/// Init process for a container
#[derive(Debug)]
pub struct InitProcess {
    pub mu: Arc<Mutex<()>>,

    // represents state transition
    pub state: ProcessState,

    wait_block: Option<tokio::sync::oneshot::Receiver<()>>,

    // This struct must contain tokio runtime to enable
    tokio_runtime: tokio::runtime::Runtime,
    pub work_dir: String,
    pub id: String,
    pub bundle: String,
    // FIXME: suspended for difficulties
    // console: ???,
    // platform: ???,
    io: Option<Arc<ProcessIO>>,
    runtime: Arc<AsyncClient>,

    /// The pausing state
    pausing: bool,
    status: isize,
    exited: Option<OffsetDateTime>,
    pid: isize,
    // FIXME: suspended for difficulties
    // closers: Vec<???>,
    // might be ugly hack
    stdin: Option<Fifo>,
    stdio: StdioConfig,

    rootfs: String,
    io_uid: isize,
    io_gid: isize,
    no_pivot_root: bool,
    no_new_keyring: bool,
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
        let runtime = utils::new_async_runc(
            opts.root,
            path,
            namespace,
            &opts.binary_name,
            opts.systemd_cgroup,
        )
        .map_err(|_| io::Error::from(io::ErrorKind::NotFound))?;
        let stdio = StdioConfig {
            stdin: config.stdin,
            stdout: config.stdout,
            stderr: config.stderr,
            terminal: config.terminal,
        };

        let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

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
            io: None,
            tokio_runtime,
            runtime: Arc::new(runtime),
            stdin: None,
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
        let mut opts = runc::options::CreateOpts {
            pid_file: Some(pid_file.clone()),
            no_pivot: self.no_pivot_root,
            ..Default::default()
        };

        debug_log!("options: {:?}", opts);
        if config.terminal {
            unimplemented!()
            // FIXME: using console is suspended for difficulties
        } else {
            // note that io contains nothing until this time, then we can insert new ProcessIO certainly.
            // FIXME: process io settings is suspended for difficulties
            let proc_io = ProcessIO::new(&self.id, self.io_uid, self.io_gid, self.stdio.clone())?;
            opts = opts.io(proc_io.io().unwrap());
            let _ = self.io.get_or_insert(Arc::new(proc_io));
        }

        // FIXME: apply appropriate error
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        self.wait_block = Some(rx);
        debug_log!("call create_and_io_preparation...");
        self.create_and_io_preparation(config, opts)?;
        tx.send(()).unwrap(); // notify successfully created.

        let mut pid_f = OpenOptions::new().read(true).open(&pid_file)?;
        let mut pid_str = String::new();
        pid_f.read_to_string(&mut pid_str)?;
        self.pid = pid_str.parse::<isize>().unwrap(); // content of init.pid is always a number
        self.state = ProcessState::Created;
        Ok(())
    }

    // Block on preparation of io for communication between shim and runc.
    // We call open on fifo in open_stdin() (write end), and then
    // open another end in copy_pipes() or copy_console()
    // Note that we have WaitGroup in some crate like crossbeam,
    // but this style may be more comprehensive.
    fn create_and_io_preparation(
        &mut self,
        config: CreateConfig,
        mut opts: runc::options::CreateOpts,
    ) -> std::io::Result<()> {
        let CreateConfig {
            id,
            bundle,
            terminal,
            stdin,
            ..
        } = config;
        debug_log!(
            "config: id={}, bundle={}, term={}, stdin={}",
            id,
            bundle,
            terminal,
            stdin
        );
        if terminal {
            unimplemented!()
            // opts.console_socket = socket;
        } else {
            // if not using terminal, self.io is always Some
            debug_log!("now: {:?}", self);
            opts.io = self.io.as_ref().unwrap().io();
        }

        debug_log!("start tokio runtime from create_and_io_preparation...");
        let ret = self.tokio_runtime.block_on(async {
            let mut tasks = vec![];
            let runtime = Arc::clone(&self.runtime);
            let create = tokio::spawn(async move {
                debug_log!("runc create...");
                runtime.create(&id, bundle, Some(&opts)).await.map_err(|e| {
                    error!("runc create failed: {}", e);
                    std::io::ErrorKind::Other.into()
                })
            });
            tasks.push(create);

            let open_stdin = if stdin != "" {
                let open_stdin = tokio::spawn(async move {
                    Fifo::open(&stdin, OFlag::O_WRONLY | OFlag::O_NONBLOCK, 0).await
                });
                Some(open_stdin)
            } else {
                None
            };

            let copy_console = if terminal {
                let copy_console = tokio::spawn(async move {
                    // unimplemented!();
                    Ok::<(), std::io::Error>(() /* should retuen console handler */)
                });
                Some(copy_console)
            } else {
                let proc_io = self.io.clone().unwrap();
                let copy_io = tokio::spawn(async move {
                    debug_log!("copy pipes...");
                    proc_io.copy_pipes().await
                });
                tasks.push(copy_io);
                None
            };

            for t in tasks {
                t.await?.map_err(|_| std::io::ErrorKind::Other)?;
            }

            let stdin = if let Some(t) = open_stdin {
                Some(t.await??)
            } else {
                None
            };

            let console = if let Some(t) = copy_console {
                Some(t.await??)
            } else {
                None
            };
            Ok::<(Option<Fifo>, Option<()>), std::io::Error>((stdin, console))
        })?;
        let (stdin, console) = ret;
        self.stdin = stdin;
        // self.console = console
        Ok(())
    }

    pub fn start(&mut self) -> io::Result<()> {
        InitState::start(self)
    }
    pub fn delete(&mut self) -> io::Result<()> {
        InitState::delete(self)
    }
    pub fn state(&mut self) -> io::Result<ProcessState> {
        InitState::state(self)
    }
    pub fn pause(&mut self) -> io::Result<()> {
        InitState::pause(self)
    }
    pub fn resume(&mut self) -> io::Result<()> {
        InitState::resume(self)
    }
    pub fn exec(&mut self, config: ExecConfig) -> io::Result<()> {
        InitState::exec(self, config)
    }
    pub fn kill(&mut self, sig: u32, all: bool) -> io::Result<()> {
        InitState::kill(self, sig, all)
    }
    pub fn set_exited(&mut self, status: isize) {
        InitState::set_exited(self, status)
    }
    pub fn update(&mut self, resource_config: Option<&dyn std::any::Any>) -> io::Result<()> {
        InitState::update(self, resource_config)
    }
    pub fn pid(&self) -> isize {
        Process::pid(self)
    }
    pub fn exit_status(&self) -> isize {
        Process::exit_status(self)
    }
    pub fn exited_at(&self) -> Option<OffsetDateTime> {
        Process::exited_at(self)
    }
    pub fn stdio(&self) -> StdioConfig {
        Process::stdio(self)
    }
    pub fn wait(&mut self) -> io::Result<()> {
        Process::wait(self)
    }
}

impl ContainerProcess for InitProcess {}

impl InitState for InitProcess {
    fn start(&mut self) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        // wait for wait() on creation process
        // while let Some(_) = self.wait_block {} // this produce deadlock because of Mutex of containers at Service
        // self.wait_block = Some(rx);
        // tx.send(()).unwrap(); // notify successfully started.
        self.tokio_runtime.block_on(async {
            self.runtime.start(&self.id).await.map_err(|e| {
                error!("runc start failed: {}", e);
                io::ErrorKind::Other
            })
        })?;
        self.state = ProcessState::Running;
        Ok(())
    }

    fn delete(&mut self) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        self.tokio_runtime.block_on(async {
            self.runtime.delete(&self.id, None).await.map_err(|e| {
                error!("runc delete failed: {}", e);
                io::ErrorKind::Other
            })
        })?;
        self.state = ProcessState::Deleted;
        Ok(())
    }

    fn pause(&mut self) -> io::Result<()> {
        unimplemented!()
    }

    fn resume(&mut self) -> io::Result<()> {
        unimplemented!()
    }

    fn update(&mut self, _resource_config: Option<&dyn std::any::Any>) -> io::Result<()> {
        unimplemented!()
    }

    fn exec(&self, _config: ExecConfig) -> io::Result<()> {
        unimplemented!()
    }

    fn kill(&mut self, sig: u32, all: bool) -> io::Result<()> {
        let _m = self.mu.lock().unwrap();
        let opts = KillOpts { all };
        self.tokio_runtime.block_on(async {
            self.runtime
                .kill(&self.id, sig, Some(&opts))
                .await
                .map_err(|e| {
                    error!("runc kill failed: {}", e);
                    io::ErrorKind::Other
                })
        })?;
        self.state = ProcessState::Stopped;
        Ok(())
    }

    fn set_exited(&mut self, status: isize) {
        let _m = self.mu.lock().unwrap();
        let time = OffsetDateTime::now_utc();
        self.state = ProcessState::Stopped;
        self.exited = Some(time);
        self.status = status;
    }

    fn state(&self) -> io::Result<ProcessState> {
        let _m = self.mu.lock().unwrap();
        match self.state {
            ProcessState::Unknown => Err(io::ErrorKind::NotFound.into()),
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

    fn exited_at(&self) -> Option<OffsetDateTime> {
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
        let rx = self
            .wait_block
            .take()
            .ok_or_else(|| io::ErrorKind::NotFound)?;
        executor::block_on(async { rx.await.map_err(|_| io::ErrorKind::Other) })?;
        self.state = ProcessState::Stopped;
        Ok(())
    }

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
