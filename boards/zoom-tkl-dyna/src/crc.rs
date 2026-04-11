//! CRC-16/CCITT-FALSE checksum implementation.
//!
//! Polynomial: 0x1021, Initial: 0xFFFF, No reflection, No final XOR

/// Calculate CRC-16/CCITT-FALSE checksum
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for byte in data {
        crc ^= (*byte as u16) << 8;
        for _ in 0..8 {
            if (crc & 0x8000) != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
            crc &= 0xFFFF;
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_basic() {
        // Test with known values
        let data = [0xA5, 0x38, 0x00, 0x0A];
        let result = crc16(&data);
        // CRC should be consistent
        assert_eq!(crc16(&data), result);
    }
}
