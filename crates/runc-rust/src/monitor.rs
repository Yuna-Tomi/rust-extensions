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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::error;
use std::process::Output;
use tokio::sync::oneshot::{Receiver, Sender};

use crate::dbg::*;

// ProcessMonitor for handling runc process exit
// Implementation is different from Go's, because if you return Sender in start() and want to
// use it in wait(), then start and wait cannot be executed concurrently.
// Alternatively, caller of start() and wait() have to prepare channel
#[async_trait]
pub trait ProcessMonitor {
    /// Caller cand choose [`std::mem::forget`] about resource
    /// associated to that command, e.g. file descriptors.
    async fn start(
        &self,
        mut cmd: tokio::process::Command,
        tx: Sender<Exit>,
        forget: bool,
    ) -> std::io::Result<Output> {
        debug_log!("spawn command... {:?}", cmd);
        let chi = cmd.spawn().map_err(|e| {
            debug_log!("{}", e);
            e
        })?;
        let pid = chi.id(); // this cause panic, because tokio::process::Child returns None after child is polled to completion.
        debug_log!("command spawned {:?}", cmd);
        let out = chi.wait_with_output().await?;
        let ts = Utc::now();
        match tx.send(Exit {
            ts,
            pid, 
            status: out.status.code().unwrap(),
        }) {
            Ok(_) => {
                debug_log!("command and notification succeeded: {:?}", cmd);
                if forget {
                    std::mem::forget(cmd);
                }
                Ok(out)
            }
            Err(e) => {
                error!("command {:?} exited but receiver dropped.", cmd);
                error!("couldn't send messages: {:?}", e);
                Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused))
            }
        }
    }
    async fn wait(&self, rx: Receiver<Exit>) -> std::io::Result<Exit> {
        debug_log!("waiting...");
        rx.await.map_err(|e| {
            error!("sender dropped.");
            std::io::Error::from(std::io::ErrorKind::BrokenPipe)
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct DefaultMonitor {}

impl ProcessMonitor for DefaultMonitor {}

impl DefaultMonitor {
    pub const fn new() -> Self {
        Self {}
    }
}

#[derive(Debug)]
pub struct Exit {
    pub ts: DateTime<Utc>,
    pub pid: Option<u32>,
    pub status: i32,
}
