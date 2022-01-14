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

use std::collections::HashMap;

use chrono::serde::ts_seconds_option;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Information for runc container
#[derive(Debug, Serialize, Deserialize)]
pub struct Container {
    pub id: Option<String>,
    pub pid: Option<u32>,
    pub status: Option<String>,
    pub bundle: Option<String>,
    pub rootfs: Option<String>,
    #[serde(with = "ts_seconds_option")]
    pub created: Option<DateTime<Utc>>,
    pub annotations: Option<HashMap<String, String>>,
}