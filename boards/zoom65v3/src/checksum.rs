/// Checksum function used for media uploads
pub fn checksum(data: &[u8]) -> [u8; 4] {
    const A: isize = 4294967295;
    let mut val = A;
    for byte in data {
        val ^= (*byte as isize) << 24;
        for _ in 0..8 {
            if val & 2147483648 != 0 {
                val = (val << 1) ^ 4374732215;
            } else {
                val <<= 1;
            }
            val &= A;
        }
    }
    [
        (val >> 24 & 255) as u8,
        (val >> 16 & 255) as u8,
        (val >> 8 & 255) as u8,
        (val & 255) as u8,
    ]
}

#[cfg(test)]
#[test]
fn checksum_test() {
    assert_eq!(
        checksum(&[
            0, 0, 71, 73, 70, 56, 57, 97, 111, 0, 111, 0, 247, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0,
        ]),
        [94, 148, 189, 206],
        "checksum should be the same as test data"
    );
}
