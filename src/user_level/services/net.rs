//! Minimal IPv4, UDP, ICMP, TCP, HTTP, and FTP networking over VirtIO-net.

#![allow(dead_code)]

use crate::user_level::drivers::{self, net::ETHERNET_FRAME_MAX, UserDriverError};

pub const QEMU_USER_IP: [u8; 4] = [10, 0, 2, 15];
pub const QEMU_USER_GATEWAY: [u8; 4] = [10, 0, 2, 2];
pub const QEMU_USER_DNS: [u8; 4] = [10, 0, 2, 3];
pub const DEFAULT_DNS_HOST: &str = "example.com";

const ETH_TYPE_IPV4: u16 = 0x0800;
const ETH_TYPE_ARP: u16 = 0x0806;
const ARP_HTYPE_ETHERNET: u16 = 1;
const ARP_PTYPE_IPV4: u16 = 0x0800;
const ARP_OPER_REQUEST: u16 = 1;
const ARP_OPER_REPLY: u16 = 2;
const IP_PROTO_ICMP: u8 = 1;
const IP_PROTO_TCP: u8 = 6;
const IP_PROTO_UDP: u8 = 17;
const DNS_PORT: u16 = 53;
const DNS_QUERY_ID_BASE: u16 = 0x534d;
const DNS_SOURCE_PORT_BASE: u16 = 49152;
const DNS_MAX_MESSAGE: usize = 512;
const DNS_QUERY_ATTEMPTS: usize = 3;
const DNS_RECV_ATTEMPTS: usize = 96;
const NET_POLL_SPINS: usize = 1_000_000;
const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_XID: u32 = 0x534d_4450;
const DHCP_MAGIC_COOKIE: u32 = 0x6382_5363;
const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;
const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;
const ICMP_IDENTIFIER: u16 = 0x534d;
const ICMP_SEQUENCE: u16 = 1;
const TCP_SOURCE_PORT_BASE: u16 = 41000;
const TCP_FLAG_FIN: u16 = 0x01;
const TCP_FLAG_SYN: u16 = 0x02;
const TCP_FLAG_RST: u16 = 0x04;
const TCP_FLAG_PSH: u16 = 0x08;
const TCP_FLAG_ACK: u16 = 0x10;
const TCP_WINDOW_SIZE: u16 = 4096;
const HTTP_PORT: u16 = 80;
const HTTPS_PORT: u16 = 443;
const FTP_PORT: u16 = 21;
pub const PING_TCP_FALLBACK_PORTS: [u16; 2] = [HTTP_PORT, HTTPS_PORT];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetError {
    Driver(UserDriverError),
    NotReady,
    InvalidHost,
    InvalidUrl,
    BufferTooSmall,
    MalformedPacket,
    Timeout,
    NoAddress,
    Unsupported,
    ConnectionReset,
    TlsUnsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetConfig {
    pub mac: [u8; 6],
    pub ip: [u8; 4],
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub mtu: usize,
    pub link_up: bool,
    pub dhcp_configured: bool,
    pub lease_seconds: u32,
}

#[derive(Clone, Copy)]
struct NetConfigState {
    ip: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
    dhcp_configured: bool,
    lease_seconds: u32,
}

impl NetConfigState {
    const fn new() -> Self {
        Self {
            ip: QEMU_USER_IP,
            gateway: QEMU_USER_GATEWAY,
            dns: QEMU_USER_DNS,
            dhcp_configured: false,
            lease_seconds: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PingReply {
    pub from: [u8; 4],
    pub bytes: usize,
    pub ttl: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcpProbeKind {
    Connected,
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TcpProbeReply {
    pub remote_ip: [u8; 4],
    pub port: u16,
    pub kind: TcpProbeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub remote_ip: [u8; 4],
    pub status_code: u16,
    pub bytes_read: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FtpResponse {
    pub remote_ip: [u8; 4],
    pub status_code: u16,
    pub bytes_read: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkSocketAddr {
    pub ip: [u8; 4],
    pub port: u16,
}

pub struct TcpSocket {
    local_port: u16,
    peer: NetworkSocketAddr,
    route_mac: [u8; 6],
    seq: u32,
    ack: u32,
    connected: bool,
}

static mut CONFIG_STATE: NetConfigState = NetConfigState::new();
static mut NEXT_DNS_QUERY_ID: u16 = DNS_QUERY_ID_BASE;
static mut NEXT_DNS_SOURCE_PORT: u16 = DNS_SOURCE_PORT_BASE;
static mut NEXT_TCP_PORT: u16 = TCP_SOURCE_PORT_BASE;

pub fn config() -> NetConfig {
    let state = config_state();
    NetConfig {
        mac: drivers::net_mac(),
        ip: state.ip,
        gateway: state.gateway,
        dns: state.dns,
        mtu: drivers::net::ETHERNET_MTU,
        link_up: drivers::net_link_up(),
        dhcp_configured: state.dhcp_configured,
        lease_seconds: state.lease_seconds,
    }
}

pub fn dhcp_configure() -> Result<NetConfig, NetError> {
    ensure_ready()?;
    let source_mac = drivers::net_mac();
    let mut frame = [0u8; ETHERNET_FRAME_MAX];
    let mut rx = [0u8; ETHERNET_FRAME_MAX];

    let discover_len = build_dhcp_discover(&mut frame, source_mac)?;
    drivers::net_send_frame(&frame[..discover_len]).map_err(NetError::Driver)?;

    let mut offered_ip = [0u8; 4];
    let mut server_ip = [0u8; 4];
    let mut router_ip = QEMU_USER_GATEWAY;
    let mut dns_ip = QEMU_USER_DNS;
    let mut lease_seconds = 0u32;

    for _ in 0..32 {
        match drivers::net_receive_frame_timeout(&mut rx, NET_POLL_SPINS) {
            Ok(len) => {
                if parse_dhcp_message(
                    &rx[..len],
                    DHCP_OFFER,
                    &mut offered_ip,
                    &mut server_ip,
                    &mut router_ip,
                    &mut dns_ip,
                    &mut lease_seconds,
                )? {
                    break;
                }
            }
            Err(UserDriverError::Timeout) => {}
            Err(err) => return Err(NetError::Driver(err)),
        }
    }

    if offered_ip == [0; 4] {
        return Err(NetError::Timeout);
    }

    let request_len = build_dhcp_request(&mut frame, source_mac, offered_ip, server_ip)?;
    drivers::net_send_frame(&frame[..request_len]).map_err(NetError::Driver)?;

    for _ in 0..32 {
        match drivers::net_receive_frame_timeout(&mut rx, NET_POLL_SPINS) {
            Ok(len) => {
                if parse_dhcp_message(
                    &rx[..len],
                    DHCP_ACK,
                    &mut offered_ip,
                    &mut server_ip,
                    &mut router_ip,
                    &mut dns_ip,
                    &mut lease_seconds,
                )? {
                    unsafe {
                        CONFIG_STATE.ip = offered_ip;
                        CONFIG_STATE.gateway = router_ip;
                        CONFIG_STATE.dns = dns_ip;
                        CONFIG_STATE.dhcp_configured = true;
                        CONFIG_STATE.lease_seconds = lease_seconds;
                    }
                    return Ok(config());
                }
            }
            Err(UserDriverError::Timeout) => {}
            Err(err) => return Err(NetError::Driver(err)),
        }
    }

    Err(NetError::Timeout)
}

pub fn dns_lookup_a(host: &str) -> Result<[u8; 4], NetError> {
    ensure_ready()?;
    validate_dns_host(host)?;

    let state = config_state();
    match dns_lookup_with_routes(host, state.dns, state.gateway) {
        Ok(ip) => Ok(ip),
        Err(NetError::Timeout) if state.dns != QEMU_USER_DNS => {
            dns_lookup_with_routes(host, QEMU_USER_DNS, state.gateway)
        }
        Err(err) => Err(err),
    }
}

pub fn ping(ip: [u8; 4]) -> Result<PingReply, NetError> {
    ensure_ready()?;
    let mut target_mac = [0u8; 6];
    let route_ip = route_ip_for(ip);
    resolve_mac(route_ip, &mut target_mac)?;
    send_icmp_echo_request(ip, target_mac)?;
    receive_icmp_echo_reply(ip)
}

pub fn tcp_probe(ip: [u8; 4], ports: &[u16]) -> Result<TcpProbeReply, NetError> {
    ensure_ready()?;
    let mut last_err = NetError::Timeout;

    for port in ports {
        let peer = NetworkSocketAddr { ip, port: *port };
        match tcp_connect(peer) {
            Ok(mut socket) => {
                let _ = socket.close();
                return Ok(TcpProbeReply {
                    remote_ip: ip,
                    port: *port,
                    kind: TcpProbeKind::Connected,
                });
            }
            Err(NetError::ConnectionReset) => {
                return Ok(TcpProbeReply {
                    remote_ip: ip,
                    port: *port,
                    kind: TcpProbeKind::Reset,
                });
            }
            Err(err) => last_err = err,
        }
    }

    Err(last_err)
}

pub fn http_get(host: &str, path: &str, out: &mut [u8]) -> Result<HttpResponse, NetError> {
    let remote_ip = dns_lookup_a(host)?;
    let mut socket = tcp_connect(NetworkSocketAddr {
        ip: remote_ip,
        port: HTTP_PORT,
    })?;
    let request_len = build_http_get_request(host, path, out)?;
    socket.write(&out[..request_len])?;
    let bytes_read = socket.read(out)?;
    let _ = socket.close();
    let status_code = parse_http_status(out, bytes_read).unwrap_or(0);
    Ok(HttpResponse {
        remote_ip,
        status_code,
        bytes_read,
    })
}

pub fn ftp_banner(host: &str, out: &mut [u8]) -> Result<FtpResponse, NetError> {
    let remote_ip = dns_lookup_a(host)?;
    let mut socket = tcp_connect(NetworkSocketAddr {
        ip: remote_ip,
        port: FTP_PORT,
    })?;
    let bytes_read = socket.read(out)?;
    let _ = socket.close();
    Ok(FtpResponse {
        remote_ip,
        status_code: parse_ftp_status(out, bytes_read).unwrap_or(0),
        bytes_read,
    })
}

pub fn tls_get(_host: &str, _path: &str, _out: &mut [u8]) -> Result<HttpResponse, NetError> {
    Err(NetError::TlsUnsupported)
}

pub fn smoke_test() -> Result<[u8; 4], NetError> {
    dns_lookup_a(DEFAULT_DNS_HOST)
}

fn resolve_mac(target_ip: [u8; 4], out: &mut [u8; 6]) -> Result<(), NetError> {
    let source_mac = drivers::net_mac();
    let mut frame = [0u8; ETHERNET_FRAME_MAX];
    let mut rx = [0u8; ETHERNET_FRAME_MAX];

    for _ in 0..3 {
        let len = build_arp_request(&mut frame, source_mac, source_ip(), target_ip)?;
        drivers::net_send_frame(&frame[..len]).map_err(NetError::Driver)?;

        for _ in 0..16 {
            match drivers::net_receive_frame_timeout(&mut rx, NET_POLL_SPINS) {
                Ok(len) => {
                    if parse_arp_reply(&rx[..len], target_ip, out) {
                        return Ok(());
                    }
                }
                Err(UserDriverError::Timeout) => break,
                Err(err) => return Err(NetError::Driver(err)),
            }
        }
    }

    Err(NetError::Timeout)
}

fn dns_lookup_with_routes(
    host: &str,
    dns_ip: [u8; 4],
    gateway_ip: [u8; 4],
) -> Result<[u8; 4], NetError> {
    let direct_route = route_ip_for(dns_ip);
    let primary_route = if dns_ip == QEMU_USER_DNS && gateway_ip != [0; 4] {
        gateway_ip
    } else {
        direct_route
    };
    match dns_lookup_with_route(host, dns_ip, primary_route) {
        Ok(ip) => Ok(ip),
        Err(NetError::Timeout)
            if direct_route != primary_route
                || (gateway_ip != [0; 4]
                    && gateway_ip != primary_route
                    && gateway_ip != dns_ip) =>
        {
            let fallback_route = if direct_route != primary_route {
                direct_route
            } else {
                gateway_ip
            };
            if fallback_route == primary_route || fallback_route == [0; 4] {
                return Err(NetError::Timeout);
            }
            dns_lookup_with_route(host, dns_ip, fallback_route)
        }
        Err(err) => Err(err),
    }
}

fn dns_lookup_with_route(
    host: &str,
    dns_ip: [u8; 4],
    route_ip: [u8; 4],
) -> Result<[u8; 4], NetError> {
    let mut route_mac = [0u8; 6];
    resolve_mac(route_ip, &mut route_mac)?;

    for _ in 0..DNS_QUERY_ATTEMPTS {
        let (query_id, source_port) = next_dns_query_token();
        send_dns_query(host, dns_ip, route_mac, query_id, source_port)?;
        match receive_dns_response(dns_ip, route_ip, query_id, source_port) {
            Ok(ip) => return Ok(ip),
            Err(NetError::Timeout) => {}
            Err(err) => return Err(err),
        }
    }

    Err(NetError::Timeout)
}

fn ensure_ready() -> Result<(), NetError> {
    if !drivers::init() || !drivers::net_ready() || !drivers::net_link_up() {
        Err(NetError::NotReady)
    } else {
        Ok(())
    }
}

fn config_state() -> NetConfigState {
    unsafe { CONFIG_STATE }
}

fn source_ip() -> [u8; 4] {
    config_state().ip
}

fn route_ip_for(target_ip: [u8; 4]) -> [u8; 4] {
    let state = config_state();
    if target_ip[0] == state.ip[0] && target_ip[1] == state.ip[1] && target_ip[2] == state.ip[2] {
        target_ip
    } else {
        state.gateway
    }
}

fn build_dhcp_discover(frame: &mut [u8], source_mac: [u8; 6]) -> Result<usize, NetError> {
    build_dhcp_frame(frame, source_mac, [0; 4], [0; 4], DHCP_DISCOVER)
}

fn build_dhcp_request(
    frame: &mut [u8],
    source_mac: [u8; 6],
    requested_ip: [u8; 4],
    server_ip: [u8; 4],
) -> Result<usize, NetError> {
    build_dhcp_frame(frame, source_mac, requested_ip, server_ip, DHCP_REQUEST)
}

fn build_dhcp_frame(
    frame: &mut [u8],
    source_mac: [u8; 6],
    requested_ip: [u8; 4],
    server_ip: [u8; 4],
    message_type: u8,
) -> Result<usize, NetError> {
    if frame.len() < 14 + 20 + 8 + 300 {
        return Err(NetError::BufferTooSmall);
    }
    frame.fill(0);
    frame[0..6].fill(0xff);
    frame[6..12].copy_from_slice(&source_mac);
    put_u16(frame, 12, ETH_TYPE_IPV4);

    let dhcp = 14 + 20 + 8;
    frame[dhcp] = 1;
    frame[dhcp + 1] = 1;
    frame[dhcp + 2] = 6;
    frame[dhcp + 3] = 0;
    put_u32(frame, dhcp + 4, DHCP_XID);
    put_u16(frame, dhcp + 10, 0x8000);
    frame[dhcp + 28..dhcp + 34].copy_from_slice(&source_mac);
    put_u32(frame, dhcp + 236, DHCP_MAGIC_COOKIE);

    let mut opt = dhcp + 240;
    frame[opt] = 53;
    frame[opt + 1] = 1;
    frame[opt + 2] = message_type;
    opt += 3;
    if requested_ip != [0; 4] {
        frame[opt] = 50;
        frame[opt + 1] = 4;
        frame[opt + 2..opt + 6].copy_from_slice(&requested_ip);
        opt += 6;
    }
    if server_ip != [0; 4] {
        frame[opt] = 54;
        frame[opt + 1] = 4;
        frame[opt + 2..opt + 6].copy_from_slice(&server_ip);
        opt += 6;
    }
    frame[opt] = 55;
    frame[opt + 1] = 3;
    frame[opt + 2] = 1;
    frame[opt + 3] = 3;
    frame[opt + 4] = 6;
    opt += 5;
    frame[opt] = 255;
    opt += 1;

    let udp_len = 8 + (opt - dhcp);
    let ip_len = 20 + udp_len;
    build_ipv4_header(
        frame,
        14,
        [0, 0, 0, 0],
        [255, 255, 255, 255],
        IP_PROTO_UDP,
        ip_len,
        DHCP_XID as u16,
    );
    let udp = 14 + 20;
    put_u16(frame, udp, DHCP_CLIENT_PORT);
    put_u16(frame, udp + 2, DHCP_SERVER_PORT);
    put_u16(frame, udp + 4, udp_len as u16);
    put_u16(frame, udp + 6, 0);
    Ok(14 + ip_len)
}

fn parse_dhcp_message(
    frame: &[u8],
    expected_type: u8,
    offered_ip: &mut [u8; 4],
    server_ip: &mut [u8; 4],
    router_ip: &mut [u8; 4],
    dns_ip: &mut [u8; 4],
    lease_seconds: &mut u32,
) -> Result<bool, NetError> {
    let Some((ip, ihl)) = ipv4_packet(frame, IP_PROTO_UDP) else {
        return Ok(false);
    };
    let udp = ip + ihl;
    if frame.len() < udp + 8 || get_u16(frame, udp) != DHCP_SERVER_PORT {
        return Ok(false);
    }
    let dhcp = udp + 8;
    if frame.len() < dhcp + 240
        || frame[dhcp] != 2
        || get_u32(frame, dhcp + 4) != DHCP_XID
        || get_u32(frame, dhcp + 236) != DHCP_MAGIC_COOKIE
    {
        return Ok(false);
    }
    offered_ip.copy_from_slice(&frame[dhcp + 16..dhcp + 20]);

    let mut message_type = 0u8;
    let mut offset = dhcp + 240;
    while offset < frame.len() {
        let code = frame[offset];
        offset += 1;
        if code == 255 {
            break;
        }
        if code == 0 {
            continue;
        }
        if offset >= frame.len() {
            return Err(NetError::MalformedPacket);
        }
        let len = frame[offset] as usize;
        offset += 1;
        if offset + len > frame.len() {
            return Err(NetError::MalformedPacket);
        }
        match code {
            53 if len >= 1 => message_type = frame[offset],
            54 if len >= 4 => server_ip.copy_from_slice(&frame[offset..offset + 4]),
            3 if len >= 4 => router_ip.copy_from_slice(&frame[offset..offset + 4]),
            6 if len >= 4 => dns_ip.copy_from_slice(&frame[offset..offset + 4]),
            51 if len >= 4 => *lease_seconds = get_u32(frame, offset),
            _ => {}
        }
        offset += len;
    }

    Ok(message_type == expected_type)
}

fn build_arp_request(
    frame: &mut [u8],
    source_mac: [u8; 6],
    source_ip: [u8; 4],
    target_ip: [u8; 4],
) -> Result<usize, NetError> {
    if frame.len() < 42 {
        return Err(NetError::BufferTooSmall);
    }

    frame[0..6].fill(0xff);
    frame[6..12].copy_from_slice(&source_mac);
    put_u16(frame, 12, ETH_TYPE_ARP);
    put_u16(frame, 14, ARP_HTYPE_ETHERNET);
    put_u16(frame, 16, ARP_PTYPE_IPV4);
    frame[18] = 6;
    frame[19] = 4;
    put_u16(frame, 20, ARP_OPER_REQUEST);
    frame[22..28].copy_from_slice(&source_mac);
    frame[28..32].copy_from_slice(&source_ip);
    frame[32..38].fill(0);
    frame[38..42].copy_from_slice(&target_ip);
    Ok(42)
}

fn parse_arp_reply(frame: &[u8], expected_ip: [u8; 4], out: &mut [u8; 6]) -> bool {
    if frame.len() < 42 || get_u16(frame, 12) != ETH_TYPE_ARP {
        return false;
    }
    if get_u16(frame, 14) != ARP_HTYPE_ETHERNET
        || get_u16(frame, 16) != ARP_PTYPE_IPV4
        || frame[18] != 6
        || frame[19] != 4
        || get_u16(frame, 20) != ARP_OPER_REPLY
    {
        return false;
    }
    if frame[28..32] != expected_ip || frame[38..42] != source_ip() {
        return false;
    }
    out.copy_from_slice(&frame[22..28]);
    true
}

fn send_dns_query(
    host: &str,
    dns_ip: [u8; 4],
    target_mac: [u8; 6],
    query_id: u16,
    source_port: u16,
) -> Result<(), NetError> {
    let source_mac = drivers::net_mac();
    let mut frame = [0u8; ETHERNET_FRAME_MAX];
    let dns_start = 14 + 20 + 8;
    let dns_len = build_dns_query(host, &mut frame[dns_start..], query_id)?;
    let udp_len = 8 + dns_len;
    let ip_len = 20 + udp_len;
    let frame_len = 14 + ip_len;

    frame[0..6].copy_from_slice(&target_mac);
    frame[6..12].copy_from_slice(&source_mac);
    put_u16(&mut frame, 12, ETH_TYPE_IPV4);

    let ip = 14;
    build_ipv4_header(
        &mut frame,
        ip,
        source_ip(),
        dns_ip,
        IP_PROTO_UDP,
        ip_len,
        query_id,
    );

    let udp = ip + 20;
    put_u16(&mut frame, udp, source_port);
    put_u16(&mut frame, udp + 2, DNS_PORT);
    put_u16(&mut frame, udp + 4, udp_len as u16);
    put_u16(&mut frame, udp + 6, 0);

    drivers::net_send_frame(&frame[..frame_len]).map_err(NetError::Driver)?;
    Ok(())
}

fn build_dns_query(host: &str, out: &mut [u8], query_id: u16) -> Result<usize, NetError> {
    if out.len() < DNS_MAX_MESSAGE {
        return Err(NetError::BufferTooSmall);
    }

    out[..DNS_MAX_MESSAGE].fill(0);
    put_u16(out, 0, query_id);
    put_u16(out, 2, 0x0100);
    put_u16(out, 4, 1);
    let mut offset = 12;
    offset = encode_dns_name(host, out, offset)?;
    put_u16(out, offset, 1);
    put_u16(out, offset + 2, 1);
    Ok(offset + 4)
}

fn encode_dns_name(host: &str, out: &mut [u8], mut offset: usize) -> Result<usize, NetError> {
    validate_dns_host(host)?;

    for label in host.as_bytes().split(|byte| *byte == b'.') {
        if offset + 1 + label.len() >= DNS_MAX_MESSAGE {
            return Err(NetError::BufferTooSmall);
        }
        out[offset] = label.len() as u8;
        offset += 1;
        for byte in label {
            out[offset] = *byte;
            offset += 1;
        }
    }

    if offset >= DNS_MAX_MESSAGE {
        return Err(NetError::InvalidHost);
    }
    out[offset] = 0;
    Ok(offset + 1)
}

fn validate_dns_host(host: &str) -> Result<(), NetError> {
    if host.is_empty() || host.len() > 253 {
        return Err(NetError::InvalidHost);
    }

    for label in host.as_bytes().split(|byte| *byte == b'.') {
        if label.is_empty() || label.len() > 63 {
            return Err(NetError::InvalidHost);
        }
        for byte in label {
            if !dns_label_byte_valid(*byte) {
                return Err(NetError::InvalidHost);
            }
        }
    }

    Ok(())
}

fn dns_label_byte_valid(byte: u8) -> bool {
    (byte >= b'a' && byte <= b'z')
        || (byte >= b'A' && byte <= b'Z')
        || (byte >= b'0' && byte <= b'9')
        || byte == b'-'
}

fn receive_dns_response(
    dns_ip: [u8; 4],
    route_ip: [u8; 4],
    query_id: u16,
    source_port: u16,
) -> Result<[u8; 4], NetError> {
    let mut rx = [0u8; ETHERNET_FRAME_MAX];

    for _ in 0..DNS_RECV_ATTEMPTS {
        match drivers::net_receive_frame_timeout(&mut rx, NET_POLL_SPINS) {
            Ok(len) => {
                if let Some(ip) =
                    parse_dns_response(&rx[..len], dns_ip, route_ip, query_id, source_port)?
                {
                    return Ok(ip);
                }
            }
            Err(UserDriverError::Timeout) => {}
            Err(err) => return Err(NetError::Driver(err)),
        }
    }

    Err(NetError::Timeout)
}

fn send_icmp_echo_request(target_ip: [u8; 4], route_mac: [u8; 6]) -> Result<(), NetError> {
    let mut frame = [0u8; ETHERNET_FRAME_MAX];
    let payload = b"smros-ping";
    let icmp = 14 + 20;
    let ip_len = 20 + 8 + payload.len();

    frame[0..6].copy_from_slice(&route_mac);
    frame[6..12].copy_from_slice(&drivers::net_mac());
    put_u16(&mut frame, 12, ETH_TYPE_IPV4);
    build_ipv4_header(
        &mut frame,
        14,
        source_ip(),
        target_ip,
        IP_PROTO_ICMP,
        ip_len,
        ICMP_IDENTIFIER,
    );
    frame[icmp] = ICMP_ECHO_REQUEST;
    frame[icmp + 1] = 0;
    put_u16(&mut frame, icmp + 2, 0);
    put_u16(&mut frame, icmp + 4, ICMP_IDENTIFIER);
    put_u16(&mut frame, icmp + 6, ICMP_SEQUENCE);
    frame[icmp + 8..icmp + 8 + payload.len()].copy_from_slice(payload);
    let checksum = internet_checksum(&frame[icmp..icmp + 8 + payload.len()]);
    put_u16(&mut frame, icmp + 2, checksum);
    drivers::net_send_frame(&frame[..14 + ip_len]).map_err(NetError::Driver)?;
    Ok(())
}

fn receive_icmp_echo_reply(expected_ip: [u8; 4]) -> Result<PingReply, NetError> {
    let mut rx = [0u8; ETHERNET_FRAME_MAX];

    for _ in 0..32 {
        match drivers::net_receive_frame_timeout(&mut rx, NET_POLL_SPINS) {
            Ok(len) => {
                let Some((ip, ihl)) = ipv4_packet(&rx[..len], IP_PROTO_ICMP) else {
                    continue;
                };
                if rx[ip + 12..ip + 16] != expected_ip || rx[ip + 16..ip + 20] != source_ip() {
                    continue;
                }
                let total_len = get_u16(&rx, ip + 2) as usize;
                let icmp = ip + ihl;
                if len < icmp + 8 || total_len < ihl + 8 {
                    return Err(NetError::MalformedPacket);
                }
                if rx[icmp] == ICMP_ECHO_REPLY
                    && get_u16(&rx, icmp + 4) == ICMP_IDENTIFIER
                    && get_u16(&rx, icmp + 6) == ICMP_SEQUENCE
                {
                    return Ok(PingReply {
                        from: expected_ip,
                        bytes: total_len - ihl,
                        ttl: rx[ip + 8],
                    });
                }
            }
            Err(UserDriverError::Timeout) => {}
            Err(err) => return Err(NetError::Driver(err)),
        }
    }

    Err(NetError::Timeout)
}

pub fn tcp_connect(peer: NetworkSocketAddr) -> Result<TcpSocket, NetError> {
    ensure_ready()?;
    let mut route_mac = [0u8; 6];
    resolve_mac(route_ip_for(peer.ip), &mut route_mac)?;
    let local_port = next_tcp_port();
    let seq = 0x534d_0000u32.wrapping_add(local_port as u32);
    send_tcp_segment(peer, route_mac, local_port, seq, 0, TCP_FLAG_SYN, &[])?;

    let mut rx = [0u8; ETHERNET_FRAME_MAX];
    for _ in 0..48 {
        match drivers::net_receive_frame_timeout(&mut rx, NET_POLL_SPINS) {
            Ok(len) => {
                if let Some(segment) = parse_tcp_segment(&rx[..len], peer.ip, peer.port, local_port)
                {
                    if segment.flags & TCP_FLAG_RST != 0 {
                        return Err(NetError::ConnectionReset);
                    }
                    if segment.flags & (TCP_FLAG_SYN | TCP_FLAG_ACK)
                        == (TCP_FLAG_SYN | TCP_FLAG_ACK)
                        && segment.ack == seq.wrapping_add(1)
                    {
                        let ack = segment.seq.wrapping_add(1);
                        send_tcp_segment(
                            peer,
                            route_mac,
                            local_port,
                            seq.wrapping_add(1),
                            ack,
                            TCP_FLAG_ACK,
                            &[],
                        )?;
                        return Ok(TcpSocket {
                            local_port,
                            peer,
                            route_mac,
                            seq: seq.wrapping_add(1),
                            ack,
                            connected: true,
                        });
                    }
                }
            }
            Err(UserDriverError::Timeout) => {}
            Err(err) => return Err(NetError::Driver(err)),
        }
    }

    Err(NetError::Timeout)
}

impl TcpSocket {
    pub fn write(&mut self, data: &[u8]) -> Result<usize, NetError> {
        if !self.connected {
            return Err(NetError::NotReady);
        }
        if data.len() > drivers::net::ETHERNET_MTU - 40 {
            return Err(NetError::BufferTooSmall);
        }
        send_tcp_segment(
            self.peer,
            self.route_mac,
            self.local_port,
            self.seq,
            self.ack,
            TCP_FLAG_PSH | TCP_FLAG_ACK,
            data,
        )?;
        self.seq = self.seq.wrapping_add(data.len() as u32);
        Ok(data.len())
    }

    pub fn read(&mut self, out: &mut [u8]) -> Result<usize, NetError> {
        if !self.connected {
            return Err(NetError::NotReady);
        }
        let mut rx = [0u8; ETHERNET_FRAME_MAX];
        let mut total = 0usize;

        for _ in 0..64 {
            match drivers::net_receive_frame_timeout(&mut rx, NET_POLL_SPINS) {
                Ok(len) => {
                    if let Some(segment) =
                        parse_tcp_segment(&rx[..len], self.peer.ip, self.peer.port, self.local_port)
                    {
                        if segment.flags & TCP_FLAG_RST != 0 {
                            self.connected = false;
                            return Err(NetError::ConnectionReset);
                        }
                        if !segment.payload.is_empty() && segment.seq == self.ack {
                            let copy_len = core::cmp::min(segment.payload.len(), out.len() - total);
                            out[total..total + copy_len]
                                .copy_from_slice(&segment.payload[..copy_len]);
                            total += copy_len;
                            self.ack = self.ack.wrapping_add(segment.payload.len() as u32);
                            send_tcp_segment(
                                self.peer,
                                self.route_mac,
                                self.local_port,
                                self.seq,
                                self.ack,
                                TCP_FLAG_ACK,
                                &[],
                            )?;
                            if total == out.len() {
                                return Ok(total);
                            }
                        }
                        if segment.flags & TCP_FLAG_FIN != 0 {
                            self.ack = self.ack.wrapping_add(1);
                            self.connected = false;
                            let _ = send_tcp_segment(
                                self.peer,
                                self.route_mac,
                                self.local_port,
                                self.seq,
                                self.ack,
                                TCP_FLAG_ACK,
                                &[],
                            );
                            return Ok(total);
                        }
                    }
                }
                Err(UserDriverError::Timeout) => {
                    if total > 0 {
                        return Ok(total);
                    }
                }
                Err(err) => return Err(NetError::Driver(err)),
            }
        }

        if total > 0 {
            Ok(total)
        } else {
            Err(NetError::Timeout)
        }
    }

    pub fn close(&mut self) -> Result<(), NetError> {
        if !self.connected {
            return Ok(());
        }
        send_tcp_segment(
            self.peer,
            self.route_mac,
            self.local_port,
            self.seq,
            self.ack,
            TCP_FLAG_FIN | TCP_FLAG_ACK,
            &[],
        )?;
        self.seq = self.seq.wrapping_add(1);
        self.connected = false;
        Ok(())
    }
}

fn parse_dns_response(
    frame: &[u8],
    dns_ip: [u8; 4],
    route_ip: [u8; 4],
    query_id: u16,
    source_port: u16,
) -> Result<Option<[u8; 4]>, NetError> {
    if frame.len() < 14 + 20 + 8 + 12 || get_u16(frame, 12) != ETH_TYPE_IPV4 {
        return Ok(None);
    }

    let ip = 14;
    let ihl = ((frame[ip] & 0x0f) as usize) * 4;
    if ihl < 20 || frame.len() < ip + ihl + 8 + 12 || frame[ip + 9] != IP_PROTO_UDP {
        return Ok(None);
    }
    let response_source_matches = frame[ip + 12..ip + 16] == dns_ip
        || (route_ip != dns_ip && frame[ip + 12..ip + 16] == route_ip);
    if !response_source_matches || frame[ip + 16..ip + 20] != source_ip() {
        return Ok(None);
    }

    let udp = ip + ihl;
    if get_u16(frame, udp) != DNS_PORT || get_u16(frame, udp + 2) != source_port {
        return Ok(None);
    }

    let udp_len = get_u16(frame, udp + 4) as usize;
    if udp_len < 8 || frame.len() < udp + udp_len {
        return Err(NetError::MalformedPacket);
    }

    let dns = &frame[udp + 8..udp + udp_len];
    if dns.len() < 12 || get_u16(dns, 0) != query_id {
        return Ok(None);
    }
    let flags = get_u16(dns, 2);
    if flags & 0x8000 == 0 {
        return Ok(None);
    }
    if flags & 0x000f != 0 {
        return Err(NetError::NoAddress);
    }

    let questions = get_u16(dns, 4) as usize;
    let answers = get_u16(dns, 6) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_dns_name(dns, offset).ok_or(NetError::MalformedPacket)?;
        if offset + 4 > dns.len() {
            return Err(NetError::MalformedPacket);
        }
        offset += 4;
    }

    for _ in 0..answers {
        offset = skip_dns_name(dns, offset).ok_or(NetError::MalformedPacket)?;
        if offset + 10 > dns.len() {
            return Err(NetError::MalformedPacket);
        }
        let record_type = get_u16(dns, offset);
        let record_class = get_u16(dns, offset + 2);
        let rdlen = get_u16(dns, offset + 8) as usize;
        offset += 10;
        if offset + rdlen > dns.len() {
            return Err(NetError::MalformedPacket);
        }
        if record_type == 1 && record_class == 1 && rdlen == 4 {
            return Ok(Some([
                dns[offset],
                dns[offset + 1],
                dns[offset + 2],
                dns[offset + 3],
            ]));
        }
        offset += rdlen;
    }

    Err(NetError::NoAddress)
}

#[derive(Clone, Copy)]
struct TcpSegment<'a> {
    seq: u32,
    ack: u32,
    flags: u16,
    payload: &'a [u8],
}

fn next_tcp_port() -> u16 {
    unsafe {
        let port = NEXT_TCP_PORT;
        NEXT_TCP_PORT = if NEXT_TCP_PORT >= TCP_SOURCE_PORT_BASE + 1024 {
            TCP_SOURCE_PORT_BASE
        } else {
            NEXT_TCP_PORT + 1
        };
        port
    }
}

fn next_dns_query_token() -> (u16, u16) {
    unsafe {
        let query_id = NEXT_DNS_QUERY_ID;
        let source_port = NEXT_DNS_SOURCE_PORT;
        NEXT_DNS_QUERY_ID = if NEXT_DNS_QUERY_ID == u16::MAX {
            DNS_QUERY_ID_BASE
        } else {
            NEXT_DNS_QUERY_ID + 1
        };
        NEXT_DNS_SOURCE_PORT = if NEXT_DNS_SOURCE_PORT >= DNS_SOURCE_PORT_BASE + 1024 {
            DNS_SOURCE_PORT_BASE
        } else {
            NEXT_DNS_SOURCE_PORT + 1
        };
        (query_id, source_port)
    }
}

fn send_tcp_segment(
    peer: NetworkSocketAddr,
    route_mac: [u8; 6],
    local_port: u16,
    seq: u32,
    ack: u32,
    flags: u16,
    payload: &[u8],
) -> Result<(), NetError> {
    let tcp_header_len = 20usize;
    let tcp_len = tcp_header_len + payload.len();
    let ip_len = 20 + tcp_len;
    if 14 + ip_len > ETHERNET_FRAME_MAX {
        return Err(NetError::BufferTooSmall);
    }

    let mut frame = [0u8; ETHERNET_FRAME_MAX];
    frame[0..6].copy_from_slice(&route_mac);
    frame[6..12].copy_from_slice(&drivers::net_mac());
    put_u16(&mut frame, 12, ETH_TYPE_IPV4);
    build_ipv4_header(
        &mut frame,
        14,
        source_ip(),
        peer.ip,
        IP_PROTO_TCP,
        ip_len,
        local_port,
    );

    let tcp = 14 + 20;
    put_u16(&mut frame, tcp, local_port);
    put_u16(&mut frame, tcp + 2, peer.port);
    put_u32(&mut frame, tcp + 4, seq);
    put_u32(&mut frame, tcp + 8, ack);
    frame[tcp + 12] = (tcp_header_len as u8 / 4) << 4;
    frame[tcp + 13] = flags as u8;
    put_u16(&mut frame, tcp + 14, TCP_WINDOW_SIZE);
    put_u16(&mut frame, tcp + 16, 0);
    put_u16(&mut frame, tcp + 18, 0);
    frame[tcp + tcp_header_len..tcp + tcp_len].copy_from_slice(payload);
    let checksum = tcp_checksum(source_ip(), peer.ip, &frame[tcp..tcp + tcp_len]);
    put_u16(&mut frame, tcp + 16, checksum);

    drivers::net_send_frame(&frame[..14 + ip_len]).map_err(NetError::Driver)?;
    Ok(())
}

fn parse_tcp_segment<'a>(
    frame: &'a [u8],
    expected_ip: [u8; 4],
    expected_source_port: u16,
    expected_dest_port: u16,
) -> Option<TcpSegment<'a>> {
    let (ip, ihl) = ipv4_packet(frame, IP_PROTO_TCP)?;
    if frame[ip + 12..ip + 16] != expected_ip || frame[ip + 16..ip + 20] != source_ip() {
        return None;
    }
    let total_len = get_u16(frame, ip + 2) as usize;
    let tcp = ip + ihl;
    if total_len < ihl + 20 || frame.len() < tcp + 20 {
        return None;
    }
    if get_u16(frame, tcp) != expected_source_port || get_u16(frame, tcp + 2) != expected_dest_port
    {
        return None;
    }
    let data_offset = ((frame[tcp + 12] >> 4) as usize) * 4;
    if data_offset < 20 || total_len < ihl + data_offset {
        return None;
    }
    let payload_start = tcp + data_offset;
    let payload_end = ip + total_len;
    if payload_end > frame.len() || payload_start > payload_end {
        return None;
    }
    Some(TcpSegment {
        seq: get_u32(frame, tcp + 4),
        ack: get_u32(frame, tcp + 8),
        flags: (frame[tcp + 13] as u16) | (((frame[tcp + 12] & 0x01) as u16) << 8),
        payload: &frame[payload_start..payload_end],
    })
}

fn build_http_get_request(host: &str, path: &str, out: &mut [u8]) -> Result<usize, NetError> {
    if host.is_empty() || !path.starts_with('/') {
        return Err(NetError::InvalidUrl);
    }
    let parts = [
        "GET ",
        path,
        " HTTP/1.0\r\nHost: ",
        host,
        "\r\nUser-Agent: SMROS/0.1\r\nConnection: close\r\n\r\n",
    ];
    let mut offset = 0usize;
    for part in parts {
        let bytes = part.as_bytes();
        if offset + bytes.len() > out.len() {
            return Err(NetError::BufferTooSmall);
        }
        out[offset..offset + bytes.len()].copy_from_slice(bytes);
        offset += bytes.len();
    }
    Ok(offset)
}

fn parse_http_status(buf: &[u8], len: usize) -> Option<u16> {
    if len < 12 || &buf[0..5] != b"HTTP/" {
        return None;
    }
    let mut offset = 5usize;
    while offset < len && buf[offset] != b' ' {
        offset += 1;
    }
    if offset + 4 > len {
        return None;
    }
    parse_three_digits(&buf[offset + 1..offset + 4])
}

fn parse_ftp_status(buf: &[u8], len: usize) -> Option<u16> {
    if len < 3 {
        return None;
    }
    parse_three_digits(&buf[0..3])
}

fn parse_three_digits(bytes: &[u8]) -> Option<u16> {
    if bytes.len() != 3
        || !bytes[0].is_ascii_digit()
        || !bytes[1].is_ascii_digit()
        || !bytes[2].is_ascii_digit()
    {
        return None;
    }
    Some(
        ((bytes[0] - b'0') as u16) * 100
            + ((bytes[1] - b'0') as u16) * 10
            + (bytes[2] - b'0') as u16,
    )
}

fn ipv4_packet(frame: &[u8], protocol: u8) -> Option<(usize, usize)> {
    if frame.len() < 14 + 20 || get_u16(frame, 12) != ETH_TYPE_IPV4 {
        return None;
    }
    let ip = 14usize;
    let ihl = ((frame[ip] & 0x0f) as usize) * 4;
    let total_len = get_u16(frame, ip + 2) as usize;
    if frame[ip] >> 4 != 4
        || ihl < 20
        || frame.len() < ip + ihl
        || total_len < ihl
        || frame.len() < ip + total_len
        || frame[ip + 9] != protocol
    {
        return None;
    }
    Some((ip, ihl))
}

fn build_ipv4_header(
    frame: &mut [u8],
    ip: usize,
    source: [u8; 4],
    dest: [u8; 4],
    protocol: u8,
    total_len: usize,
    ident: u16,
) {
    frame[ip] = 0x45;
    frame[ip + 1] = 0;
    put_u16(frame, ip + 2, total_len as u16);
    put_u16(frame, ip + 4, ident);
    put_u16(frame, ip + 6, 0x4000);
    frame[ip + 8] = 64;
    frame[ip + 9] = protocol;
    put_u16(frame, ip + 10, 0);
    frame[ip + 12..ip + 16].copy_from_slice(&source);
    frame[ip + 16..ip + 20].copy_from_slice(&dest);
    let checksum = internet_checksum(&frame[ip..ip + 20]);
    put_u16(frame, ip + 10, checksum);
}

fn tcp_checksum(source: [u8; 4], dest: [u8; 4], tcp: &[u8]) -> u16 {
    let mut sum = 0u32;
    sum = checksum_add(sum, ((source[0] as u16) << 8) | source[1] as u16);
    sum = checksum_add(sum, ((source[2] as u16) << 8) | source[3] as u16);
    sum = checksum_add(sum, ((dest[0] as u16) << 8) | dest[1] as u16);
    sum = checksum_add(sum, ((dest[2] as u16) << 8) | dest[3] as u16);
    sum = checksum_add(sum, IP_PROTO_TCP as u16);
    sum = checksum_add(sum, tcp.len() as u16);
    checksum_finish(checksum_bytes(sum, tcp))
}

fn skip_dns_name(packet: &[u8], mut offset: usize) -> Option<usize> {
    for _ in 0..128 {
        if offset >= packet.len() {
            return None;
        }
        let len = packet[offset];
        if len & 0xc0 == 0xc0 {
            return if offset + 2 <= packet.len() {
                Some(offset + 2)
            } else {
                None
            };
        }
        if len == 0 {
            return Some(offset + 1);
        }
        let label_len = len as usize;
        if len & 0xc0 != 0 || offset + 1 + label_len > packet.len() {
            return None;
        }
        offset += 1 + label_len;
    }
    None
}

fn internet_checksum(bytes: &[u8]) -> u16 {
    checksum_finish(checksum_bytes(0, bytes))
}

fn checksum_bytes(mut sum: u32, bytes: &[u8]) -> u32 {
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        sum = checksum_add(sum, get_u16(bytes, index));
        index += 2;
    }
    if index < bytes.len() {
        sum = checksum_add(sum, (bytes[index] as u16) << 8);
    }
    sum
}

fn checksum_add(sum: u32, value: u16) -> u32 {
    sum.wrapping_add(value as u32)
}

fn checksum_finish(mut sum: u32) -> u16 {
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn get_u16(buf: &[u8], offset: usize) -> u16 {
    ((buf[offset] as u16) << 8) | buf[offset + 1] as u16
}

fn get_u32(buf: &[u8], offset: usize) -> u32 {
    ((buf[offset] as u32) << 24)
        | ((buf[offset + 1] as u32) << 16)
        | ((buf[offset + 2] as u32) << 8)
        | buf[offset + 3] as u32
}

fn put_u16(buf: &mut [u8], offset: usize, value: u16) {
    buf[offset] = (value >> 8) as u8;
    buf[offset + 1] = value as u8;
}

fn put_u32(buf: &mut [u8], offset: usize, value: u32) {
    buf[offset] = (value >> 24) as u8;
    buf[offset + 1] = (value >> 16) as u8;
    buf[offset + 2] = (value >> 8) as u8;
    buf[offset + 3] = value as u8;
}
