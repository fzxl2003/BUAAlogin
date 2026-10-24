use crate::{cancellation::Cancellation, interfaces::Interface, transport::bound_socket};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(super) struct Proxy {
    pub address: SocketAddr,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Proxy {
    pub fn start(
        interface: Interface,
        gateway: Ipv4Addr,
        cancel: Cancellation,
    ) -> Result<Self, String> {
        let listener =
            TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| "无法启动网页代理")?;
        let address = listener.local_addr().map_err(|_| "无法读取网页代理地址")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "无法配置网页代理")?;
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let worker = thread::spawn(move || {
            let mut workers: Vec<JoinHandle<()>> = vec![];
            while !stop.load(Ordering::SeqCst) && cancel.check().is_ok() {
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].is_finished() {
                        let _ = workers.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
                match listener.accept() {
                    Ok((client, _)) if workers.len() < 16 => {
                        let interface = interface.clone();
                        let cancel = cancel.clone();
                        let stop = stop.clone();
                        workers.push(thread::spawn(move || {
                            let _ = tunnel(client, &interface, gateway, &cancel, &stop);
                        }));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
            stop.store(true, Ordering::SeqCst);
            for worker in workers {
                let _ = worker.join();
            }
        });
        Ok(Self {
            address,
            stopped,
            worker: Some(worker),
        })
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn allowed(request: &str) -> bool {
    let mut parts = request.lines().next().unwrap_or("").split_whitespace();
    parts.next() == Some("CONNECT")
        && parts.next() == Some("gw.buaa.edu.cn:443")
        && parts.next() == Some("HTTP/1.1")
        && parts.next().is_none()
}
fn tunnel(
    mut client: TcpStream,
    interface: &Interface,
    gateway: Ipv4Addr,
    cancel: &Cancellation,
    stop: &AtomicBool,
) -> Result<(), String> {
    client
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| "代理读取配置失败")?;
    client
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|_| "代理写入配置失败")?;
    let mut header = vec![];
    let header_deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !header.ends_with(b"\r\n\r\n") {
        cancel.check()?;
        if stop.load(Ordering::SeqCst)
            || header.len() >= 8192
            || std::time::Instant::now() >= header_deadline
        {
            return Err("代理请求已终止".into());
        }
        let mut byte = [0];
        match client.read(&mut byte) {
            Ok(0) => return Ok(()),
            Ok(_) => header.push(byte[0]),
            Err(error) if temporary(&error) => continue,
            Err(_) => return Err("代理读取失败".into()),
        }
    }
    if !allowed(std::str::from_utf8(&header).unwrap_or("")) {
        let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return Err("网页请求地址不被允许".into());
    }
    cancel.check()?;
    if stop.load(Ordering::SeqCst) {
        return Ok(());
    }
    let socket = bound_socket(
        socket2::Domain::IPV4.into(),
        socket2::Type::STREAM.into(),
        socket2::Protocol::TCP.into(),
        interface.index,
    )?;
    socket
        .bind(&SocketAddr::from((interface.address, 0)).into())
        .map_err(|_| "网页代理源 IP 绑定失败")?;
    socket
        .connect_timeout(
            &SocketAddr::from((gateway, 443)).into(),
            Duration::from_secs(3),
        )
        .map_err(|_| "网页代理无法连接网关")?;
    cancel.check()?;
    if stop.load(Ordering::SeqCst) {
        return Ok(());
    }
    let mut upstream: TcpStream = socket.into();
    client
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .map_err(|_| "代理应答失败")?;
    client.set_nonblocking(true).map_err(|_| "代理配置失败")?;
    upstream.set_nonblocking(true).map_err(|_| "代理配置失败")?;
    let mut outgoing = Vec::new();
    let mut incoming = Vec::new();
    while cancel.check().is_ok() && !stop.load(Ordering::SeqCst) {
        if !pump(&mut client, &mut upstream, &mut outgoing)?
            || !pump(&mut upstream, &mut client, &mut incoming)?
        {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
fn temporary(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
    )
}
fn pump(
    source: &mut TcpStream,
    destination: &mut TcpStream,
    pending: &mut Vec<u8>,
) -> Result<bool, String> {
    if pending.is_empty() {
        let mut bytes = [0; 16384];
        match source.read(&mut bytes) {
            Ok(0) => return Ok(false),
            Ok(count) => pending.extend_from_slice(&bytes[..count]),
            Err(error) if temporary(&error) => {}
            Err(_) => return Ok(false),
        }
    }
    if !pending.is_empty() {
        match destination.write(pending) {
            Ok(0) => return Ok(false),
            Ok(count) => {
                pending.drain(..count);
            }
            Err(error) if temporary(&error) => {}
            Err(_) => return Ok(false),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_gateway_tunnels_are_accepted() {
        assert!(allowed("CONNECT gw.buaa.edu.cn:443 HTTP/1.1\r\n\r\n"));
        for request in [
            "GET https://gw.buaa.edu.cn/ HTTP/1.1",
            "CONNECT other.example:443 HTTP/1.1",
            "CONNECT gw.buaa.edu.cn:80 HTTP/1.1",
            "CONNECT user@gw.buaa.edu.cn:443 HTTP/1.1",
            "CONNECT gw.buaa.edu.cn:443 HTTP/1.1 extra",
        ] {
            assert!(!allowed(request));
        }
    }
    #[test]
    fn rejected_requests_and_idle_clients_are_cancelled() {
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let interface = Interface {
            name: "fixture".into(),
            address: "127.0.0.1".parse().unwrap(),
            index: 1,
        };
        let proxy = Proxy::start(
            interface,
            "127.0.0.1".parse().unwrap(),
            Cancellation::new(generation.clone(), 0),
        )
        .unwrap();
        let mut client = TcpStream::connect(proxy.address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .write_all(b"CONNECT other.example:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut response = [0; 256];
        let length = client.read(&mut response).unwrap();
        assert!(std::str::from_utf8(&response[..length])
            .unwrap()
            .starts_with("HTTP/1.1 403"));
        let _idle = TcpStream::connect(proxy.address).unwrap();
        generation.fetch_add(1, Ordering::SeqCst);
        let start = std::time::Instant::now();
        drop(proxy);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
