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

use crate::container::{self, Container};
use crate::options::oci::Options;
use crate::process::state::ProcessState;
use crate::utils;
use std::collections::HashMap;
use std::env;
use std::sync::RwLock;

use containerd_runc_rust as runc;
use containerd_shim as shim;
use containerd_shim_protos as protos;

use log::info;
use once_cell::sync::Lazy;
use protobuf::well_known_types::Timestamp;
use protobuf::{RepeatedField, SingularPtrField};
use protos::shim::task::Status as TaskStatus;
use protos::shim::{
    empty::Empty,
    shim::{
        CreateTaskRequest, CreateTaskResponse, DeleteRequest, DeleteResponse, ExecProcessRequest,
        ExecProcessResponse, KillRequest, StartRequest, StartResponse, StateRequest, StateResponse,
        WaitRequest, WaitResponse,
    },
};
use runc::console::ReceivePtyMaster;
use runc::error::Error as RuncError;
use runc::options::*;
use shim::{api, ExitSignal, TtrpcContext, TtrpcResult};
use sys_mount::UnmountFlags;
use ttrpc::{Code, Status};

use crate::dbg::*;

// group labels specifies how the shim groups services.
// currently supports a runc.v2 specific .group label and the
// standard k8s pod label.  Order matters in this list
const GROUP_LABELS: [&str; 2] = [
    "io.containerd.runc.v2.group",
    "io.kubernetes.cri.sandbox-id",
];

const RUN_DIR: &str = "/run/containerd/runc";
const TASK_DIR: &str = "/run/containerd/io.containerd.runtime.v2.task";

static PTY_MASTER: Lazy<RwLock<ReceivePtyMaster>> = Lazy::new(|| {
    RwLock::new(ReceivePtyMaster::new_with_temp_sock().expect("failed to bind socket."))
});

static CONTAINERS: Lazy<RwLock<HashMap<String, Container>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

// for debug: forciblly return error
macro_rules! debug_status {
    () => {
        Err(ttrpc::error::Error::RpcStatus(Status::default()))
    };
}

#[derive(Clone)]
pub struct Service {
    /// Runtime id
    runtime_id: String,
    /// Container id
    id: String,
    namespace: String,
    exit: ExitSignal,
}

// impl Service {
//     fn get_container(&self, )

// }

impl shim::Shim for Service {
    type Error = shim::Error;
    type T = Service;

    fn new(
        _runtime_id: &str,
        _id: &str,
        _namespace: &str,
        _publisher: shim::RemotePublisher,
        _config: &mut shim::Config,
    ) -> Self {
        let runtime_id = _runtime_id.to_string();
        let id = _id.to_string();
        let namespace = _namespace.to_string();
        let exit = ExitSignal::default();
        debug_log!("shim service successfully created.");
        Self {
            runtime_id,
            id,
            namespace,
            exit,
        }
    }

    #[cfg(target_os = "linux")]
    fn start_shim(&mut self, opts: shim::StartOpts) -> Result<String, shim::Error> {
        let address = shim::spawn(opts, Vec::new())?;
        debug_log!("shim successfully spawned.");
        Ok(address)
    }

    #[cfg(not(target_os = "linux"))]
    fn start_shim(&mut self, opts: shim::StartOpts) -> Result<String, shim::Error> {
        let address = shim::spawn(opts, Vec::new())?;
        Err(shim::Error::Start(
            "non-linux implementation is not supported now.",
        ))
    }

    fn wait(&mut self) {
        self.exit.wait();
    }

    fn get_task_service(&self) -> Self::T {
        self.clone()
    }

    fn delete_shim(&mut self) -> Result<DeleteResponse, Self::Error> {
        debug_log!("call delete_shim");
        let cwd = env::current_dir()?;
        debug_log!("current dir: {:?}", cwd);
        let parent = cwd.parent().expect("shim running on root directory.");
        let path = parent.join(&self.id);
        // let runtime = container::read_runtime(&path).map_err(|e| Self::Error::Delete(e.to_string()))?;
        // debug_log!("delete_shim: runtime={}", runtime);
        let opts =
            container::read_options(&path).map_err(|e| Self::Error::Delete(e.to_string()))?;
        let root = match opts {
            Some(Options { root, .. }) if root != "" => root,
            _ => RUN_DIR.to_string(),
        };
        let runc = utils::new_runc(&root, &path, self.namespace.clone(), "", false)
            .map_err(|e| Self::Error::Delete(e.to_string()))?;
        let opts = DeleteOpts::new().force(true);
        runc.delete(&self.id, Some(&opts))
            .map_err(|e| Self::Error::Delete(e.to_string()))?;
        sys_mount::unmount(&path.as_path().join("rootfs"), UnmountFlags::empty()).map_err(|e| {
            log::error!("failed to cleanup rootfs mount");
            Self::Error::Delete(e.to_string())
        })?;

        let now = Some(Timestamp {
            // FIXME: for debug
            ..Default::default()
        });
        let exited_at = SingularPtrField::from_option(now);

        debug_log!("successfully deleted shim.");
        Ok(DeleteResponse {
            exited_at,
            exit_status: 137, // SIGKILL + 128
            ..Default::default()
        })
    }
}

impl shim::Task for Service {
    fn create(
        &self,
        _ctx: &ttrpc::TtrpcContext,
        _req: CreateTaskRequest,
    ) -> ttrpc::Result<CreateTaskResponse> {
        debug_log!("TTRPC call: create\nid={}", _req.id);
        // let mut opts = CreateOpts::new().pid_file(pid_file);
        // if _req.terminal {
        //     let pty_master = PTY_MASTER.try_read().unwrap();
        //     opts = opts.console_socket(&pty_master.console_socket);
        // }

        let id = _req.id.clone();
        let unknown_fields = _req.unknown_fields.clone();
        let cached_size = _req.cached_size.clone();
        // FIXME: error handling
        debug_log!("call Container::new()");
        let container = match Container::new(_req) {
            Ok(c) => c,
            Err(e) => {
                debug_log!("container create failed: {:?}", e);
                return Err(ttrpc::Error::Others(format!(
                    "container create failed: id={}, err={}",
                    id, e
                )));
            }
        };
        let mut c = CONTAINERS.write().unwrap();
        let pid = container.pid() as u32;
        if c.contains_key(&id) {
            return Err(ttrpc::Error::Others(format!(
                "create: container \"{}\" already exists.",
                id
            )));
        } else {
            let _ = c.insert(id, container);
        }

        debug_log!("TTRPC call succeeded: create\npid={}", pid);
        // sleep for debug.
        std::thread::sleep(std::time::Duration::from_secs(100));

        Ok(CreateTaskResponse {
            pid,
            unknown_fields,
            cached_size,
        })
    }

    fn start(
        &self,
        _ctx: &ttrpc::TtrpcContext,
        _req: StartRequest,
    ) -> ttrpc::Result<StartResponse> {
        debug_log!(
            "TTRPC call: start\nid={}, exec_id={}",
            _req.get_id(),
            _req.get_exec_id()
        );
        let mut c = CONTAINERS.write().unwrap();
        debug_log!("request: id={}", _req.get_id());

        let container = c.get_mut(_req.get_id()).ok_or_else(|| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: "container not created".to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        debug_log!("call Container::start()");
        let pid = container.start(&_req).map_err(|_|
            // FIXME: appropriate error mapping
            ttrpc::error::Error::RpcStatus(Status {
                code: Code::UNKNOWN,
                message: "couldn't start container process.".to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
        }))?;

        debug_log!("TTRPC call succeeded: start");
        Ok(StartResponse {
            pid: pid as u32,
            unknown_fields: _req.unknown_fields,
            cached_size: _req.cached_size,
        })
    }

    fn exec(
        &self,
        _ctx: &::ttrpc::TtrpcContext,
        _req: ExecProcessRequest,
    ) -> ::ttrpc::Result<Empty> {
        debug_log!("TTRPC call: exec");
        debug_log!("request: id={}", _req.get_id());
        Err(::ttrpc::Error::RpcStatus(::ttrpc::get_status(
            ::ttrpc::Code::NOT_FOUND,
            "/containerd.task.v2.Task/Exec is not supported".to_string(),
        )))
    }

    fn state(
        &self,
        _ctx: &ttrpc::TtrpcContext,
        _req: StateRequest,
    ) -> ttrpc::Result<StateResponse> {
        debug_log!(
            "TTRPC call: state\nid={}, exec_id={}",
            _req.get_id(),
            _req.get_exec_id()
        );

        let c = CONTAINERS.write().unwrap();
        let container = c.get(_req.get_id()).ok_or_else(|| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: "container not created".to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        let exec_id = _req.get_exec_id();
        let p = container.process(exec_id).map_err(|_| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: format!("process {} doesn't exist.", exec_id).to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        #[rustfmt::skip]
        let status = match p.state {
            ProcessState::Unknown   => TaskStatus::UNKNOWN,
            ProcessState::Created   => TaskStatus::CREATED,
            ProcessState::Running   => TaskStatus::RUNNING,
            ProcessState::Stopped |
            ProcessState::Deleted   => TaskStatus::STOPPED,
            ProcessState::Paused    => TaskStatus::PAUSED,
            ProcessState::Pausing   => TaskStatus::PAUSING,
        };

        let stdio = p.stdio();
        debug_log!(
            "TTRPC call succeeded: state\nid={}, exec_id={}",
            _req.get_id(),
            _req.get_exec_id()
        );
        Ok(StateResponse {
            id: _req.exec_id,
            bundle: p.bundle.clone(),
            pid: p.pid() as u32,
            status,
            stdin: stdio.stdin,
            stdout: stdio.stdout,
            stderr: stdio.stderr,
            terminal: stdio.terminal,
            exit_status: p.exit_status() as u32,
            unknown_fields: _req.unknown_fields,
            cached_size: _req.cached_size,
            ..Default::default()
        })
    }

    fn wait(&self, _ctx: &ttrpc::TtrpcContext, _req: WaitRequest) -> ttrpc::Result<WaitResponse> {
        debug_log!(
            "TTRPC call: wait\nid={}, exec_id={}",
            _req.get_id(),
            _req.get_exec_id()
        );

        let mut c = CONTAINERS.write().unwrap();
        let container = c.get_mut(_req.get_id()).ok_or_else(|| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: "container not created".to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        let exec_id = _req.get_exec_id();
        let p = container.process_mut(exec_id).map_err(|_| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: format!("process {} doesn't exist.", exec_id).to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        debug_log!("call InitProcess::wait");
        p.wait().map_err(|e| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: format!("process {} failed: {}", exec_id, e).to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        // Might be ugly hack
        debug_log!("InitProcess::wait succeeded.");
        let exited_at = match p.exited_at() {
            Some(t) => Some(Timestamp {
                // nanos: t.timestamp_nanos() as i32, // ugly hack
                ..Default::default() // all default, just for debug
            }),
            None => None,
        };

        debug_log!(
            "TTRPC call: wait succeeded \nid={}, exec_id={}",
            _req.get_id(),
            _req.get_exec_id()
        );
        Ok(WaitResponse {
            exit_status: p.exit_status() as u32,
            exited_at: SingularPtrField::from_option(exited_at),
            unknown_fields: _req.unknown_fields,
            cached_size: _req.cached_size,
        })
    }

    fn kill(&self, _ctx: &ttrpc::TtrpcContext, _req: KillRequest) -> ttrpc::Result<Empty> {
        debug_log!("TTRPC call: kill");
        debug_log!("request: id={}", _req.get_id());

        let mut c = CONTAINERS.write().unwrap();
        let container = c.get_mut(_req.get_id()).ok_or_else(|| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: "container not created".to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        container.kill(&_req).map_err(|e| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: format!("failed to kill the container {}: {}", _req.id, e),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        debug_log!("TTRPC succeeded: kill");
        Ok(containerd_shim_protos::shim::empty::Empty {
            unknown_fields: _req.unknown_fields,
            cached_size: _req.cached_size,
        })
    }

    fn delete(
        &self,
        _ctx: &ttrpc::TtrpcContext,
        _req: DeleteRequest,
    ) -> ttrpc::Result<DeleteResponse> {
        debug_log!("TTRPC call: delete");
        debug_log!("request: id={}", _req.get_id());

        let mut c = CONTAINERS.write().unwrap();
        let container = c.get_mut(_req.get_id()).ok_or_else(|| {
            ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: "container not created".to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields.clone(),
                cached_size: _req.cached_size.clone(),
            })
        })?;

        match container.delete(&_req) {
            Ok((pid, exit_status, exited_at)) => {
                debug_log!("TTRPC call succeeded: delete");
                // Might be ugly hack
                let exited_at = match exited_at {
                    Some(t) => Some(Timestamp {
                        // nanos: t.timestamp_nanos() as i32, // ugly hack
                        ..Default::default() // all default, just for debug.
                    }),
                    None => None,
                };

                Ok(DeleteResponse {
                    pid: pid as u32,
                    exit_status: exit_status as u32,
                    exited_at: SingularPtrField::from_option(exited_at),
                    unknown_fields: _req.unknown_fields,
                    cached_size: _req.cached_size,
                })
            }
            _ => Err(ttrpc::Error::RpcStatus(Status {
                code: Code::NOT_FOUND,
                message: "failed to delete container.".to_string(),
                details: RepeatedField::new(),
                unknown_fields: _req.unknown_fields,
                cached_size: _req.cached_size,
            })),
        }
    }

    fn connect(
        &self,
        _ctx: &TtrpcContext,
        _req: api::ConnectRequest,
    ) -> TtrpcResult<api::ConnectResponse> {
        info!("Connect request");
        Ok(api::ConnectResponse {
            version: self.runtime_id.clone(),
            ..Default::default()
        })
    }

    fn shutdown(&self, _ctx: &TtrpcContext, _req: api::ShutdownRequest) -> TtrpcResult<Empty> {
        info!("Shutdown request");
        self.exit.signal();
        Ok(Empty::default())
    }
}

fn err_mapping(e: RuncError) -> (Code, String) {
    (
        match e {
            RuncError::BundleExtractFailed(_) => Code::FAILED_PRECONDITION,
            RuncError::NotFound => Code::NOT_FOUND,
            RuncError::Unimplemented(_) => Code::UNIMPLEMENTED,
            RuncError::InvalidCommand(_) | RuncError::CommandFailed { .. } => Code::ABORTED,
            RuncError::CommandTimeout(_) => Code::DEADLINE_EXCEEDED,
            _ => Code::UNKNOWN,
        },
        e.to_string(),
    )
}
