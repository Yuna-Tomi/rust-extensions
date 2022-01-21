use chrono::{DateTime, Local, Utc};
use once_cell::sync::Lazy;
use std::fs::OpenOptions;
use std::{fs::File, sync::Mutex};
const LOGFILE: &str = "/home/ytomida.linux/nerd_dev/rust-extensions/crates/.mydebug/debug-shim";
pub static LOG: Lazy<Mutex<File>> = Lazy::new(|| {
    Mutex::new({
        let now = Local::now().format("%Y:%m:%d-%H:%M:%S").to_string();
        // panic!("{}", format!("{}{}.log", LOGFILE, now));
        OpenOptions::new()
            .write(true)
            .create(true)
            .open(&format!("{}{}.log", LOGFILE, now))
            .unwrap()
    })
});

#[macro_export]
macro_rules! debug_log {
    ($fmt: expr) => {
        {
            let mut l = LOG.try_lock().unwrap();
            write!(*l, "{}", format!(concat!($fmt, "\n"))).unwrap();
            l.flush().unwrap();
        }
	};

	($fmt: expr, $($arg: tt)*) =>{
        {
            let mut l = LOG.try_lock().unwrap();
            write!(*l, "{}", format!(concat!($fmt, "\n"), $($arg)*)).unwrap();
            l.flush().unwrap();
        }
	};
}
