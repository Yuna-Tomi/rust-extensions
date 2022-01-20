use chrono::Duration;
use containerd_runc_rust as runc;
// NOTE: This test is not ready to available on repository

use log::warn;
use runc::{
    console::ReceivePtyMaster, error::Error, options::*, RuncAsyncClient, RuncClient, RuncConfig,
};
use std::path::{Path, PathBuf};
use std::{
    env,
    fs::{self, File},
    io,
};
use tar::Archive;
use uuid::Uuid;
use xz2::read::XzDecoder;

const RUNC: &str = "runc";
const RUNC_TAR: &str = "test_fixture/runc_v1.0.3.tar.xz";
const BUNDLE_TAR: &str = "/run/runc-rust";
const RUN_DIR: &str = "/run/runc-rust";

fn prepare_runc(bin: impl AsRef<Path>) -> RuncClient {
    let runc_id = Uuid::new_v4().to_string();
    let runc_dir = env::temp_dir().join(&runc_id);
    let runc_path = runc_dir.join(bin);
    let runc_root = PathBuf::from(RUN_DIR).join(&runc_id);
    fs::create_dir_all(&runc_root).expect("unable to create runc root");
    extract_tarball(RUNC_TAR, runc_dir).expect("unable to extract runc");
    RuncConfig::new()
        .command(runc_path)
        .root(runc_root)
        .build()
        .expect("unable to create runc instance")
}

/// Extract an OCI bundle tarball to a directory
fn extract_tarball(tarball: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    let mut archive = Archive::new(XzDecoder::new(File::open(tarball)?));
    archive.unpack(dst)?;
    Ok(())
}

// #[test]
fn command_test() {
    let runc = prepare_runc(RUNC);
}

struct SandBox {
    id: PathBuf,
    runc: RuncClient,
}

impl SandBox {
    async fn new(
        runc_path: impl AsRef<Path>,
        runc_root: impl AsRef<Path>,
        compressed_bundle: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let id = Uuid::new_v4().to_string();
        // let bundle =
        Err(Error::UnimplementedError("".to_string()))
    }
}

//
const BUNDLE_PATH: &str = "tests/bundle";
const LOG_PATH: &str = "tests/bundle/log.json";
#[tokio::test]
async fn test() {
    let console = ReceivePtyMaster::new_with_temp_sock().unwrap();
    let runc = RuncConfig::new()
        .command(RUNC)
        .log(LOG_PATH)
        .log_format_json()
        .timeout(u64::MAX / 100000)
        .build_async()
        .unwrap();

    let opts = CreateOpts::new().console_socket(console.console_socket.clone());

    tokio::spawn(async move {
        runc.create("myos", BUNDLE_PATH.to_string(), Some(&opts))
            .await
            .expect("runc runc failed.");
    })
    .await
    .unwrap();
}
