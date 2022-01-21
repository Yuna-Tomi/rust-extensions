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

use crate::container::Container;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::thread::sleep;
use std::time::Duration;

use containerd_runc_rust as runc;
use containerd_shim as shim;
use containerd_shim_protos as protos;

use log::info;
use nix::pty::PtyMaster;
use once_cell::sync::Lazy;
use protobuf::well_known_types::Timestamp;
use protobuf::{RepeatedField, SingularPtrField};
use protos::shim::{
    empty::Empty,
    shim::{
        CreateTaskRequest, CreateTaskResponse, DeleteRequest, DeleteResponse, ExecProcessRequest,
        ExecProcessResponse, KillRequest, StartRequest, StartResponse,
    },
};
use runc::console::ReceivePtyMaster;
use runc::error::Error as RuncError;
use runc::options::*;
use runc::{RuncClient, RuncConfig};
use shim::{api, ExitSignal, TtrpcContext, TtrpcResult};
use ttrpc::{Code, Status};
use uuid::Uuid;

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
    runtime_id: String,
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
        let exit = ExitSignal::default();
        debug_log!("shim service successfully created.");
        Self { runtime_id, exit }
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
}

impl shim::Task for Service {
    fn create(
        &self,
        _ctx: &ttrpc::TtrpcContext,
        _req: CreateTaskRequest,
    ) -> ttrpc::Result<CreateTaskResponse> {
        debug_log!("TTRPC call: create");
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

        debug_log!("TTRPC call succeeded: create");
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
        debug_log!("TTRPC call: start");
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
        let pid = container.start(_req.clone()).map_err(|_|
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

    // fn kill(
    //     &self,
    //     _ctx: &ttrpc::TtrpcContext,
    //     _req: KillRequest,
    // ) -> ttrpc::Result<Empty> {
    //     let opts = KillOpts::new().all(_req.all);
    //     self.runc
    //         .kill(&_req.id, _req.signal, Some(&opts))
    //         .map_err(|e| {
    //             let (code, message) = err_mapping(e);
    //             ttrpc::Error::RpcStatus(Status {
    //                 code,
    //                 message,
    //                 details: RepeatedField::new(),
    //                 unknown_fields: _req.unknown_fields.clone(),
    //                 cached_size: _req.cached_size.clone(),
    //             })
    //         })?;
    //     Ok(containerd_shim_protos::shim::empty::Empty {
    //         unknown_fields: _req.unknown_fields,
    //         cached_size: _req.cached_size,
    //     })
    // }

    // fn delete(
    //     &self,
    //     _ctx: &ttrpc::TtrpcContext,
    //     _req: DeleteRequest,
    // ) -> ttrpc::Result<DeleteResponse> {
    //     let opts = DeleteOpts::new().force(true);
    //     let res = self.runc.delete(&_req.id, Some(&opts)).map_err(|e| {
    //         let (code, message) = err_mapping(e);

    //         ttrpc::Error::RpcStatus(Status {
    //             code,
    //             message,
    //             details: RepeatedField::new(),
    //             unknown_fields: _req.unknown_fields.clone(),
    //             cached_size: _req.cached_size.clone(),
    //         })
    //     })?;
    //     Ok(DeleteResponse {
    //         pid: res.pid,
    //         exit_status: res.status.code().unwrap() as u32,
    //         exited_at: SingularPtrField::default(),
    //         unknown_fields: _req.unknown_fields,
    //         cached_size: _req.cached_size,
    //     })
    // }

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
            RuncError::BundleExtractError(_) => Code::FAILED_PRECONDITION,
            RuncError::NotFoundError => Code::NOT_FOUND,
            RuncError::UnimplementedError(_) => Code::UNIMPLEMENTED,
            RuncError::CommandError(_) | RuncError::CommandFaliedError { .. } => Code::ABORTED,
            RuncError::CommandTimeoutError(_) => Code::DEADLINE_EXCEEDED,
            _ => Code::UNKNOWN,
        },
        e.to_string(),
    )
}
