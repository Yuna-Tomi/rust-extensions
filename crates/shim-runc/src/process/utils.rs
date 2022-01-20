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
// use runc::{error::Error, RuncClient, RuncConfig};

use std::error::Error;
use std::fs::OpenOptions;
use std::io::Read;
use std::num::ParseIntError;
use std::path::Path;

// fn read_pid_file(pid_file: &str) -> Result<isize, Box<dyn std::error::Error>> {
//     let mut pid_f = OpenOptions::new().read(true).open(&pid_file)?;
//     let mut pid_str = String::new();
//     pid_f.read_to_string(&mut pid_str)?;
//     pid_str.parse::<isize>().map_err(|e| Box::new(e.source().unwrap())) // content of init.pid is always a number
// }
