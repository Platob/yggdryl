//! Arrow casts owned by this datatype family.

use arrow_buffer::i256;

pub(crate) struct DecimalText {
    bytes: [u8; 78],
    start: usize,
}

impl DecimalText {
    pub(crate) fn new(value: i256) -> Self {
        let negative = value.is_negative();
        let mut raw = value.to_le_bytes();
        if negative {
            let mut carry = true;
            for byte in &mut raw {
                *byte = !*byte;
                if carry {
                    let (next, overflow) = byte.overflowing_add(1);
                    *byte = next;
                    carry = overflow;
                }
            }
        }
        let mut limbs = [0_u64; 4];
        for (limb, chunk) in limbs.iter_mut().zip(raw.chunks_exact(8)) {
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(chunk);
            *limb = u64::from_le_bytes(bytes);
        }

        let mut bytes = [0_u8; 78];
        let mut start = bytes.len();
        loop {
            let mut remainder = 0_u128;
            for limb in limbs.iter_mut().rev() {
                let value = (remainder << 64) | u128::from(*limb);
                *limb = u64::try_from(value / 10).unwrap_or(u64::MAX);
                remainder = value % 10;
            }
            start -= 1;
            bytes[start] = b'0' + u8::try_from(remainder).unwrap_or(0);
            if limbs.iter().all(|limb| *limb == 0) {
                break;
            }
        }
        if negative {
            start -= 1;
            bytes[start] = b'-';
        }
        Self { bytes, start }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[self.start..]
    }
}
