# SMROS Networking

SMROS now has a user-level VirtIO-MMIO network driver for QEMU `virt`.

The default `make run`, `make debug`, and `make gdb` targets attach:

```text
-netdev user,id=smrosnet
-device virtio-net-device,netdev=smrosnet
```

The driver binds the QEMU VirtIO net device as `eth0`, reads the device MAC,
posts receive buffers, and exposes raw Ethernet send/receive through
`crate::user_level::drivers`.

The first network service layer now includes:

- static QEMU user-network defaults: `10.0.2.15`, gateway `10.0.2.2`, DNS `10.0.2.3`
- DHCP discover/request/ack for QEMU user networking
- ARP resolution
- IPv4 packet construction/parsing
- UDP and DNS A-record lookup
- ICMP echo request/reply
- a small TCP client path with SYN/SYN-ACK/ACK, payload send/read, and FIN close
- HTTP `GET` over plain TCP
- FTP control-channel banner read
- a NIC-backed user-level TCP socket facade in `src/user_level/services/net.rs`

Useful shell commands:

```text
drivers
ifconfig
dhcp
dns example.com
dns www.126.com
ping 10.0.2.2
ping github.com
ping https://github.com/opencontainers/image-spec/blob/main/manifest.md
curl http://example.com/
ftp speedtest.tele2.net
tls
```

Current limits:

- TCP is a minimal client implementation, not a complete congestion-control,
  retransmission, window-scaling, or server/listener stack.
- FTP support currently reads the control-channel banner; active/passive data
  transfers are not implemented.
- TLS is explicitly reported as unsupported. A real TLS implementation still
  needs cryptography, certificate validation, entropy, and a maintained TLS
  protocol state machine.
- `curl https://...` therefore prints a clear HTTPS/TLS limitation. Use
  `dns <host>`, `ping <host-or-url>`, or plain `http://` URLs until TLS exists.
- `dns <host>` is the resolver test. `ping <host>` resolves the host first and
  then sends ICMP; external ICMP can still time out under QEMU user networking
  even when DNS and TCP/HTTP are working. On Linux hosts, allow unprivileged
  ICMP echo sockets before starting QEMU if external ping is blocked:

  ```sh
  sudo sysctl -w net.ipv4.ping_group_range="0 2147483647"
  ```

- Linux socket syscalls are still modeled compatibility objects. The NIC-backed
  socket integration added here is the user-level `TcpSocket` facade used by
  `curl` and `ftp`; routing Linux `socket`/`connect`/`send`/`recv` syscalls
  through this stack is still separate kernel syscall work.
