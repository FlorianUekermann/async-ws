pub mod decode;
pub use decode::*;

// Masks or unmasks a buffer with payload bytes. The offset is the offset of the buffer within the
// frames payload segment. Any multiple of 4 may be added to or subtracted from the offset without
// any effect on the result.
pub fn mask(mask: [u8; 4], mut offset: usize, buffer: &mut [u8]) {
    if mask != [0u8, 0u8, 0u8, 0u8] {
        for byte in buffer.iter_mut() {
            *byte ^= mask[offset & 3];
            offset = offset.wrapping_add(1);
        }
    }
}
