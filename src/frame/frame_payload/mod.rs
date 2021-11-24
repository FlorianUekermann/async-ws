pub mod decode;
pub use decode::*;

// Masks or unmasks a buffer with payload bytes. The offset is the offset of the buffer within the
// frames payload segment. Any multiple of 4 may be added to or subtracted from the offset without
// any effect on the result.
pub fn payload_mask(mask: [u8; 4], mut offset: usize, buffer: &mut [u8]) {
    if mask != [0u8, 0u8, 0u8, 0u8] {
        for byte in buffer.iter_mut() {
            *byte ^= mask[offset & 3];
            offset = offset.wrapping_add(1);
        }
    }
}

// Calculate maximum payload length given a maximum frame size.
pub fn max_payload_len(masked: bool, max_frame_size: u64) -> u64 {
    match (masked, max_frame_size) {
        (true, 0..=6) => 0,
        (true, 7..=132) => max_frame_size - 6,
        (true, 133..=134) => 126,
        (true, 135..=65542) => max_frame_size - 8,
        (true, 65543..=65548) => 65536,
        (true, 65549..=2147483661) => max_frame_size - 14,
        (false, 0..=2) => 0,
        (false, 3..=128) => max_frame_size - 2,
        (false, 129..=130) => 126,
        (false, 131..=65538) => max_frame_size - 4,
        (false, 65539..=65544) => 65536,
        (false, 65545..=2147483657) => max_frame_size - 10,
        _ => 2147483647,
    }
}
