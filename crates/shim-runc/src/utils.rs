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
use containerd_runc_rust as runc;
use containerd_shim_protos as protos;
use runc::{error::Error, RuncClient, RuncConfig};
use std::{io, path::Path};
use sys_mount::{Mount, MountFlags, SupportedFilesystems};

use crate::process::config::MountConfig;

/// A mount helper, similar to Go version.
pub struct MountUtil {
    /// Type specifies the host-specific of the mount.
    pub mount_type: String,
    /// Source specifies where to mount from. Depending on the host system, this can be a source path or device.
    pub source: String,
    /// Options contains zero or more fstab-style mount options. Typically, these are platform specific.
    pub options: Vec<String>,
    pub mount: Mount,
}

// impl MountUtil {
//     pub fn new<P>(&self, target: P) -> io::Result<Mount>
//     where
//         P: AsRef<Path>,
//     {
//         let fs = SupportedFilesystems::new()?;
//         Some(Mount::new(&self.source, target, &fs, MountFlags::empty(), None))
//     }
// }

pub fn mount<T>(mnt: MountConfig, target: T) -> io::Result<()>
where
    T: AsRef<Path>,
{
    let fs = SupportedFilesystems::new()?;
    let m = sys_mount::Mount::new(&mnt.source, target, &fs, MountFlags::empty(), None)?;
    Ok(())
}

// pub fn unmount<P>(path: P) -> io::Result<()> {

// }
const DEFAULT_RUNC_ROOT: &str = "/run/containerd/runc";

// NOTE: checkpoint is not supported now, then skipping criu for args.
pub fn new_runc<R, P>(
    root: Option<R>,
    path: P,
    namespace: String,
    runtime: String,
    systemd_cgroup: bool,
) -> Result<RuncClient, Error>
where
    R: AsRef<Path>,
    P: AsRef<Path>,
{
    let root = if let Some(r) = &root {
        r.as_ref()
    } else {
        Path::new(DEFAULT_RUNC_ROOT)
    }
    .join(namespace);
    let log = path.as_ref().join("log.json");

    let config = RuncConfig::new()
        .command(runtime)
        .log(log)
        .log_format_json()
        .root(root)
        .systemd_cgroup(systemd_cgroup);

    // FIXME: not good to unwrap() here
    // NOTE: this returns error only if the runc binary does not exists.
    RuncClient::from_config(config)
}
