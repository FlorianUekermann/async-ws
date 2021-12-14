use utf8::Incomplete;

pub(super) fn process_utf8(state: &mut Incomplete, input: &[u8]) -> bool {
    for byte in input {
        if let Some((Err(_), _)) = state.try_complete(std::slice::from_ref(byte)) {
            return false;
        }
    }
    true
}
