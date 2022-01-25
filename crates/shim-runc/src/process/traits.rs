use super::config::ExecConfig;
use super::io::StdioConfig;
use super::state::ProcessState;
use chrono::{DateTime, Utc};
use std::io;
pub trait InitState {
    fn start(&mut self) -> io::Result<()>;
    fn delete(&mut self) -> io::Result<()>;
    fn pause(&mut self) -> io::Result<()>;
    fn resume(&mut self) -> io::Result<()>;
    fn update(&mut self, resource_config: Option<&dyn std::any::Any>) -> io::Result<()>;
    // FIXME: suspended for difficulties
    // fn checkpoint(&self) -> io::Result<()>;
    fn exec(&self, config: ExecConfig) -> io::Result<()>; // FIXME: Result<dyn impl Process>
    fn kill(&mut self, sig: u32, all: bool) -> io::Result<()>;
    fn set_exited(&mut self, status: isize);
    fn state(&self) -> io::Result<ProcessState>;
}

pub trait Process {
    fn id(&self) -> String;
    fn pid(&self) -> isize;
    fn exit_status(&self) -> isize;
    fn exited_at(&self) -> Option<DateTime<Utc>>;
    // FIXME: suspended for difficulties
    // fn stdin(&self) -> ???;
    fn stdio(&self) -> StdioConfig;
    fn wait(&mut self) -> io::Result<()>;
    // FIXME: suspended for difficulties
    // fn resize(&self) -> io::Result<()>;
    fn start(&mut self) -> io::Result<()>;
    fn delete(&mut self) -> io::Result<()>;
    fn kill(&mut self, sig: u32, all: bool) -> io::Result<()>;
    fn set_exited(&mut self, status: isize);
    fn state(&self) -> io::Result<ProcessState>;
}

pub trait ContainerProcess: InitState + Process {}
