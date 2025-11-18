use std::ffi::OsStr;
use std::io;
use std::mem::{size_of, MaybeUninit};
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;

use windows_sys::core::PCWSTR;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Module32FirstW, Process32FirstW, Process32NextW, MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverStatus {
    Success,
    DriverNotFound,
    FailedToAttach,
    ReadFailed,
    ReadPartial,
    WriteFailed,
    WritePartial,
    NotInitialized,
}

#[derive(thiserror::Error, Debug)]
pub enum DriverError {
    #[error("{0}")]
    Message(String),
    #[error("Win32 error: {0}")]
    Win32(#[from] io::Error),
    #[error("Driver status error: {0:?}")]
    Status(DriverStatus),
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
struct Info {
    target_pid: u64,
    target_address: u64,
    buffer_address: u64,
    bytes_to_read: u64,
    bytes_read: u64,
}

// CTL_CODE macro from wdm.h/winioctl.h:
// #define CTL_CODE(DeviceType, Function, Method, Access) \
//     (((DeviceType) << 16) | ((Access) << 14) | ((Function) << 2) | (Method))
// FILE_DEVICE_UNKNOWN = 0x22
// METHOD_BUFFERED = 0
// FILE_ANY_ACCESS = 0, FILE_SPECIAL_ACCESS is not standard (checking if it means FILE_ANY_ACCESS)
// Let me match the C++ exactly
const FILE_DEVICE_UNKNOWN: u32 = 0x00000022;
const METHOD_BUFFERED: u32 = 0;
const FILE_ANY_ACCESS: u32 = 0;

const INITCODE: u32 = (FILE_DEVICE_UNKNOWN << 16) | (FILE_ANY_ACCESS << 14) | (0x775 << 2) | METHOD_BUFFERED;
const READCODE: u32 = (FILE_DEVICE_UNKNOWN << 16) | (FILE_ANY_ACCESS << 14) | (0x776 << 2) | METHOD_BUFFERED;
const WRITECODE: u32 = (FILE_DEVICE_UNKNOWN << 16) | (FILE_ANY_ACCESS << 14) | (0x777 << 2) | METHOD_BUFFERED;
const GETSBADDR: u32 = (FILE_DEVICE_UNKNOWN << 16) | (FILE_ANY_ACCESS << 14) | (0x778 << 2) | METHOD_BUFFERED;

pub struct DriverManager {
    handle: HANDLE,
    target_pid: u32,
}

impl Drop for DriverManager {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.handle); }
    }
}

impl DriverManager {
    pub fn new(driver_name: &str) -> Result<Self, DriverError> {
        let path = format!("\\\\.\\{}", driver_name);
        tracing::debug!("Opening driver at path: {}", path);
        let wpath: Vec<u16> = OsStr::new(&path).encode_wide().chain(std::iter::once(0)).collect();
        unsafe {
            let handle = CreateFileW(
                wpath.as_ptr() as PCWSTR,
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                0 as HANDLE,
            );
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                let error_code = std::io::Error::last_os_error();
                tracing::error!("CreateFileW failed for {}: {:?}", path, error_code);
                return Err(DriverError::Status(DriverStatus::DriverNotFound));
            }
            tracing::debug!("Driver handle opened: {:?}", handle);
            Ok(Self { handle, target_pid: 0 })
        }
    }

    pub fn is_valid(&self) -> bool { !self.handle.is_null() && self.handle != INVALID_HANDLE_VALUE }
    pub fn target_pid(&self) -> u32 { self.target_pid }

    pub fn get_pid_by_name(proc_name: &str, use_um_module_enum: bool) -> Result<(u32, Option<usize>), DriverError> {
        tracing::debug!("Searching for process: {}", proc_name);
        let mut base_addr: Option<usize> = None;
        let proc_name_lower = proc_name.to_ascii_lowercase();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot.is_null() || snapshot == INVALID_HANDLE_VALUE {
                let error = std::io::Error::last_os_error();
                tracing::error!("CreateToolhelp32Snapshot failed: {:?}", error);
                return Err(DriverError::Message(format!("CreateToolhelp32Snapshot failed: {:?}", error)));
            }
            let mut pe32: PROCESSENTRY32W = std::mem::zeroed();
            pe32.dwSize = size_of::<PROCESSENTRY32W>() as u32;
            let mut process_count = 0;
            if Process32FirstW(snapshot, &mut pe32) != 0 {
                loop {
                    let exe = widestr_to_string(&pe32.szExeFile);
                    process_count += 1;
                    if exe.to_ascii_lowercase() == proc_name_lower {
                        let pid = pe32.th32ProcessID;
                        tracing::info!("Found process '{}' with PID: {}", exe, pid);
                        if use_um_module_enum {
                            tracing::debug!("Enumerating modules for PID {}", pid);
                            base_addr = Self::get_main_module_base(pid).ok();
                            if let Some(addr) = base_addr {
                                tracing::debug!("UM module base address: 0x{:X}", addr);
                            }
                        }
                        return Ok((pid, base_addr));
                    }
                    if Process32NextW(snapshot, &mut pe32) == 0 { break; }
                }
            }
            tracing::warn!("Process '{}' not found. Enumerated {} processes", proc_name, process_count);
        }
        Err(DriverError::Message(format!("Process '{}' not found", proc_name)))
    }

    fn get_main_module_base(pid: u32) -> Result<usize, DriverError> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
            if snapshot.is_null() || snapshot == INVALID_HANDLE_VALUE { return Err(DriverError::Message("CreateToolhelp32Snapshot (modules) failed".into())); }
            let mut me32: MODULEENTRY32W = std::mem::zeroed();
            me32.dwSize = size_of::<MODULEENTRY32W>() as u32;
            if Module32FirstW(snapshot, &mut me32) != 0 {
                return Ok(me32.modBaseAddr as usize);
            }
        }
        Err(DriverError::Message("Failed to enumerate modules".into()))
    }

    pub fn attach_to_process(&mut self, proc_name: &str, use_um_module_enum: bool) -> Result<Option<usize>, DriverError> {
        tracing::info!("Attaching to process: {}, use_um_module_enum: {}", proc_name, use_um_module_enum);
        let (pid, base_from_um) = Self::get_pid_by_name(proc_name, use_um_module_enum)?;
        self.target_pid = pid;
        tracing::debug!("Target PID set to: {}", pid);

        // Notify driver of target PID
        let mut info = Info { target_pid: pid as u64, ..Default::default() };
        let mut bytes_returned: u32 = 0;
        tracing::debug!("Sending INITCODE (0x{:08X}) to driver with PID: {}", INITCODE, pid);
        unsafe {
            let ok = DeviceIoControl(
                self.handle,
                INITCODE,
                &info as *const _ as *mut _,
                size_of::<Info>() as u32,
                &mut info as *mut _ as *mut _,
                size_of::<Info>() as u32,
                &mut bytes_returned,
                null_mut(),
            );
            if ok == 0 {
                let error = std::io::Error::last_os_error();
                tracing::error!("DeviceIoControl(INITCODE) failed: {:?}, bytes_returned: {}", error, bytes_returned);
                return Err(DriverError::Status(DriverStatus::FailedToAttach));
            }
            tracing::debug!("INITCODE succeeded, bytes_returned: {}", bytes_returned);
        }

        if use_um_module_enum {
            tracing::debug!("Using UM module enum, returning base: {:?}", base_from_um);
            return Ok(base_from_um);
        }

        // Ask driver for base address
        tracing::debug!("Requesting base address from driver via GETSBADDR");
        let mut kernel_base: u64 = 0;
        info.target_address = 0; // Driver will write base address to info.target_address
        info.buffer_address = 0;
        info.bytes_to_read = 0;
        info.bytes_read = 0;
        unsafe {
            let mut bytes_returned: u32 = 0;
            let ok = DeviceIoControl(
                self.handle,
                GETSBADDR,
                &info as *const _ as *mut _,
                size_of::<Info>() as u32,
                &mut info as *mut _ as *mut _,
                size_of::<Info>() as u32,
                &mut bytes_returned,
                null_mut(),
            );
            if ok == 0 {
                let error = std::io::Error::last_os_error();
                tracing::warn!("DeviceIoControl(GETSBADDR) failed: {:?}, bytes_returned: {}", error, bytes_returned);
                return Ok(None);
            }
            tracing::debug!("GETSBADDR succeeded: bytes_returned: {}, info.target_address: 0x{:X}", 
                bytes_returned, info.target_address);
            if info.target_address == 0 {
                tracing::warn!("Driver returned 0 for base address in target_address field");
                return Ok(None);
            }
            kernel_base = info.target_address;
        }
        tracing::info!("Driver returned base address: 0x{:X}", kernel_base);
        Ok(Some(kernel_base as usize))
    }

    pub fn read<T: Default + Copy, U: IntoAddress>(&self, address: U) -> Result<T, DriverError> {
        if !self.is_valid() {
            tracing::error!("Read called on invalid driver handle");
            return Err(DriverError::Status(DriverStatus::NotInitialized));
        }
        let addr = address.into_address();
        //tracing::debug!("Reading {} bytes from address 0x{:X} (PID: {})", size_of::<T>(), addr, self.target_pid);
        
        let mut buffer: MaybeUninit<T> = MaybeUninit::zeroed();
        let mut info = Info {
            target_pid: self.target_pid as u64,
            target_address: addr,
            buffer_address: buffer.as_mut_ptr() as u64,
            bytes_to_read: size_of::<T>() as u64,
            bytes_read: 0,
        };
        let mut bytes_returned: u32 = 0;
        
        //tracing::trace!("READCODE (0x{:08X}): target_pid={}, target_address=0x{:X}, buffer_address=0x{:X}, bytes_to_read={}", 
            //READCODE, info.target_pid, info.target_address, info.buffer_address, info.bytes_to_read);
        
        unsafe {
            let ok = DeviceIoControl(
                self.handle,
                READCODE,
                &info as *const _ as *mut _,
                size_of::<Info>() as u32,
                &mut info as *mut _ as *mut _,
                size_of::<Info>() as u32,
                &mut bytes_returned,
                null_mut(),
            );
            if ok == 0 {
                let error = std::io::Error::last_os_error();
                tracing::error!("DeviceIoControl(READCODE) failed: {:?}, bytes_returned: {}", error, bytes_returned);
                return Err(DriverError::Status(DriverStatus::ReadFailed));
            }
            //tracing::trace!("READCODE succeeded: bytes_returned={}, info.bytes_read={}", bytes_returned, info.bytes_read);
            
            if info.bytes_read != size_of::<T>() as u64 {
                tracing::error!("Partial read: expected {} bytes, got {} bytes", size_of::<T>(), info.bytes_read);
                return Err(DriverError::Status(DriverStatus::ReadPartial));
            }
            //tracing::debug!("Successfully read {} bytes from 0x{:X}", info.bytes_read, addr);
            Ok(buffer.assume_init())
        }
    }

    pub fn write<T: Copy, U: IntoAddress>(&self, address: U, data: &T) -> Result<(), DriverError> {
        if !self.is_valid() {
            return Err(DriverError::Status(DriverStatus::NotInitialized));
        }
        let mut info = Info {
            target_pid: self.target_pid as u64,
            target_address: address.into_address(),
            buffer_address: (data as *const T) as u64,
            bytes_to_read: size_of::<T>() as u64,
            bytes_read: 0,
        };
        let mut bytes_returned: u32 = 0;
        unsafe {
            let ok = DeviceIoControl(
                self.handle,
                WRITECODE,
                &info as *const _ as *mut _,
                size_of::<Info>() as u32,
                &mut info as *mut _ as *mut _,
                size_of::<Info>() as u32,
                &mut bytes_returned,
                null_mut(),
            );
            if ok == 0 {
                return Err(DriverError::Status(DriverStatus::WriteFailed));
            }
            if info.bytes_read != size_of::<T>() as u64 {
                return Err(DriverError::Status(DriverStatus::WritePartial));
            }
        }
        Ok(())
    }
}

pub trait IntoAddress {
    fn into_address(self) -> u64;
}
impl IntoAddress for u64 { fn into_address(self) -> u64 { self } }
impl IntoAddress for usize { fn into_address(self) -> u64 { self as u64 } }
impl<T> IntoAddress for *const T { fn into_address(self) -> u64 { self as u64 } }
impl<T> IntoAddress for *mut T { fn into_address(self) -> u64 { self as u64 } }

fn widestr_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}
