//! Native byte transport only. Framing, commands and application policy live in flows.
use hmi_core::{CoreError, CoreResult, TransportConfig};
use serde::Serialize;
use std::time::Instant;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub device_key: String,
    pub path: String,
    pub location: String,
}

pub trait DeviceIo: Send {
    fn write(&mut self, bytes: &[u8], deadline: Instant) -> CoreResult<()>;
    fn read(&mut self, deadline: Instant) -> CoreResult<Vec<u8>>;
}

pub fn device_key(path: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("usb_{:x}", Sha256::digest(path.to_uppercase().as_bytes()))
}

#[cfg(not(windows))]
pub fn enumerate(_: &TransportConfig) -> CoreResult<Vec<DeviceInfo>> {
    Err(CoreError::new(
        "TRANSPORT_PLATFORM",
        "USB Printer transport requires Windows",
    ))
}
#[cfg(not(windows))]
pub fn open(_: &str, _: u32) -> CoreResult<Box<dyn DeviceIo>> {
    Err(CoreError::new(
        "TRANSPORT_PLATFORM",
        "USB Printer transport requires Windows",
    ))
}

#[cfg(windows)]
pub use native::{enumerate, open};

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        mem::size_of,
        ptr::{null, null_mut},
    };
    use windows_sys::{
        core::GUID,
        Win32::{
            Devices::DeviceAndDriverInstallation::*,
            Foundation::*,
            Storage::FileSystem::*,
            System::{Threading::*, IO::*},
        },
    };

    struct Handle(HANDLE);
    unsafe impl Send for Handle {}
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct DeviceSet(HDEVINFO);
    impl Drop for DeviceSet {
        fn drop(&mut self) {
            unsafe {
                SetupDiDestroyDeviceInfoList(self.0);
            }
        }
    }
    fn failure(operation: &str) -> CoreError {
        CoreError::new(
            "TRANSPORT_IO",
            format!("{operation}: {}", std::io::Error::last_os_error()),
        )
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    pub fn enumerate(config: &TransportConfig) -> CoreResult<Vec<DeviceInfo>> {
        let guid = uuid::Uuid::parse_str(config.interface_guid.trim_matches(['{', '}']))
            .map_err(|error| CoreError::new("TRANSPORT_CONFIG", error.to_string()))?;
        let (data1, data2, data3, data4) = guid.as_fields();
        let guid = GUID {
            data1,
            data2,
            data3,
            data4: *data4,
        };
        let raw = unsafe {
            SetupDiGetClassDevsW(
                &guid,
                null(),
                null_mut(),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
        };
        if raw == INVALID_HANDLE_VALUE as isize {
            return Err(failure("SetupDiGetClassDevs"));
        }
        let set = DeviceSet(raw);
        let mut output = Vec::new();
        for index in 0..4096 {
            let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
            interface.cbSize = size_of::<SP_DEVICE_INTERFACE_DATA>() as u32;
            if unsafe { SetupDiEnumDeviceInterfaces(set.0, null(), &guid, index, &mut interface) }
                == 0
            {
                if unsafe { GetLastError() } == ERROR_NO_MORE_ITEMS {
                    break;
                }
                return Err(failure("SetupDiEnumDeviceInterfaces"));
            }
            let mut required = 0;
            unsafe {
                SetupDiGetDeviceInterfaceDetailW(
                    set.0,
                    &interface,
                    null_mut(),
                    0,
                    &mut required,
                    null_mut(),
                );
            }
            if !(8..=65536).contains(&required) {
                return Err(CoreError::new(
                    "TRANSPORT_IO",
                    "Invalid USB interface detail length",
                ));
            }
            // u64 storage preserves native structure alignment; DevicePath starts at byte 4.
            let mut detail = vec![0u64; (required as usize).div_ceil(8)];
            let pointer = detail
                .as_mut_ptr()
                .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
            let mut devinfo: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
            devinfo.cbSize = size_of::<SP_DEVINFO_DATA>() as u32;
            unsafe {
                (*pointer).cbSize = if cfg!(target_pointer_width = "64") {
                    8
                } else {
                    6
                };
            }
            if unsafe {
                SetupDiGetDeviceInterfaceDetailW(
                    set.0,
                    &interface,
                    pointer,
                    required,
                    &mut required,
                    &mut devinfo,
                )
            } == 0
            {
                return Err(failure("SetupDiGetDeviceInterfaceDetail"));
            }
            let path_units = unsafe {
                std::slice::from_raw_parts(
                    detail.as_ptr().cast::<u8>().add(4).cast::<u16>(),
                    (required as usize - 4) / 2,
                )
            };
            let length = path_units
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(path_units.len());
            let path = String::from_utf16_lossy(&path_units[..length]);
            let upper = path.to_uppercase();
            if !upper.contains(&format!("VID_{:04X}", config.vendor_id))
                || !upper.contains(&format!("PID_{:04X}", config.product_id))
            {
                continue;
            }
            let mut location = [0u16; 512];
            unsafe {
                SetupDiGetDeviceRegistryPropertyW(
                    set.0,
                    &devinfo,
                    SPDRP_LOCATION_INFORMATION,
                    null_mut(),
                    location.as_mut_ptr().cast(),
                    (location.len() * 2) as u32,
                    null_mut(),
                );
            }
            let length = location
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(location.len());
            output.push(DeviceInfo {
                device_key: device_key(&path),
                path,
                location: String::from_utf16_lossy(&location[..length]),
            });
        }
        output.sort_by(|a, b| a.device_key.cmp(&b.device_key));
        Ok(output)
    }

    struct UsbPrinter {
        reader: Handle,
        writer: Handle,
        packet_size: usize,
    }
    pub fn open(path: &str, packet_size: u32) -> CoreResult<Box<dyn DeviceIo>> {
        if packet_size == 0 || packet_size > 65536 {
            return Err(CoreError::new("TRANSPORT_CONFIG", "Invalid packet size"));
        }
        let name = wide(path);
        let create = |access| {
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    access,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    null(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED | FILE_FLAG_WRITE_THROUGH,
                    null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                Err(failure("CreateFile USB"))
            } else {
                Ok(Handle(handle))
            }
        };
        Ok(Box::new(UsbPrinter {
            reader: create(GENERIC_READ)?,
            writer: create(GENERIC_WRITE)?,
            packet_size: packet_size as usize,
        }))
    }

    fn transfer(
        handle: HANDLE,
        buffer: *mut c_void,
        count: u32,
        read: bool,
        deadline: Instant,
    ) -> CoreResult<usize> {
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(failure("CreateEvent"));
        }
        let event = Handle(event);
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
        overlapped.hEvent = event.0;
        let mut transferred = 0;
        let ready = unsafe {
            if read {
                ReadFile(
                    handle,
                    buffer.cast(),
                    count,
                    &mut transferred,
                    &mut overlapped,
                )
            } else {
                WriteFile(
                    handle,
                    buffer.cast(),
                    count,
                    &mut transferred,
                    &mut overlapped,
                )
            }
        };
        if ready == 0 {
            if unsafe { GetLastError() } != ERROR_IO_PENDING {
                return Err(failure(if read { "ReadFile" } else { "WriteFile" }));
            }
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(u32::MAX as u128 - 1) as u32;
            if unsafe { WaitForSingleObject(event.0, remaining) } != WAIT_OBJECT_0 {
                // The OVERLAPPED and buffer must remain alive until the driver completes cancellation.
                unsafe {
                    CancelIoEx(handle, &overlapped);
                    GetOverlappedResult(handle, &overlapped, &mut transferred, 1);
                }
                return Err(CoreError::new(
                    "TRANSPORT_TIMEOUT",
                    "USB transaction timed out; request was not retried",
                ));
            }
            if unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, 0) } == 0 {
                return Err(failure("GetOverlappedResult"));
            }
        }
        Ok(transferred as usize)
    }
    impl DeviceIo for UsbPrinter {
        fn write(&mut self, bytes: &[u8], deadline: Instant) -> CoreResult<()> {
            for packet in bytes.chunks(self.packet_size) {
                if Instant::now() >= deadline {
                    return Err(CoreError::new(
                        "TRANSPORT_TIMEOUT",
                        "USB write deadline reached",
                    ));
                }
                let count = transfer(
                    self.writer.0,
                    packet.as_ptr().cast_mut().cast(),
                    packet.len() as u32,
                    false,
                    deadline,
                )?;
                if count != packet.len() {
                    return Err(CoreError::new(
                        "TRANSPORT_WRITE",
                        "Partial USB write; request was not retried",
                    ));
                }
            }
            if !bytes.is_empty() && bytes.len() % self.packet_size == 0 {
                transfer(self.writer.0, null_mut(), 0, false, deadline)?;
            }
            Ok(())
        }
        fn read(&mut self, deadline: Instant) -> CoreResult<Vec<u8>> {
            let mut buffer = vec![0; 4096];
            let count = transfer(
                self.reader.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                true,
                deadline,
            )?;
            if count == 0 {
                return Err(CoreError::new(
                    "TRANSPORT_DISCONNECTED",
                    "USB device returned end of stream",
                ));
            }
            buffer.truncate(count);
            Ok(buffer)
        }
    }
}
