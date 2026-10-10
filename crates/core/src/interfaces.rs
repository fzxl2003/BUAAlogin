use serde::Serialize;
use std::net::Ipv4Addr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct Interface {
    pub name: String,
    pub address: Ipv4Addr,
    pub index: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct InterfaceRecord {
    #[serde(flatten)]
    pub interface: Interface,
    pub binding: String,
    pub label: String,
    pub campus: bool,
    pub message: String,
    pub campus_ip: Option<String>,
}
impl Interface {
    pub fn selected(&self, selection: &Option<String>) -> bool {
        selection
            .as_ref()
            .is_none_or(|s| s == &self.name || s == &self.address.to_string())
    }
    pub fn eligible(&self, virtuals: bool) -> bool {
        let lower = self.name.to_lowercase();
        !self.address.is_loopback()
            && !self.address.is_unspecified()
            && (virtuals
                || (!self.address.is_link_local()
                    && !["utun", "tun", "tap", "wg", "awdl", "llw"]
                        .iter()
                        .any(|p| lower.starts_with(p))))
    }
    pub fn binding(&self) -> String {
        if cfg!(windows) {
            self.address.to_string()
        } else {
            self.name.clone()
        }
    }
}
pub fn discover(virtuals: bool) -> Result<Vec<Interface>, String> {
    let mut records = native_discover()?;
    records.retain(|i| i.eligible(virtuals));
    records.sort_by_key(|i| (i.address.octets()[0] != 10, i.name.clone(), i.address));
    records.dedup();
    Ok(records)
}
#[cfg(unix)]
fn native_discover() -> Result<Vec<Interface>, String> {
    unsafe {
        let mut head = std::ptr::null_mut();
        if libc::getifaddrs(&mut head) != 0 {
            return Err("无法读取网络接口".into());
        }
        struct List(*mut libc::ifaddrs);
        impl Drop for List {
            fn drop(&mut self) {
                unsafe { libc::freeifaddrs(self.0) }
            }
        }
        let _list = List(head);
        let mut records = vec![];
        let mut cursor = head;
        while !cursor.is_null() {
            let item = &*cursor;
            if !item.ifa_addr.is_null()
                && (*item.ifa_addr).sa_family as i32 == libc::AF_INET
                && item.ifa_flags as i32 & libc::IFF_UP != 0
                && item.ifa_flags as i32 & libc::IFF_RUNNING != 0
                && item.ifa_flags as i32 & libc::IFF_LOOPBACK == 0
            {
                let addr = &*(item.ifa_addr as *const libc::sockaddr_in);
                records.push(Interface {
                    name: std::ffi::CStr::from_ptr(item.ifa_name)
                        .to_string_lossy()
                        .into_owned(),
                    address: Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
                    index: libc::if_nametoindex(item.ifa_name),
                });
            }
            cursor = item.ifa_next;
        }
        Ok(records)
    }
}
#[cfg(windows)]
fn native_discover() -> Result<Vec<Interface>, String> {
    use windows_sys::Win32::{
        NetworkManagement::{IpHelper::*, Ndis::IfOperStatusUp},
        Networking::WinSock::*,
    };
    unsafe {
        let mut size = 16_384u32;
        loop {
            // usize storage ensures IP_ADAPTER_ADDRESSES alignment.
            let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
            let head = buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH;
            let result = GetAdaptersAddresses(
                AF_INET as u32,
                GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                std::ptr::null(),
                head,
                &mut size,
            );
            if result == 111 {
                continue;
            }
            if result != 0 {
                return Err("无法读取网络接口".into());
            }
            let mut records = vec![];
            let mut current = head;
            while !current.is_null() {
                let adapter = &*current;
                if adapter.OperStatus == IfOperStatusUp && adapter.IfType != 24 {
                    let mut len = 0;
                    while *adapter.FriendlyName.add(len) != 0 {
                        len += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(
                        adapter.FriendlyName,
                        len,
                    ));
                    let mut unicast = adapter.FirstUnicastAddress;
                    while !unicast.is_null() {
                        let address = (*unicast).Address.lpSockaddr;
                        if !address.is_null()
                            && (*address).sa_family == AF_INET
                            && (*unicast).DadState == IpDadStatePreferred
                        {
                            let v4 = &*(address as *const SOCKADDR_IN);
                            records.push(Interface {
                                name: name.clone(),
                                address: Ipv4Addr::from(v4.sin_addr.S_un.S_addr.to_ne_bytes()),
                                index: adapter.Anonymous1.Anonymous.IfIndex,
                            });
                        }
                        unicast = (*unicast).Next;
                    }
                }
                current = adapter.Next;
            }
            return Ok(records);
        }
    }
}

// macOS feeds SystemConfiguration notifications through the C ABI.
#[cfg(target_os = "macos")]
pub fn watch(_: crate::engine::Signal) -> Option<()> {
    Some(())
}

#[cfg(target_os = "linux")]
pub struct Watcher {
    stop: std::os::fd::OwnedFd,
    thread: Option<std::thread::JoinHandle<()>>,
}
#[cfg(target_os = "linux")]
impl Drop for Watcher {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        let value = 1u64;
        unsafe {
            libc::write(self.stop.as_raw_fd(), &value as *const _ as *const _, 8);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
#[cfg(target_os = "linux")]
pub fn watch(signal: crate::engine::Signal) -> Option<Watcher> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    unsafe {
        let fd = libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        );
        if fd < 0 {
            return None;
        }
        let socket = OwnedFd::from_raw_fd(fd);
        let mut address: libc::sockaddr_nl = std::mem::zeroed();
        address.nl_family = libc::AF_NETLINK as u16;
        address.nl_groups = 1 | 0x10 | 0x40; // LINK, IPv4 ADDRESS, IPv4 ROUTE
        if libc::bind(
            fd,
            &address as *const _ as *const _,
            std::mem::size_of_val(&address) as _,
        ) != 0
        {
            return None;
        }
        let stop_fd = libc::eventfd(0, libc::EFD_CLOEXEC);
        if stop_fd < 0 {
            return None;
        }
        let stop = OwnedFd::from_raw_fd(stop_fd);
        let read_fd = libc::dup(stop_fd);
        if read_fd < 0 {
            return None;
        }
        let read = OwnedFd::from_raw_fd(read_fd);
        let thread = std::thread::spawn(move || {
            let mut fds = [
                libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: read.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            loop {
                if libc::poll(fds.as_mut_ptr(), 2, -1) < 0 {
                    continue;
                }
                if fds[1].revents != 0 {
                    break;
                }
                if fds[0].revents & libc::POLLIN != 0 {
                    let mut buffer = [0u8; 8192];
                    let received = libc::recv(
                        socket.as_raw_fd(),
                        buffer.as_mut_ptr() as _,
                        buffer.len(),
                        0,
                    );
                    if received <= 0 {
                        break;
                    }
                    let mut offset = 0;
                    let mut names = vec![];
                    let mut global = false;
                    while offset + 16 <= received as usize {
                        let len = u32::from_ne_bytes(buffer[offset..offset + 4].try_into().unwrap())
                            as usize;
                        if len < 16 || offset + len > received as usize {
                            break;
                        }
                        let kind =
                            u16::from_ne_bytes(buffer[offset + 4..offset + 6].try_into().unwrap());
                        if [16, 17, 20, 21].contains(&kind) && len >= 24 {
                            let index = u32::from_ne_bytes(
                                buffer[offset + 20..offset + 24].try_into().unwrap(),
                            );
                            names.push(format!("#{index}"));
                        } else if kind == 28 || kind == 29 {
                            global = true;
                        }
                        offset += (len + 3) & !3;
                    }
                    if global {
                        signal.network_changed();
                    } else if !names.is_empty() {
                        signal.network_changed_for(names);
                    }
                }
            }
        });
        Some(Watcher {
            stop,
            thread: Some(thread),
        })
    }
}
#[cfg(windows)]
pub struct Watcher {
    handle: windows_sys::Win32::Foundation::HANDLE,
    address_handle: windows_sys::Win32::Foundation::HANDLE,
    context: *mut crate::engine::Signal,
}
#[cfg(windows)]
impl Drop for Watcher {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::NetworkManagement::IpHelper::CancelMibChangeNotify2(self.handle);
            windows_sys::Win32::NetworkManagement::IpHelper::CancelMibChangeNotify2(
                self.address_handle,
            );
            drop(Box::from_raw(self.context));
        }
    }
}
#[cfg(windows)]
pub fn watch(signal: crate::engine::Signal) -> Option<Watcher> {
    use windows_sys::Win32::{NetworkManagement::IpHelper::*, Networking::WinSock::AF_UNSPEC};
    unsafe extern "system" fn changed(
        context: *const std::ffi::c_void,
        row: *const MIB_IPINTERFACE_ROW,
        _: MIB_NOTIFICATION_TYPE,
    ) {
        if let Some(signal) = (context as *const crate::engine::Signal).as_ref() {
            if let Some(row) = row.as_ref() {
                signal.network_changed_for(vec![format!("#{}", row.InterfaceIndex)]);
            } else {
                signal.network_changed();
            }
        }
    }
    unsafe extern "system" fn address_changed(
        context: *const std::ffi::c_void,
        row: *const MIB_UNICASTIPADDRESS_ROW,
        _: MIB_NOTIFICATION_TYPE,
    ) {
        if let Some(signal) = (context as *const crate::engine::Signal).as_ref() {
            if let Some(row) = row.as_ref() {
                signal.network_changed_for(vec![format!("#{}", row.InterfaceIndex)]);
            } else {
                signal.network_changed();
            }
        }
    }
    unsafe {
        let context = Box::into_raw(Box::new(signal));
        let mut handle = std::ptr::null_mut();
        if NotifyIpInterfaceChange(AF_UNSPEC, Some(changed), context as _, false, &mut handle) != 0
        {
            drop(Box::from_raw(context));
            return None;
        }
        let mut address_handle = std::ptr::null_mut();
        if NotifyUnicastIpAddressChange(
            AF_UNSPEC,
            Some(address_changed),
            context as _,
            false,
            &mut address_handle,
        ) != 0
        {
            CancelMibChangeNotify2(handle);
            drop(Box::from_raw(context));
            return None;
        }
        Some(Watcher {
            handle,
            address_handle,
            context,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_selection_and_filter() {
        let i = Interface {
            name: "en0".into(),
            address: "10.1.2.3".parse().unwrap(),
            index: 2,
        };
        assert!(i.selected(&Some("en0".into())));
        assert!(!i.selected(&Some("en1".into())));
        let vpn = Interface {
            name: "utun0".into(),
            ..i
        };
        assert!(!vpn.eligible(false));
        assert!(vpn.eligible(true));
    }
}
