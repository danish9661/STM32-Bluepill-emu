import * as wasm from "./stm32_bluepill_wasm_bg.wasm";
import { __wbg_set_wasm } from "./stm32_bluepill_wasm_bg.js";

__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export {
    adc_set_sim_value, add_i2c_eeprom, add_i2c_oled, add_lcd, add_software_spi, add_spi_flash, add_touchscreen, can_inject_message, clear_current_interrupt, dma_get_pending, dma_get_pending_count, dma_set_completed, get_next_pending_interrupt, get_uart_output, gpio_read_input, gpio_read_output, gpio_set_input, has_pending_interrupt, init, init_svd, is_watchdog_reset_requested, periph_read, periph_write, set_intr_masks, step, step_batch, tick, touchscreen_set_touch, uart_rx_byte
} from "./stm32_bluepill_wasm_bg.js";
