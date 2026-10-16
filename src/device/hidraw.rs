// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal Linux hidraw access: finding a device's interfaces through sysfs, sending
//! feature and output reports, and waiting for input reports.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    /// The device node, like /dev/hidraw3.
    pub node: PathBuf,
    /// USB interface number.
    pub number: u8,
    pub descriptor: Vec<u8>,
    /// USB port path shared by all interfaces of one physical device.
    pub port: String,
}

/// Lists the hidraw interfaces of every connected USB device with this vendor and product id.
/// `sysfs` is normally /sys/class/hidraw and `dev` /dev; tests point them elsewhere.
pub fn find(sysfs: &Path, dev: &Path, vendor: u16, product: u16) -> Vec<Interface> {
    let wanted = format!("HID_ID=0003:{vendor:08X}:{product:08X}");
    let Ok(entries) = std::fs::read_dir(sysfs) else { return vec![] };
    let mut found: Vec<Interface> = entries
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let uevent = std::fs::read_to_string(dir.join("device/uevent")).ok()?;
            if !uevent.lines().any(|line| line.eq_ignore_ascii_case(&wanted)) {
                return None;
            }
            // HID_PHYS looks like usb-0000:0e:00.3-1/input1
            let phys = uevent.lines().find_map(|line| line.strip_prefix("HID_PHYS="))?;
            let (port, number) = phys.rsplit_once("/input")?;
            Some(Interface {
                node: dev.join(entry.file_name()),
                number: number.parse().ok()?,
                descriptor: std::fs::read(dir.join("device/report_descriptor")).ok()?,
                port: port.to_string(),
            })
        })
        .collect();
    found.sort_by(|a, b| (&a.port, a.number).cmp(&(&b.port, b.number)));
    found
}

/// `HIDIOCSFEATURE(len)` from linux/hidraw.h: `_IOC(_IOC_WRITE | _IOC_READ, 'H', 0x06, len)`.
fn hidiocsfeature(len: usize) -> libc::c_ulong {
    const READ_WRITE: libc::c_ulong = 3;
    (READ_WRITE << 30) | ((len as libc::c_ulong) << 16) | ((b'H' as libc::c_ulong) << 8) | 0x06
}

pub struct Handle {
    file: File,
}

impl Handle {
    pub fn open(node: &Path, write: bool) -> io::Result<Self> {
        Ok(Self { file: OpenOptions::new().read(true).write(write).open(node)? })
    }

    /// Sends a feature report. `report[0]` is the report id (0 for devices without ids).
    pub fn send_feature(&self, report: &[u8]) -> io::Result<()> {
        // SAFETY: the kernel reads exactly `report.len()` bytes from the buffer, which is that long.
        let result = unsafe { libc::ioctl(self.file.as_raw_fd(), hidiocsfeature(report.len()) as _, report.as_ptr()) };
        if result < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Sends an output report. `report[0]` is the report id.
    pub fn send_output(&mut self, report: &[u8]) -> io::Result<()> {
        self.file.write_all(report)
    }

    /// Reads one input report. Blocks until one arrives; use [`poll`] first.
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }
}

/// Waits up to `timeout` until any of `handles` has an input report. Returns which do.
pub fn poll(handles: &[&Handle], timeout: Duration) -> io::Result<Vec<bool>> {
    let mut fds: Vec<libc::pollfd> =
        handles.iter().map(|h| libc::pollfd { fd: h.file.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
    // SAFETY: `fds` holds `fds.len()` valid pollfd entries for the duration of the call.
    let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout.as_millis() as libc::c_int) };
    if ready < 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted { Ok(vec![false; fds.len()]) } else { Err(error) };
    }
    if fds.iter().any(|fd| fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0) {
        return Err(io::Error::new(io::ErrorKind::NotConnected, "the device went away"));
    }
    Ok(fds.iter().map(|fd| fd.revents & libc::POLLIN != 0).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_number_matches_linux() {
        // HIDIOCSFEATURE(643) as computed by the C macro on x86-64 and arm64.
        assert_eq!(hidiocsfeature(643), 0xC283_4806);
    }

    #[test]
    fn finds_interfaces_by_id() {
        let root = std::env::temp_dir().join(format!("open-steel-studio-sysfs-{}", std::process::id()));
        let add = |name: &str, id: &str, phys: &str, descriptor: &[u8]| {
            let dir = root.join(name).join("device");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("uevent"), format!("DRIVER=hid-generic\nHID_ID={id}\nHID_NAME=x\nHID_PHYS={phys}\n")).unwrap();
            std::fs::write(dir.join("report_descriptor"), descriptor).unwrap();
        };
        add("hidraw7", "0003:00001038:0000161C", "usb-0000:0e:00.3-1/input1", &[1, 2]);
        add("hidraw2", "0003:00001038:0000161C", "usb-0000:0e:00.3-1/input0", &[3]);
        add("hidraw4", "0003:00001038:000012E0", "usb-0000:0e:00.4-1/input3", &[4]); // a headset
        let found = find(&root, Path::new("/dev"), 0x1038, 0x161C);
        std::fs::remove_dir_all(&root).ok();

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].number, 0);
        assert_eq!(found[1].node, Path::new("/dev/hidraw7"));
        assert_eq!(found[1].descriptor, vec![1, 2]);
        assert_eq!(found[1].port, "usb-0000:0e:00.3-1");
    }
}
