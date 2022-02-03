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

/// Generated option structures.
#[rustfmt::skip]
pub mod options;

pub use containerd_shim as shim;
pub use containerd_shim_protos as protos;
pub use runc;

pub mod v2 {
    pub use crate::options::oci::*;
    pub use crate::service::Service;
}

mod container;
mod debug;
mod process;
mod service;
mod utils;

use crate::service::Service;

pub mod dbg {
    pub use crate::debug::*;
    pub use crate::{check_fds, debug_log};
    pub use std::io::Write as DbgWrite;
}
use dbg::*;

fn main() {
    // all arguments will be parsed inside "run" function.
    shim::run::<Service>("io.containerd.runc.v2");
    debug_log!("stop main.");
}
