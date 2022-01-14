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
use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Unable to extract test files: {0}")]
    BundleExtractError(io::Error),

    #[error("Invalid path: {0}")]
    InvalidPathError(io::Error),

    #[error(transparent)]
    JsonDeserializationError(#[from] serde_json::error::Error),

    #[error("Missing container statistics")]
    MissingContainerStatsError,

    #[error(transparent)]
    ProcessSpawnError(io::Error),

    #[error("Runc command failed, stdout: \"{stdout}\", \"{stderr}\"")]
    RuncCommandError { stdout: String, stderr: String },

    #[error("Runc command timed out: {0}")]
    RuncCommandTimeoutError(tokio::time::error::Elapsed),

    #[error("Unable to parse runc version")]
    RuncInvalidVersionError,

    #[error("Unable to locate the runc")]
    RuncNotFoundError,

    #[error("Failed to spec file: {0}")]
    SpecFileCreationError(io::Error),

    #[error(transparent)]
    SpecFileCleanupError(io::Error),

    #[error("Failed to filnd valid path for spec file")]
    SpecFilePathError,

    #[error("Top command is missing a pid header")]
    TopMissingPidHeaderError,

    #[error("Top command returned an empty response")]
    TopShortResponseError,

    #[error("Unix socket connection error: {0}")]
    UnixSocketConnectionError(io::Error),

    #[error("Unable to bind to unix socket: {0}")]
    UnixSocketOpenError(io::Error),

    #[error("Unix socket failed to receive pty")]
    UnixSocketReceiveMessageError,

    #[error("Unix socket unexpectedly closed")]
    UnixSocketUnexpectedCloseError,

    #[error("Sorry, this part of api is not implemented: {0}")]
    UnimplementedError(String),
}
