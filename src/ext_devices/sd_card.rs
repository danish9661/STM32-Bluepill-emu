//! SD card backing image for the SDIO peripheral (STM32F103, SDHC only).
//!
//! The image is a plain sector array (512 B/sector); the card personality
//! (CID/CSD/OCR/RCA) is derived deterministically, with the CSD capacity
//! fields computed from the image size so real stacks (SdFat) size the
//! volume correctly. Only block-addressed SDHC is modeled (CCS=1); byte
//! addressing (SDSC) is out of scope.

/// 12-bit R1 status used for generic command success: READY_FOR_DATA + TRAN.
pub const R1_READY_TRAN: u32 = 0x900;
/// Default relative card address published by CMD3.
pub const DEFAULT_RCA: u32 = 0x1234;

pub struct SdCardConfig {
    pub peripheral: String,
    pub image: Vec<u8>,
}

pub struct SdCard {
    pub config: SdCardConfig,
    pub image: Vec<u8>,
    pub rca: u32,
}

impl SdCard {
    pub fn new(peripheral: &str, image: &[u8]) -> Self {
        SdCard {
            config: SdCardConfig { peripheral: peripheral.to_string(), image: image.to_vec() },
            image: image.to_vec(),
            rca: DEFAULT_RCA,
        }
    }

    pub fn name(&self) -> &str { &self.config.peripheral }

    /// Sector count (image is a whole number of 512 B sectors; round down).
    pub fn sectors(&self) -> u32 { (self.image.len() / 512).max(1) as u32 }

    /// R3 OCR value: voltage window + power-up/busy bit + (once ready) CCS.
    /// `ccs` false = standard-capacity card (byte addressing, SDSC).
    pub fn ocr(&self, ready: bool, ccs: bool) -> u32 {
        // 3.2-3.3V + 3.3-3.4V window, busy bit when still powering up.
        0x00FF_8000 | if ready { 0x8000_0000 | if ccs { 0x4000_0000 } else { 0 } } else { 0 }
    }

    /// R3 OCR for the MMC path (CMD1): same busy polarity, sector-access
    /// mode (bit 30) instead of SD's CCS once ready.
    pub fn ocr_mmc(&self, ready: bool) -> u32 {
        0x00FF_8000 | if ready { 0xC000_0000 } else { 0 }
    }

    /// 512-byte EXT_CSD register (MMC CMD8): revision, card type and sector
    /// count are the fields stacks actually consume.
    pub fn ext_csd(&self) -> Vec<u8> {
        let mut ext = vec![0u8; 512];
        ext[192] = 8; // EXT_CSD_REV: v5.1
        ext[196] = 0x03; // CARD_TYPE: 26 MHz + 52 MHz supported
        ext[183] = 0; // BUS_WIDTH: 1-bit default
        ext[185] = 0; // HS_TIMING: legacy speed
        let sec = self.sectors();
        ext[212..216].copy_from_slice(&sec.to_le_bytes()); // SEC_COUNT
        ext
    }

    /// 128-bit CID (R2 order: [127:96], [95:64], [63:32], [31:0]).
    pub fn cid(&self) -> [u32; 4] {
        // MID=0x03, OID="SD", PNM="EMU01", PRV=1.0, PSN=0x12345678.
        [
            0x0353_4430,
            0x454D_5530,
            0x3110_1234,
            0x5678_0101,
        ]
    }

    /// 128-bit CSD v2.0 (SDHC, R2 order), capacity derived from image size:
    /// C_SIZE = sectors/1024 - 1, capacity = (C_SIZE+1) * 512 KiB.
    pub fn csd(&self) -> [u32; 4] {
        let csize = self.sectors() / 1024;
        let csize = csize.saturating_sub(1).min(0x3F_FFFF);
        let mut v: u128 = 0;
        v |= 0b01 << 126;                    // CSD_STRUCTURE = 2.0
        v |= 0x32 << 112;                    // TRAN_SPEED
        v |= 0x5B5 << 84;                    // CCC
        v |= 9 << 80;                        // READ_BL_LEN = 512 B
        v |= (csize as u128) << 48;          // C_SIZE [69:48]
        v |= 1;                              // trailing 1 bit
        [
            (v >> 96) as u32,
            (v >> 64) as u32,
            (v >> 32) as u32,
            v as u32,
        ]
    }

    /// 128-bit CSD v1.0 (R2 order) for byte-addressed SDSC cards:
    /// C_SIZE is 12 bits with a C_SIZE_MULT multiplier (MULT=7 → 512-block
    /// units). Small images round UP to a whole unit; out-of-range reads
    /// zero-fill like the SDHC path.
    pub fn csd_v1(&self) -> [u32; 4] {
        let units = (self.sectors() + 511) / 512;
        let csize = units.saturating_sub(1).min(0xFFF);
        let mut v: u128 = 0;
        v |= 0b00 << 126;                    // CSD_STRUCTURE = 1.0
        v |= 0x32 << 104;                    // TRAN_SPEED
        v |= 0x5B5 << 92;                    // CCC (12 bits [103:92])
        v |= 9 << 80;                        // READ_BL_LEN = 512 B
        v |= (csize as u128) << 62;          // C_SIZE [73:62]
        v |= 7 << 47;                        // C_SIZE_MULT = 7 (x512 blocks)
        v |= 1;                              // trailing 1 bit
        [
            (v >> 96) as u32,
            (v >> 64) as u32,
            (v >> 32) as u32,
            v as u32,
        ]
    }

    /// R6 response to CMD3: RCA + status bits.
    pub fn r6(&self) -> u32 { (self.rca << 16) | 0x120 }

    pub fn read_block(&self, lba: u32, out: &mut [u8]) {
        let start = lba as usize * 512;
        // Out-of-range LBA: zero-fill (an empty range with an out-of-bounds
        // start still panics, so guard first — same as write_block).
        if start >= self.image.len() {
            for b in out.iter_mut() { *b = 0; }
            return;
        }
        let n = out.len().min(self.image.len() - start);
        out[..n].copy_from_slice(&self.image[start..start + n]);
        for b in &mut out[n..] { *b = 0; }
    }

    pub fn write_block(&mut self, lba: u32, data: &[u8]) {
        let start = lba as usize * 512;
        if start >= self.image.len() { return; }
        let n = data.len().min(self.image.len() - start);
        self.image[start..start + n].copy_from_slice(&data[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csd_capacity_matches_image() {
        // 2048 sectors = 1 MiB -> C_SIZE = 1 -> (1+1)*512KiB.
        let card = SdCard::new("SDIO", &vec![0u8; 2048 * 512]);
        let csd = card.csd();
        let v = ((csd[0] as u128) << 96) | ((csd[1] as u128) << 64)
            | ((csd[2] as u128) << 32) | csd[3] as u128;
        assert_eq!((v >> 126) & 3, 1, "CSD v2.0 structure");
        assert_eq!((v >> 48) & 0x3F_FFFF, 1, "C_SIZE for 2048 sectors");
        assert_eq!(v & 1, 1, "trailing bit");
    }

    #[test]
    fn block_rw_round_trip() {
        let mut card = SdCard::new("SDIO", &vec![0u8; 4 * 512]);
        let data: Vec<u8> = (0..512).map(|i| (i & 0xFF) as u8).collect();
        card.write_block(2, &data);
        let mut out = vec![0u8; 512];
        card.read_block(2, &mut out);
        assert_eq!(out, data);
        // Out-of-range reads zero-fill instead of panicking.
        card.read_block(99, &mut out);
        assert!(out.iter().all(|&b| b == 0));
    }
}
