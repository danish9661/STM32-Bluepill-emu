use crate::native;
use crate::system::{System, VmEvent, get_uart_output};
use std::sync::Mutex;

/// STM32 system-memory bootloader responder (AN3155 USART protocol shape)
/// for the F103 (USART1, no wire needed).
///
/// Real hardware enters this ROM when BOOT0=1/BOOT1=0 at reset; here it is
/// enabled explicitly (`bootloader_enable`) and then claims USART1 RX,
/// answering the host flashing flow headlessly: autobaud, GET / version /
/// ID (PID 0x0410), READ / GO / WRITE / ERASE with AN3155 framing and
/// checksums. Flash and RAM operations land in the real loaded image
/// (ROM privilege, like silicon), so a full erase-write-verify cycle is
/// exercisable in CI with no hardware and no debugger.
///
/// Supported command set: GET(0x00) GET_VERSION(0x01) GET_ID(0x02)
/// READ(0x11) GO(0x21) WRITE(0x31) ERASE(0x43). Anything else NACKs.
/// Multi-byte frames carry an XOR checksum (address words, length byte,
/// data); a bad checksum NACKs and the session stays in command state.
/// Lengths cap at 256 bytes per transfer. GO records its address
/// (`bootloader_go_addr`) but does not retarget the CPU — the host jumps
/// itself, e.g. via `rustcpu_set_pc`.
/// Erase understands page lists (1 KB F1 pages) and mass erase; both fill
/// 0xFF for real. WRP is not enforced (ROM-privilege model, documented).
pub const USART_BASE: u32 = 0x4001_3800;
pub const ACK: u8 = 0x79;
pub const NACK: u8 = 0x1F;
pub const BOOT_VERSION: u8 = 0x22;
pub const PID_HI: u8 = 0x04;
pub const PID_LO: u8 = 0x10;
pub const MAX_XFER: usize = 256;
pub const PAGE_SIZE: usize = 1024;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    /// Waiting for the 0x7F autobaud byte.
    Autobaud,
    /// Waiting for [cmd, ~cmd].
    Command,
    /// Collecting a multi-byte frame for `then`. Fixed-size ops use
    /// `need` directly; variable ops (erase/write-data) derive the total
    /// from the count byte once it arrives (total = count + 3).
    Collect { need: usize, then: Op },
}

#[derive(Clone, Copy, PartialEq)]
enum Op {
    ReadAddr,
    ReadLen,
    GoAddr,
    WriteAddr,
    WriteData,
    EraseList,
}

struct State {
    enabled: bool,
    phase: Phase,
    buf: Vec<u8>,
    /// Address stashed by *_Addr frames for the following data frame.
    stash: [u8; 4],
    go_addr: Option<u32>,
}

impl Default for State {
    fn default() -> Self {
        Self { enabled: false, phase: Phase::Autobaud, buf: Vec::new(), stash: [0; 4], go_addr: None }
    }
}

static STATE: Mutex<State> = Mutex::new(State {
    enabled: false,
    phase: Phase::Autobaud,
    buf: Vec::new(),
    stash: [0; 4],
    go_addr: None,
});

/// Enable/disable the bootloader responder (claims USART1 RX while on).
pub fn set_enabled(on: bool) {
    let mut st = STATE.lock().unwrap();
    st.enabled = on;
    st.phase = Phase::Autobaud;
    st.buf.clear();
    if !on {
        st.go_addr = None;
    }
}

pub fn is_enabled() -> bool {
    STATE.lock().unwrap().enabled
}

/// Last GO target address, if the host issued GO since enable.
pub fn go_addr() -> Option<u32> {
    STATE.lock().unwrap().go_addr
}

/// Reset session state (called from init paths alongside the other
/// peripheral resets so a re-init never inherits a session).
pub fn reset() {
    let mut st = STATE.lock().unwrap();
    *st = State::default();
}

fn tx(sys: &System, bytes: &[u8]) {
    if let Ok(mut out) = get_uart_output().lock() {
        for &b in bytes {
            out.push(b as char);
        }
    }
    for &b in bytes {
        sys.push_event(VmEvent::UartTx { usart: 1, byte: b });
    }
}

fn ack(sys: &System) {
    tx(sys, &[ACK]);
}

fn nack(sys: &System) {
    tx(sys, &[NACK]);
}

fn mem_ranges() -> Option<((u32, u32), (u32, u32))> {
    native::mem_ranges()
}

/// Address range valid for READ/WRITE/GO/ERASE (flash image or RAM).
fn valid_span(addr: u32, len: usize) -> bool {
    let end = match addr.checked_add(len as u32) {
        Some(e) => e,
        None => return false,
    };
    match mem_ranges() {
        Some(((fb, fl), (rb, rl))) => {
            (addr >= fb && end <= fb + fl) || (addr >= rb && end <= rb + rl)
        }
        None => false,
    }
}

fn xor_all(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |a, &b| a ^ b)
}

fn be_u32(b: &[u8]) -> u32 {
    ((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32
}

/// Feed one USART1 RX byte into the responder. Always consumes the byte
/// (returns true) while enabled.
pub fn rx_byte(sys: &System, byte: u8) {
    let phase = STATE.lock().unwrap().phase;
    match phase {
        Phase::Autobaud => {
            if byte == 0x7F {
                let mut st = STATE.lock().unwrap();
                st.phase = Phase::Command;
                st.buf.clear();
                drop(st);
                ack(sys);
            } else {
                nack(sys);
            }
        }
        Phase::Command => {
            let frame = {
                let mut st = STATE.lock().unwrap();
                st.buf.push(byte);
                if st.buf.len() < 2 {
                    return;
                }
                std::mem::take(&mut st.buf)
            };
            if frame[1] != !frame[0] {
                nack(sys);
                return;
            }
            match frame[0] {
                0x00 => {
                    // GET: ACK, N, version, command codes, final ACK.
                    const CODES: [u8; 7] = [0x00, 0x01, 0x02, 0x11, 0x21, 0x31, 0x43];
                    let mut r = vec![ACK, CODES.len() as u8, BOOT_VERSION];
                    r.extend_from_slice(&CODES);
                    r.push(ACK);
                    tx(sys, &r);
                }
                0x01 => {
                    tx(sys, &[ACK, BOOT_VERSION, 0x00, 0x00, ACK]);
                }
                0x02 => {
                    tx(sys, &[ACK, 0x01, PID_HI, PID_LO, ACK]);
                }
                0x11 | 0x21 | 0x31 => {
                    ack(sys);
                    let op = match frame[0] {
                        0x11 => Op::ReadAddr,
                        0x21 => Op::GoAddr,
                        _ => Op::WriteAddr,
                    };
                    let mut st = STATE.lock().unwrap();
                    st.phase = Phase::Collect { need: 5, then: op };
                    st.buf.clear();
                }
                0x43 => {
                    ack(sys);
                    let mut st = STATE.lock().unwrap();
                    st.phase = Phase::Collect { need: 1, then: Op::EraseList };
                    st.buf.clear();
                }
                _ => nack(sys),
            }
        }
        Phase::Collect { need, then } => {
            let frame = {
                let mut st = STATE.lock().unwrap();
                st.buf.push(byte);
                // Variable-length frames derive their total from the count
                // byte: [N][N+1 payload bytes][chk] = N+3.
                let total = match then {
                    Op::EraseList | Op::WriteData if st.buf.len() >= 1 => {
                        (st.buf[0] as usize) + 3
                    }
                    _ => need,
                };
                if st.buf.len() < total {
                    return;
                }
                st.phase = Phase::Command;
                std::mem::take(&mut st.buf)
            };
            run_frame(sys, then, &frame);
        }
    }
}

/// Collected multi-byte frames: address/length/data/erase handling.
fn run_frame(sys: &System, op: Op, buf: &[u8]) {
    match op {
        Op::ReadAddr => {
            if buf.len() != 5 || xor_all(&buf[..4]) != buf[4] {
                nack(sys);
                return;
            }
            let addr = be_u32(buf);
            if !valid_span(addr, 1) {
                nack(sys);
                return;
            }
            ack(sys);
            let mut st = STATE.lock().unwrap();
            st.phase = Phase::Collect { need: 2, then: Op::ReadLen };
            st.buf.clear();
            st.stash.copy_from_slice(&buf[..4]); // stash address
        }
        Op::ReadLen => {
            let addr = {
                let st = STATE.lock().unwrap();
                be_u32(&st.stash)
            };
            if buf.len() != 2 || buf[1] != !buf[0] {
                nack(sys);
                return;
            }
            let n = buf[0] as usize + 1;
            if n > MAX_XFER || !valid_span(addr, n) {
                nack(sys);
                return;
            }
            ack(sys);
            let data = native::rustcpu_mem_read(addr, n as u32);
            tx(sys, &data);
        }
        Op::GoAddr => {
            if buf.len() != 5 || xor_all(&buf[..4]) != buf[4] {
                nack(sys);
                return;
            }
            let addr = be_u32(buf);
            if !valid_span(addr, 4) {
                nack(sys);
                return;
            }
            STATE.lock().unwrap().go_addr = Some(addr);
            ack(sys);
        }
        Op::WriteAddr => {
            if buf.len() != 5 || xor_all(&buf[..4]) != buf[4] {
                nack(sys);
                return;
            }
            let addr = be_u32(buf);
            // Accept now; the data frame re-validates the full span.
            if !valid_span(addr, 1) {
                nack(sys);
                return;
            }
            ack(sys);
            let mut st = STATE.lock().unwrap();
            st.phase = Phase::Collect { need: 1, then: Op::WriteData };
            st.buf.clear();
            st.stash.copy_from_slice(&buf[..4]); // stash address
        }
        Op::WriteData => {
            let addr = {
                let st = STATE.lock().unwrap();
                be_u32(&st.stash)
            };
            // Frame: [N][d0..dN][chk = XOR(N, data...)], N+1 data bytes.
            if buf.is_empty() {
                nack(sys);
                return;
            }
            let n = buf[0] as usize + 1;
            if buf.len() != n + 2 || n > MAX_XFER {
                nack(sys);
                return;
            }
            if xor_all(&buf[..n + 1]) != buf[n + 1] {
                nack(sys);
                return;
            }
            if !valid_span(addr, n) {
                nack(sys);
                return;
            }
            ack(sys);
            native::rustcpu_mem_write_raw(addr, &buf[1..n + 1]);
        }
        Op::EraseList => {
            // Frame: [N][c0..cN][chk = XOR(N, codes)]. N==0xFF with a
            // single 0xFF code = mass erase; else 1KB pages by code.
            if buf.is_empty() || xor_all(&buf[..buf.len() - 1]) != buf[buf.len() - 1] {
                nack(sys);
                return;
            }
            let n = buf[0] as usize;
            let codes = &buf[1..buf.len() - 1];
            if codes.len() != n + 1 {
                nack(sys);
                return;
            }
            let ranges = match mem_ranges() {
                Some(r) => r,
                None => {
                    nack(sys);
                    return;
                }
            };
            let (fb, fl) = ranges.0;
            if n == 0xFF && codes == [0xFF] {
                native::rustcpu_mem_write_raw(fb, &vec![0xFF; fl as usize]);
            } else {
                for &c in codes {
                    let base = fb + (c as u32) * PAGE_SIZE as u32;
                    if base >= fb && base < fb + fl {
                        let len = ((fb + fl - base) as usize).min(PAGE_SIZE);
                        native::rustcpu_mem_write_raw(base, &vec![0xFF; len]);
                    }
                }
            }
            ack(sys);
        }
    }
}
