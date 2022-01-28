use chrono::Local;
use once_cell::sync::Lazy;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use std::{fs::File, sync::Mutex};

pub static LOG_STATIC_DBG: Lazy<Mutex<File>> = Lazy::new(|| {
    Mutex::new({
        let mut path = String::new();
        let mut f = File::open("/root/debug_dir.txt").unwrap();
        f.read_to_string(&mut path).unwrap();
        drop(f);

        let r = rand::random::<u16>();
        let now = Local::now().format("%Y:%m:%d-%H:%M:%S").to_string();
        let logfile = Path::new(&path).join(&format!("debug-runc{}-{}.log", now, r));
        // panic!("{}", format!("{}{}.log", LOGFILE, now));
        OpenOptions::new()
            .write(true)
            .create(true)
            .open(logfile)
            .unwrap()
    })
});

#[macro_export]
macro_rules! debug_log {
    ($fmt: expr) => {
        {
            let mut l = LOG_STATIC_DBG.try_lock().unwrap();
            writeln!(*l, $fmt).unwrap();
            l.flush().unwrap();
        }
	};

	($fmt: expr, $($arg: tt)*) =>{
        {
            let mut l = LOG_STATIC_DBG.try_lock().unwrap();
            writeln!(*l, $fmt, $($arg)*).unwrap();
            l.flush().unwrap();
        }
	};
}

#[macro_export]
macro_rules! check_fds {
    () => {{
        let _out = std::process::Command::new("ls")
            .arg("-l")
            .arg("/proc/self/fd")
            .output()
            .map_err(|e| {
                debug_log!("{}", e);
                e
            })
            .unwrap();
        let _out = String::from_utf8(_out.stdout).unwrap();
        _out.split("\n")
            .map(|s| s.to_string())
            .collect::<Vec<String>>()
    }};
}
