# WebNetSim — Agent & Project Architecture Context

## 1. Project Overview & Vision
**WebNetSim** is a lightweight, high-performance, in-browser interactive Computer Network Simulator and Packet Flow Visualizer designed as a modern, zero-install alternative to Cisco Packet Tracer for computer science students, teachers, and networking engineers.

It runs **100% client-side in the browser** (static HTML/CSS/JS + WebAssembly if needed) with zero backend server dependencies, allowing it to be deployed directly on GitHub Pages, Cloudflare Pages, or Vercel.

---

## 2. Target Audience & Core Value Proposition
- **CS / IT University Students**: Complete networking lab assignments (OSI model, Subnetting, ARP, Ping, Routing, DHCP, DNS).
- **Teachers & Professors**: Live interactive classroom demonstrations without debugging 50+ desktop installations.
- **CCNA / Network+ Aspirants**: Visualizing how headers (Layer 2 Ethernet frames, Layer 3 IP, Layer 4 TCP/UDP) mutate across hops.
- **Key Advantage over Cisco Packet Tracer**: Instant URL loading (<1s), no account login required, runs on any device (Chromebooks, iPads, Macs, Linux, Windows), open source, and visual step-by-step packet inspection.

---

## 3. Technology Stack & Design Decisions
- **Core Frontend**: Modern Vanilla ES Modules (`index.html`, `main.js`, Vanilla CSS with dark mode + glassmorphism design tokens).
- **Topology Canvas Engine**: HTML5 Canvas / SVG rendering with 60 FPS smooth dragging, interactive bezier link wiring, and animated packet travel.
- **Simulation Engine**: Discrete Event Simulation (DES) queue running in pure JS / WASM:
  - Supports **Real-Time Mode** (packets flow continuously with realistic latency) and **Step/Simulation Mode** (play, pause, step-forward, inspect packet headers at each hop).
- **State Serialization**: Save & load network topologies as standard JSON files (`.netsim.json`) or export/share via URL base64 hashes.

---

## 4. Network Model & Protocol Stack Implementation

### A. Supported Device Nodes
1. **Host / PC (`HostNode`)**:
   - Configuration: IPv4 Address, Subnet Mask, Default Gateway, DNS Server, MAC Address.
   - Built-in Terminal CLI: `ping`, `traceroute`, `arp -a`, `ipconfig` / `ifconfig`, `curl` / `fetch`.
2. **Switch (`SwitchNode` - Layer 2)**:
   - Dynamic MAC Address Table (CAM Table) with aging timers.
   - Flooding unknown unicast / broadcast packets (e.g. ARP requests).
   - Port forwarding based on learned MAC addresses.
3. **Router (`RouterNode` - Layer 3)**:
   - Multiple interfaces (e.g., `eth0: 192.168.1.1/24`, `eth1: 10.0.0.1/24`).
   - Routing Table lookup with longest prefix match.
   - TTL decrement & ICMP Time Exceeded generation.
   - Layer 2 MAC header rewriting on each hop.
4. **Server (`ServerNode`)**:
   - HTTP Web Server (serves a dummy HTML page).
   - DNS Server (translates domain names to IP addresses).
   - DHCP Server (leases dynamic IPs to connected PCs).
5. **Hub (`HubNode` - Layer 1)**:
   - Simple physical layer repeater (broadcasts every incoming frame to all other ports) for educational collision demonstrations.

### B. Supported Protocols & Frame Architecture
Each packet in flight contains structured layers that can be inspected:
- **Layer 2 (Ethernet Frame)**: Preamble, Destination MAC, Source MAC, EtherType (`0x0800` IPv4, `0x0806` ARP), CRC.
- **Layer 3 (Network)**:
  - **IPv4**: Source IP, Destination IP, TTL, Protocol ID (1=ICMP, 6=TCP, 17=UDP), Checksum.
  - **ARP**: Opcode (Request / Reply), Sender MAC/IP, Target MAC/IP.
  - **ICMP**: Type 8 (Echo Request), Type 0 (Echo Reply), Type 11 (Time Exceeded), Type 3 (Destination Unreachable).
- **Layer 4 (Transport)**:
  - **UDP**: Source Port, Destination Port, Length.
  - **TCP**: SYN, SYN-ACK, ACK handshake state machine.
- **Layer 7 (Application)**:
  - **DNS**: Query name, Answer IP.
  - **DHCP**: Discover, Offer, Request, ACK (DORA flow).
  - **HTTP**: `GET /index.html`, `HTTP/1.1 200 OK`.

---

## 5. User Interface & Interactive Features

### 1. Canvas Workbench
- **Palette Toolbar**: Click-and-drop devices onto the canvas.
- **Cable Tool**: Connect ports between devices (Copper Straight-Through, Cross-Over, Serial).
- **Port Status Indicators**: Green (Link Up), Red (Link Down), Amber (STP Learning / Blocking).

### 2. Device Configuration Modals & Embedded Terminal
- Clicking any device opens its tabbed interface:
  - **Desktop / Terminal**: Interactive command-line prompt (`C:\> ping 192.168.2.1`).
  - **Config Tab**: GUI fields for IP, Subnet, Gateway, DNS, Port Speed.
  - **Tables Tab**: Live view of ARP Table, Routing Table, MAC Address Table.

### 3. Simulation & Packet Inspector Drawer
- **Step-by-Step Execution**: Pause time, click "Step Next", and watch packets advance 1 hop.
- **Interactive Packet Click**: Clicking any flying packet sprite opens a detailed **Wireshark-style Layer Breakdown** showing exactly what each OSI layer is doing and why the device chose its forwarding decision.

### 4. Interactive Pre-Built Labs & Guided Tutorials
- **Lab 1**: Two PCs & a Switch (Understanding ARP and Broadcast).
- **Lab 2**: Connecting Two Subnets via a Router (Default Gateways & IP Routing).
- **Lab 3**: Setting up a DHCP & DNS Server for Automatic Configuration.
- **Lab 4**: TCP 3-Way Handshake & Web Server access.

---

## 6. Project Directory Structure
```text
WebNetSim/
├── index.html                 # Main Single Page Application shell
├── css/
│   ├── main.css               # Design system, tokens, typography & layout
│   ├── canvas.css             # Workbench canvas & device node styling
│   ├── modal.css              # Terminal CLI & device config modals
│   └── inspector.css          # Packet inspector & simulation drawer
├── js/
│   ├── app.js                 # App initialization & global state manager
│   ├── canvas/
│   │   ├── viewport.js        # Zoom, pan, and grid rendering
│   │   ├── renderer.js        # 60fps canvas drawing loop
│   │   └── wire_manager.js    # Cable connections & port link states
│   ├── models/
│   │   ├── device.js          # Base Device class
│   │   ├── host.js            # PC / Laptop model
│   │   ├── switch.js          # L2 Switch + MAC table model
│   │   ├── router.js          # L3 Router + Routing table model
│   │   └── server.js          # DHCP, DNS, HTTP Server model
│   ├── protocols/
│   │   ├── packet.js          # Generic Packet & Layer data structures
│   │   ├── arp.js             # ARP protocol handler
│   │   ├── ip.js              # IPv4 forwarding & validation
│   │   ├── icmp.js            # Ping & Traceroute logic
│   │   ├── dhcp.js            # DHCP DORA state machine
│   │   └── tcp.js             # TCP 3-way handshake simulation
│   ├── sim/
│   │   ├── engine.js          # Discrete Event Simulator queue
│   │   └── animator.js        # Packet travel animations along wire paths
│   ├── ui/
│   │   ├── terminal.js        # Interactive CLI terminal component
│   │   ├── inspector_ui.js    # Wireshark-style header inspector
│   │   ├── lab_manager.js     # Pre-built tutorials & validation checker
│   │   └── storage.js         # JSON save/load & URL hash sharing
│   └── presets/
│       ├── basic_lan.json     # Pre-made lab presets
│       ├── routed_network.json
│       └── client_server.json
├── assets/
│   └── icons/                 # SVG icons for router, switch, pc, server
├── README.md
└── LICENSE                    # MIT License
```

---

## 7. Development & Implementation Roadmap

### Phase 1: Canvas & Basic L2/L3 Simulation (MVP)
- [ ] Implement responsive HTML5 Canvas with grid, zoom, pan, and device drag-and-drop.
- [ ] Connect devices with cable lines and establish link-state bindings.
- [ ] Implement `HostNode`, `SwitchNode`, and `RouterNode` data models.
- [ ] Implement ARP request/reply and ICMP Echo (Ping) packet exchange.
- [ ] Animated packet movement along cables with Real-Time and Step Mode.

### Phase 2: Terminal CLI & Packet Inspector
- [ ] Device Terminal CLI supporting `ping`, `ipconfig`, `arp -a`, `route print`.
- [ ] Wireshark-style Packet Inspector drawer displaying Layer 2 & Layer 3 headers on click.
- [ ] MAC Table and Routing Table live inspection tabs.

### Phase 3: L7 Services & Interactive Labs
- [ ] DHCP Server & auto-configuration client logic.
- [ ] DNS Server & Web Browser / HTTP client (`curl` or simulated mini-browser).
- [ ] 5 interactive pre-built labs with automatic challenge pass/fail verification.
- [ ] JSON export/import & URL shareable permalinks.
