use once_cell::sync::Lazy;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use std::{fs::File, sync::Mutex};
use time::OffsetDateTime;

pub static M: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
pub static LOG_STATIC_DBG: Lazy<Mutex<File>> = Lazy::new(|| {
    Mutex::new({
        let mut path = String::new();
        let mut f = File::open("/root/debug_dir.txt").unwrap();
        f.read_to_string(&mut path).unwrap();
        drop(f);

        let r = rand::random::<u16>();
        let now = OffsetDateTime::now_utc().to_string();
        let logfile = Path::new(&path).join(&format!("debug-shim{}-{}.log", now, r));
        OpenOptions::new()
            .write(true)
            .create(true)
            .open(logfile)
            .unwrap()
    })
});

pub static LOG_FILE_NAME: Lazy<String> = Lazy::new(|| {
    let mut path = String::new();
    let mut f = File::open("/root/debug_dir.txt").unwrap();
    f.read_to_string(&mut path).unwrap();
    drop(f);

    let r = rand::random::<u16>();
    let now = OffsetDateTime::now_utc().to_string();
    let logfile = Path::new(&path).join(&format!("debug-shim{}-{}.log", now, r));
    logfile.to_string_lossy().parse::<String>().unwrap()
});

// #[macro_export]
// macro_rules! debug_log {
//     ($fmt: expr) => {
//         {
//             let _m = M.lock().unwrap();
//             let mut f = std::fs::OpenOptions::new()
//                 .write(true)
//                 .create(true)
//                 .open(&*LOG_FILE_NAME)
//                 .unwrap();
//             f.write_all($fmt.as_bytes()).unwrap();
//             f.flush().unwrap();
//             drop(f);
//             drop(_m);
//         }
// 	};

// 	($fmt: expr, $($arg: tt)*) =>{
//         {
//             let _m = M.lock().unwrap();
//             let mut f = std::fs::OpenOptions::new()
//                 .write(true)
//                 .create(true)
//                 .open(&*LOG_FILE_NAME)
//                 .unwrap();
//             f.write_all(format!($fmt, $($arg)*).as_bytes()).unwrap();
//             f.flush().unwrap();
//             drop(f);
//             drop(_m);
//         }
// 	};
// }

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
