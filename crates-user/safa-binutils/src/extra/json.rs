use std::{fs::File, io::BufReader};

use serde::Deserialize;

#[derive(Deserialize)]
pub struct MemInfo {
    total: usize,
    free: usize,
    used: usize,
}
use std::io;

use crate::ostring_to_string;

impl MemInfo {
    pub const fn total(&self) -> usize {
        self.total
    }

    pub const fn total_kib(&self) -> usize {
        self.total / 1024
    }

    pub const fn total_mib(&self) -> usize {
        self.total_kib() / 1024
    }

    pub const fn used(&self) -> usize {
        self.used
    }

    pub const fn used_kib(&self) -> usize {
        self.used / 1024
    }

    pub const fn used_mib(&self) -> usize {
        self.used_kib() / 1024
    }

    pub const fn free(&self) -> usize {
        self.free
    }

    pub const fn free_kib(&self) -> usize {
        self.free / 1024
    }

    pub const fn free_mib(&self) -> usize {
        self.free_kib() / 1024
    }

    pub fn fetch() -> io::Result<Self> {
        let file = File::open("proc:/meminfo")?;
        let reader = BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    }
}

#[derive(Deserialize)]
pub struct ProcessInfo {
    // TODO: define a maximum name length in abi
    name: heapless::String<128>,
    is_alive: bool,
    pid: u32,
    ppid: u32,
}

impl ProcessInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_alive(&self) -> bool {
        self.is_alive
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn ppid(&self) -> u32 {
        self.ppid
    }

    /// Gets all process info in `proc:/`
    /// returns a Vector of ProcessInfo, longest process name length, and longest pid length (as str) if sucessfull for formating pruposes
    pub fn fetch_all() -> io::Result<(Vec<ProcessInfo>, usize, usize)> {
        let mut processes = Vec::new();
        let mut longest_name = 0;
        let mut longest_pid = 0;

        let dir = std::fs::read_dir("proc:/")?;

        for entry in dir {
            let entry = entry.unwrap();
            let name = ostring_to_string(entry.file_name());

            if let Ok(_) = name.parse::<usize>() {
                let info_file_path = entry.path().join("info");

                let file = File::open(info_file_path)?;
                let reader = BufReader::new(file);

                let info: ProcessInfo = serde_json::from_reader(reader)?;

                if name.len() > longest_pid {
                    longest_pid = name.len();
                }

                if info.name().len() > longest_name {
                    longest_name = info.name().len();
                }

                processes.push(info);
            }
        }

        Ok((processes, longest_name, longest_pid))
    }
}

#[derive(Deserialize)]
pub struct CpuInfo {
    vendor_id: heapless::String<128>,
    model: heapless::String<128>,
}

impl CpuInfo {
    pub fn vendor(&self) -> &str {
        &self.vendor_id
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn fetch() -> io::Result<Self> {
        let file = File::open("proc:/cpuinfo")?;
        let reader = BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    }
}

#[derive(Deserialize)]
pub struct KernelInfo {
    name: heapless::String<128>,
    version: heapless::String<128>,
    compile_time: String,
    compile_date: String,
    uptime: u64,
}

impl KernelInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn compile_date(&self) -> &str {
        &self.compile_date
    }

    pub fn compile_time(&self) -> &str {
        &self.compile_time
    }

    pub fn uptime(&self) -> u64 {
        self.uptime
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.uptime() / 1000
    }

    pub fn uptime_minutes(&self) -> (u64, u8) {
        let seconds = self.uptime_seconds();
        (seconds / 60, (seconds % 60) as u8)
    }

    pub fn uptime_hours(&self) -> (u64, u8, u8) {
        let (minutes, seconds) = self.uptime_minutes();
        (minutes / 60, (minutes % 60) as u8, seconds)
    }

    pub fn fetch() -> io::Result<Self> {
        let file = File::open("proc:/kernelinfo")?;
        let reader = BufReader::new(file);
        Ok(serde_json::from_reader(reader)?)
    }
}
