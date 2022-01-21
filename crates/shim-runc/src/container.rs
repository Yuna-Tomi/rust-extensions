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

use nix::errno::Errno;
use nix::sys::stat;
use nix::unistd;
use protobuf::reflect::ProtobufValue;
use serde_json;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use sys_mount::{MountFlags, SupportedFilesystems, UnmountFlags};

use crate::options::oci::Options;
use crate::process::{
    config::{CreateConfig, MountConfig},
    process::{InitProcess, Process},
};

use crate::utils::{self, new_runc};
pub use containerd_shim_protos as protos;
use log::warn;
use protobuf::{Message, RepeatedField};
use protos::shim::{
    empty::Empty,
    shim::{
        CreateTaskRequest, CreateTaskResponse, DeleteRequest, DeleteResponse, ExecProcessRequest,
        ExecProcessResponse, KillRequest, StartRequest, StartResponse,
    },
};

// for debug
use crate::{debug::LOG, debug_log};

const OPTIONS_FILENAME: &str = "options.json";

#[derive(Debug, Clone, Default)]
/// Struct for managing runc containers.
pub struct Container {
    mu: Arc<Mutex<()>>,
    id: String,
    bundle: String,
    // cgroup: impl protos::api:: ,
    /// This container's process itself. (e.g. init process)
    process_self: InitProcess,
    /// processes running inside this container.
    processes: HashMap<String, InitProcess>,
}

impl Container {
    /// When this struct is created, container is ready to create.
    /// That means, mounting rootfs is done etc.
    pub fn new(req: protos::shim::shim::CreateTaskRequest) -> io::Result<Self> {
        // FIXME
        let namespace = "default".to_string();

        let opts = if req.options.is_some() && req.options.as_ref().unwrap().get_type_url() != "" {
            // FIXME: option should be unmarshaled
            // https://github.com/containerd/containerd/blob/main/runtime/v2/runc/container.go#L52
            // let v = unmarshal_any(req.options);
            // v.options.clone();
            Options::default()
        } else {
            Options::default()
        };

        let mut mounts = Vec::new();
        for mnt in &req.rootfs {
            mounts.push(MountConfig::from_proto_mount(mnt.clone()));
        }

        let rootfs = if mounts.len() > 0 {
            let path = Path::new(&req.bundle).join("rootfs");
            debug_log!("mkdir rootfs: {:?}", path);
            match unistd::mkdir(&path, stat::Mode::from_bits_truncate(0o711)) {
                Ok(_) | Err(Errno::EEXIST) => {}
                Err(e) => return Err(io::Error::from(e)),
            };
            path
        } else {
            PathBuf::new()
        };
        debug_log!("mkdir succeeded! rootfs={:?}", rootfs);

        let config = CreateConfig {
            id: req.id.clone(),
            bundle: req.bundle.clone(),
            runtime: opts.binary_name.clone(),
            rootfs: mounts.clone(),
            terminal: req.terminal,
            stdin: req.stdin.clone(),
            stdout: req.stdout.clone(),
            stderr: req.stderr.clone(),
            options: req.options.clone().into_option(),
        };

        // Write options to file, which will be removed when shim stops.
        match write_options(&req.bundle, &opts) {
            Ok(_) => {}
            Err(e) => {
                debug_log!("{}", e);
                return Err(e);
            }
        }

        // For historical reason, we write binary name as well as the entire opts
        write_runtime(&req.bundle, &opts.binary_name)?;

        // split functionality in order to cleanup rootfs when error occurs after mount.
        Self::inner_new(&rootfs, req, namespace, opts, config, mounts).map_err(|e| {
            if let Err(_) = sys_mount::unmount(rootfs, UnmountFlags::empty()) {
                debug_log!("failed to cleanup mounts.");
            }
            e
        })
    }

    fn inner_new<R>(
        rootfs: R,
        req: protos::shim::shim::CreateTaskRequest,
        namespace: String,
        opts: Options,
        config: CreateConfig,
        mounts: Vec<MountConfig>,
    ) -> io::Result<Self>
    where
        R: AsRef<Path>,
    {
        for mnt in mounts {
            debug_log!("call utils::mount: {:?}", mnt);
            utils::mount(mnt, &rootfs)?;
            debug_log!("mount succeeded!");
        }
        let id = req.id.clone();
        let bundle = req.bundle.clone();

        debug_log!("call InitProcess::new: {:?}", bundle);
        let mut init = InitProcess::new(
            &bundle,
            Path::new(&bundle).join("work"),
            namespace,
            config.clone(),
            opts,
            rootfs,
        );

        debug_log!("call init create: {:?}", config);
        // create the init process
        init.create(config)?;
        let pid = init.pid();
        debug_log!("init successfully created: pid={}", pid);

        if pid > 0 {
            // FIXME: setting config for cgroup
        }

        Ok(Container {
            id,
            bundle,
            process_self: init,
            ..Default::default()
        })
    }

    // pub fn all(&self) /* -> [] */
    // {
    //     match self.mu.lock() {
    //         Ok(m) => {}
    //         Err(e) => {}
    //     }
    // }

    // pub fn execd_processes(&self) /* -> [] */
    // {
    //     match self.mu.lock() {
    //         Ok(m) => {}
    //         Err(e) => {}
    //     }
    // }

    pub fn pid(&self) -> isize {
        let _m = self.mu.lock().unwrap();
        self.process_self.pid
    }

    // pub fn cgroup(&self) /* -> [] */
    // {
    //     match self.mu.lock() {
    //         Ok(m) => {}
    //         Err(e) => {}
    //     }
    // }

    // pub fn cgroup_set(&self) /* -> [] */
    // {
    //     match self.mu.lock() {
    //         Ok(m) => {}
    //         Err(e) => {}
    //     }
    // }

    // pub fn reserve_process(&self) /* -> [] */
    // {
    //     match self.mu.lock() {
    //         Ok(m) => {}
    //         Err(e) => {}
    //     }
    // }

    // pub fn process_add(&self) /* -> [] */
    // {
    //     match self.mu.lock() {
    //         Ok(m) => {}
    //         Err(e) => {}
    //     }
    // }

    pub fn process_remove(&mut self, id: &str) /* -> [] */
    {
        let _m = self.mu.lock().unwrap();
        let _ = self.processes.remove(id);
    }

    pub fn process(&self, id: &str) -> Result<InitProcess, Box<dyn std::error::Error>> {
        let _m = self.mu.lock().unwrap();
        // Might be ugly hack: is it good multiple "InitProcess"s that represent same process exist?
        if id == "" {
            Ok(self.process_self.clone())
        } else {
            let p = self
                .processes
                .get(id)
                .ok_or_else(|| ttrpc::Error::Others("process does not exists".to_string()))?;
            Ok(p.clone())
        }
    }

    /// Start a container process and return its pid
    pub fn start(&mut self, req: StartRequest) -> Result<isize, Box<dyn std::error::Error>> {
        let _m = self.mu.lock().unwrap();
        // Might be ugly hack: is it good multiple "InitProcess"s that represent same process exist?
        let p = if req.id == "" {
            &mut self.process_self
        } else {
            self.processes
                .get_mut(&req.id)
                .ok_or_else(|| ttrpc::Error::Others("process does not exists".to_string()))?
        };
        p.start()?;
        Ok(p.pid)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let p = self.process(id)?;

        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn exec(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn pause(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn resume(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn resize_pty(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn kill(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn close_io(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn checkpoint(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn update(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }

    pub fn has_pid(&self) -> Result<(), Box<dyn std::error::Error>> {
        Err(Box::new(ttrpc::Error::Others(
            "not implemented yet".to_string(),
        )))
    }
}

// // FIXME: define config
// fn new_init_process<P, W, R>(
//     path: P,
//     work_dir: W,
//     namespace: &str,
//     config: &str,
//     options: Options,
//     rootfs: R,
// ) -> io::Result<()> {
//     let runtime = new_runc(Some(options.root.clone()), path, namespace, runtime, systemd)
//     Ok(())
// }

/// reads the option information from the path.
/// When the file does not exist, returns [`None`] without an error.
fn read_options<P>(path: P) -> io::Result<Option<Options>>
where
    P: AsRef<Path>,
{
    let file_path = path.as_ref().join(OPTIONS_FILENAME);
    let f = match File::open(file_path) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    // NOTE: serde_json::from_reader is usually slower than from_str or from_slice
    // after read file contents into memory.
    let mut reader = BufReader::new(f);
    let msg = Message::parse_from_reader(&mut reader)?;
    Ok(Some(msg))
}

fn write_options<P>(path: P, opts: &Options) -> io::Result<()>
where
    P: AsRef<Path>,
{
    let file_path = path.as_ref().join(OPTIONS_FILENAME);
    debug_log!("write options.");
    let f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o600)
        .open(&file_path)?;
    let mut writer = BufWriter::new(f);
    opts.write_to_writer(&mut writer)?;
    writer.flush()?;
    debug_log!("write options succeeded: {:?}", file_path.as_os_str());
    Ok(())
}

fn read_runtime<P>(path: P) -> Result<Option<Options>, Box<dyn std::error::Error>>
where
    P: AsRef<Path>,
{
    Err(Box::new(ttrpc::Error::Others(
        "not implemented yet".to_string(),
    )))
}

fn write_runtime<P, R>(path: P, runtime: R) -> io::Result<()>
where
    P: AsRef<Path>,
    R: AsRef<str>,
{
    debug_log!("write runtime.");
    let file_path = path.as_ref().join("runtime");
    let f = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o600)
        .open(&file_path)?;
    let mut writer = BufWriter::new(f);
    writer.write_all(runtime.as_ref().as_bytes())?;
    debug_log!("write runtime succeeded: {:?}", file_path.as_os_str());
    Ok(())
}

fn new_container() -> Result<Container, Box<dyn std::error::Error>> {
    Err(Box::new(ttrpc::Error::Others(
        "not implemented yet".to_string(),
    )))
}
