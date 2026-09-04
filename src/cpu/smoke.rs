// Native bring-up tests for the Rust CPU core (Path B): run real firmware
// binaries through `Cpu` + `FlatMemory` + the real peripheral model with no
// JS/Unicorn involvement. The driver loop below (sliced run + step_batch +
// mask sync) is the reference shape for the future wasm/JS backend.

use super::{Cpu, mem::{FlatMemory, Memory}};
use crate::{init, sys, step_batch, set_intr_masks, get_uart_output, gpio_read_output, uart_rx_byte};
use crate::interrupts::intr_next;

/// Run one handler (IRQ or SVC) to exception return, capped like the JS
/// handler runs. Returns false if the handler itself faulted or hung.
/// Single-steps (budget 1) so a handler return never overshoots into thread
/// code: `run(1000)` keeps executing past `bx lr` for the rest of the budget,
/// running ~1000 thread instructions unticked — from an SPI2 print that
/// overshoot reaches testSVC and faults on `svc #2` (seen 2026-09-04).
fn run_handler_to_return(cpu: &mut Cpu, mem: &mut FlatMemory) -> bool {
    let sys = sys();
    let mut hdone = 0u32;
    while cpu.ipsr != 0 && hdone < 20000 && cpu.fault.is_none() {
        set_intr_masks(cpu.regs.primask, 0);
        hdone += cpu.run(sys, mem, 1);
    }
    cpu.fault.is_none()
}

/// Dispatch all pending interrupts within the shared per-batch budget.
fn dispatch_interrupts(cpu: &mut Cpu, mem: &mut FlatMemory) -> bool {
    let sys = sys();
    loop {
        let irq = intr_next();
        if irq <= -100 {
            return true;
        }
        cpu.take_exception(sys, mem, irq);
        if !run_handler_to_return(cpu, mem) {
            return false;
        }
    }
}

/// SVC fault from run(): consume it, step past the svc, and dispatch the
/// SVCall handler synchronously (mirrors the JS intr_svc_enter path, but on
/// the real stack, so no mirror is needed).
fn handle_svc(cpu: &mut Cpu, mem: &mut FlatMemory) -> bool {
    let sys = sys();
    match cpu.fault.take() {
        Some(f) if f.op1 & 0xFF00 == 0xDF00 => {
            cpu.regs.r[15] = f.pc.wrapping_add(2) | 1;
            cpu.take_exception(sys, mem, -5);
            run_handler_to_return(cpu, mem)
        }
        other => {
            cpu.fault = other;
            false
        }
    }
}

/// One emulation slice: DMA pump, CPU run, SVC catch, peripheral tick, DMA
/// pump, interrupt dispatch, watchdog check. Returns false to stop.
fn step_slice(cpu: &mut Cpu, mem: &mut FlatMemory, slice: u32) -> bool {
    use crate::is_watchdog_reset_requested;
    let sys = sys();
    if crate::dma_get_pending_count() > 0 {
        let plan = sys.dma_build_plan();
        sys.dma_exec_plan(mem, &plan);
    }
    set_intr_masks(cpu.regs.primask, 0);
    let n = cpu.run(sys, mem, slice);
    if cpu.fault.is_some() && !handle_svc(cpu, mem) {
        return false;
    }
    if n == 0 && cpu.fault.is_none() {
        // Asleep with nothing pending (or empty budget slice): still tick
        // time forward so a pending wake can land.
    }
    step_batch(slice);
    if crate::dma_get_pending_count() > 0 {
        let plan = sys.dma_build_plan();
        sys.dma_exec_plan(&mut *mem, &plan);
    }
    if !dispatch_interrupts(cpu, mem) {
        return false;
    }
    !is_watchdog_reset_requested()
}

fn boot(path: &str) -> (Cpu, FlatMemory) {
    // NOTE: no lock here — each test holds crate::test_util::lock() for its
    // whole body (the guard must outlive the run; returning it would work
    // too, but explicit test-scope locking reads clearer).
    init();
    let fw = std::fs::read(path).unwrap();
    assert!(fw.len() >= 8, "firmware too small");
    let sp = u32::from_le_bytes([fw[0], fw[1], fw[2], fw[3]]);
    let pc = u32::from_le_bytes([fw[4], fw[5], fw[6], fw[7]]);
    assert!(sp != 0 && pc != 0, "bad vector table");
    let mut mem = FlatMemory::new(64 * 1024, 20 * 1024);
    mem.load(&fw, 0x08000000);
    assert_eq!(mem.read32(0x08000000), sp, "flash load failed");
    let mut cpu = Cpu::new(sp, pc | 1);
    cpu.dsp = false; // Cortex-M3: DSP extension faults
    cpu.deliver_irqs = false; // lazy batch-boundary dispatch (JS parity)
    (cpu, mem)
}

#[test]
fn blinky_runs_and_toggles_led() {
    let _held = crate::test_util::lock();
    let (mut cpu, mut mem) = boot("site/blink.bin");
    let start = gpio_read_output(2, 13);
    let mut flips = 0;
    let mut last = start;
    let mut done = 0u64;
    while done < 10_000_000 && flips < 2 {
        // drive in 200K-instruction legs so a hang still terminates
        let mut leg = 0u64;
        while leg < 200_000 {
            if !step_slice(&mut cpu, &mut mem, 2000) {
                break;
            }
            leg += 2000;
            let now = gpio_read_output(2, 13);
            if now != last {
                flips += 1;
                last = now;
            }
            if flips >= 2 {
                break;
            }
        }
        done += leg;
        if cpu.fault.is_some() || leg == 0 {
            break;
        }
    }
    assert!(cpu.fault.is_none(), "cpu faulted: {:?}", cpu.fault);
    assert!(flips >= 2, "PC13 never toggled in {done} instructions");
}

#[test]
fn echo_banner_and_roundtrip() {
    let _held = crate::test_util::lock();
    let (mut cpu, mut mem) = boot("tests/arduino_echo/build/arduino_echo.ino.bin");
    // collect the boot banner (USART1 TX)
    let mut out = String::new();
    let mut done = 0u64;
    while done < 15_000_000 && !out.contains("Echo demo") {
        if !step_slice(&mut cpu, &mut mem, 2000) {
            break;
        }
        done += 2000;
        out.push_str(&get_uart_output());
    }
    assert!(cpu.fault.is_none(), "cpu faulted: {:?}", cpu.fault);
    assert!(out.contains("Echo demo"), "no banner in {done} instructions");
    // round-trip: inject 'A', expect it back
    let mark = out.len();
    assert!(uart_rx_byte(0x40013800, 0x41));
    while done < 25_000_000 {
        if !step_slice(&mut cpu, &mut mem, 2000) {
            break;
        }
        done += 2000;
        out.push_str(&get_uart_output());
        if out[mark..].contains('A') {
            break;
        }
    }
    assert!(out[mark..].contains('A'), "echo byte never returned");
}

fn parse_can_flag(map: &str) -> u32 {
    for line in map.lines() {
        let t = line.trim();
        if t.ends_with("canRxArmed") {
            for tok in t.split_whitespace() {
                if let Some(hex) = tok.strip_prefix("0x") {
                    if let Ok(v) = u32::from_str_radix(hex, 16) {
                        if v >= 0x20000000 {
                            return v;
                        }
                    }
                }
            }
        }
    }
    panic!("canRxArmed not found in map");
}

#[test]
fn periph39_full_run_native() {
    let _held = crate::test_util::lock();
    use crate::{
        add_i2c_eeprom, add_spi_flash, add_i2c_oled, add_lcd, add_touchscreen,
        uart_rx_pending, dma_get_pending_count, can_inject_message, reset_ext_devices,
    };
    let rb = |p: &str| std::fs::read(p).unwrap();
    reset_ext_devices();
    add_i2c_eeprom("I2C1", 0x50, &rb("tests/arduino_periph_test/build/eeprom.bin"));
    add_i2c_eeprom("I2C2", 0x51, &rb("tests/arduino_periph_test/build/eeprom2.bin"));
    add_i2c_oled("I2C1", 0x3C, 128, 64);
    add_spi_flash("SPI1", 0xEF4016, &rb("tests/arduino_periph_test/build/spi_flash.bin"), Some("PA4".to_string()));
    add_spi_flash("SPI2", 0xEF4017, &rb("tests/arduino_periph_test/build/spi_flash2.bin"), Some("PB12".to_string()));
    add_lcd("SPI1", Some("PA1".to_string()));
    add_touchscreen("SPI1", Some("PA3".to_string()), Some("PA2".to_string()));
    init();
    let fw = rb("tests/arduino_periph_test/build/arduino_periph_test.ino.bin");
    let sp = u32::from_le_bytes([fw[0], fw[1], fw[2], fw[3]]);
    let pc = u32::from_le_bytes([fw[4], fw[5], fw[6], fw[7]]);
    let mut mem = FlatMemory::new(64 * 1024, 20 * 1024);
    mem.load(&fw, 0x08000000);
    let mut cpu = Cpu::new(sp, pc | 1);
    cpu.dsp = false;
    let can_flag = parse_can_flag(&String::from_utf8_lossy(&rb(
        "tests/arduino_periph_test/build/arduino_periph_test.ino.map",
    )));
    let uart = 0x40013800u32;
    let mut stdin_q: Vec<u8> = vec![0x41, 0x42];
    let mut can_done = false;
    let mut done = 0u64;
    const MAX: u64 = 200_000_000;
    const SLICE: u32 = 20000;
    while done < MAX {
        while !stdin_q.is_empty()
            && uart_rx_pending(uart) == 0
            && dma_get_pending_count() == 0
        {
            uart_rx_byte(uart, stdin_q.remove(0));
        }
        if !step_slice(&mut cpu, &mut mem, SLICE) {
            break;
        }
        done += SLICE as u64;
        if !can_done && mem.read32(can_flag) != 0 {
            can_done = can_inject_message(0x40006400, 0 << 21, 2, 0xDEAD, 0);
        }
    }
    let out = get_uart_output();
    assert!(cpu.fault.is_none(), "cpu faulted: {:?}", cpu.fault);
    assert!(out.contains("SUMMARY pass=39 fail=0"), "39/39 missing after {done} instr, can={can_done}:\n{}", tail(&out, 6000));
}

fn tail(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("...{}", &s[s.len() - n..])
    }
}

/// True when `out` contains `key=` followed by a live decimal digit (rules
/// out banner echoes like "adc=<0..4095>" which contain the bare key).
fn has_live_value(out: &str, key: &str) -> bool {
    let needle = format!("{key}=");
    out.match_indices(&needle).any(|(i, _)| {
        out[i + needle.len()..].chars().next().is_some_and(|c| c.is_ascii_digit())
    })
}
/// Boot a raw .bin image at flash base with a fresh model (caller adds ext
/// devices first, after reset_ext_devices()).
fn boot_image(path: &str) -> (Cpu, FlatMemory) {
    init();
    let fw = std::fs::read(path).unwrap();
    assert!(fw.len() >= 8, "firmware too small: {path}");
    let sp = u32::from_le_bytes([fw[0], fw[1], fw[2], fw[3]]);
    let pc = u32::from_le_bytes([fw[4], fw[5], fw[6], fw[7]]);
    assert!(sp != 0 && pc != 0, "bad vector table: {path}");
    let mut mem = FlatMemory::new(64 * 1024, 20 * 1024);
    mem.load(&fw, 0x08000000);
    let mut cpu = Cpu::new(sp, pc | 1);
    cpu.dsp = false; // Cortex-M3
    cpu.deliver_irqs = false; // lazy batch-boundary dispatch (product parity)
    (cpu, mem)
}

/// Run until every marker appears in UART output (but always at least `min`
/// instructions, so banner-echoing markers can't stop the run before live
/// values print) or `max` instructions. Returns (instructions, full output).
/// Faults stop the run (asserted after).
fn run_until(cpu: &mut Cpu, mem: &mut FlatMemory, max: u64, min: u64, markers: &[&str]) -> (u64, String) {
    let mut out = String::new();
    let mut done = 0u64;
    const SLICE: u32 = 20000;
    while done < max {
        if !step_slice(cpu, mem, SLICE) {
            break;
        }
        done += SLICE as u64;
        out.push_str(&get_uart_output());
        if done >= min && markers.iter().all(|m| out.contains(m)) {
            break;
        }
    }
    out.push_str(&get_uart_output());
    (done, out)
}

/// Firmware gallery: every other shipped demo boots fault-free natively and
/// prints its banner + first live values (same bar as the browser presets).
#[test]
fn firmware_gallery_native() {
    let _held = crate::test_util::lock();
    use crate::{add_i2c_eeprom, add_spi_flash, add_i2c_oled, add_lcd, reset_ext_devices};
    let rb = |p: &str| std::fs::read(p).unwrap();

    // ADC on UART (PA0 sim value, prints every second).
    reset_ext_devices();
    let (mut cpu, mut mem) = boot_image("tests/arduino_adc_uart/build/arduino_adc_uart.ino.bin");
    let (done, out) = run_until(&mut cpu, &mut mem, 120_000_000, 90_000_000, &["ADC on UART demo"]);
    assert!(cpu.fault.is_none(), "adc_uart faulted: {:?}", cpu.fault);
    assert!(has_live_value(&out, "adc"), "adc_uart no live readings after {done}:\n{}", tail(&out, 500));

    // TIM2 timer on UART (1s lines).
    reset_ext_devices();
    let (mut cpu, mut mem) = boot_image("tests/arduino_timer_uart/build/arduino_timer_uart.ino.bin");
    let (done, out) = run_until(&mut cpu, &mut mem, 120_000_000, 90_000_000, &["TIM2 timer on UART demo"]);
    assert!(cpu.fault.is_none(), "timer_uart faulted: {:?}", cpu.fault);
    assert!(has_live_value(&out, "cnt"), "timer_uart no live lines after {done}:\n{}", tail(&out, 500));

    // TIM2 PWM fade (4 duty lines, then silent breathing).
    reset_ext_devices();
    let (mut cpu, mut mem) = boot_image("tests/arduino_fade/build/arduino_fade.ino.bin");
    let (done, out) = run_until(&mut cpu, &mut mem, 40_000_000, 10_000_000, &["TIM2 PWM fade demo"]);
    assert!(cpu.fault.is_none(), "fade faulted: {:?}", cpu.fault);
    assert!(has_live_value(&out, "duty"), "fade no duty lines after {done}:\n{}", tail(&out, 500));

    // SPI flash + I2C EEPROM + OLED probe (setup-only prints).
    reset_ext_devices();
    add_i2c_eeprom("I2C1", 0x50, &rb("tests/arduino_periph_test/build/eeprom.bin"));
    add_i2c_oled("I2C1", 0x3C, 128, 64);
    add_spi_flash("SPI1", 0xEF4016, &rb("tests/arduino_periph_test/build/spi_flash.bin"), Some("PA4".to_string()));
    let (mut cpu, mut mem) = boot_image("tests/arduino_flash_demo/build/arduino_flash_demo.ino.bin");
    let (done, out) = run_until(&mut cpu, &mut mem, 40_000_000, 0, &["JEDEC=EF4016", "OLED=found", "EEPROM wr=1 rd=42"]);
    assert!(cpu.fault.is_none(), "flash_demo faulted: {:?}", cpu.fault);
    assert!(
        out.contains("JEDEC=EF4016") && out.contains("OLED=found") && out.contains("EEPROM wr=1 rd=42"),
        "flash_demo mismatch after {done}:\n{}", tail(&out, 800)
    );

    // 7-peripheral showcase (OLED + LCD; 7-seg/RGB/buzzer/button need no images).
    reset_ext_devices();
    add_i2c_oled("I2C1", 0x3C, 128, 64);
    add_lcd("SPI1", Some("PA8".to_string()));
    let (mut cpu, mut mem) = boot_image("tests/arduino_hw_showcase/build/arduino_hw_showcase.ino.bin");
    let (done, out) = run_until(&mut cpu, &mut mem, 150_000_000, 100_000_000, &["Peripheral showcase", "BTN=armed"]);
    assert!(cpu.fault.is_none(), "showcase faulted: {:?}", cpu.fault);
    assert!(out.contains("BTN=armed") && has_live_value(&out, "t"), "showcase not live after {done}:\n{}", tail(&out, 500));

    // WS2812 strip over SPI1+DMA (frames print ~2s in).
    reset_ext_devices();
    let (mut cpu, mut mem) = boot_image("tests/arduino_ws2812/build/arduino_ws2812.ino.bin");
    let (done, out) = run_until(&mut cpu, &mut mem, 200_000_000, 170_000_000, &["WS2812=ok"]);
    assert!(cpu.fault.is_none(), "ws2812 faulted: {:?}", cpu.fault);
    assert!(has_live_value(&out, "frames"), "ws2812 no frames after {done}:\n{}", tail(&out, 500));
}
