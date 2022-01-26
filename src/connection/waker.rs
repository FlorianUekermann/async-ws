use std::task::{Waker, RawWakerVTable, RawWaker};
use std::sync::Arc;
use std::mem;

pub(super) struct Wakers {
    pub stream: Option<Waker>,
}

impl Wakers {
    unsafe fn from_raw(w: *const ()) -> Arc<Wakers> {
        Arc::from_raw(w as *mut Wakers)
    }
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: Arc<Self>) {
        if let Some(w) = &self.stream {
            w.wake_by_ref();
        }
        mem::forget(self)
    }
    fn raw_waker(self: &Arc<Self>) -> RawWaker {
        let w = Arc::as_ptr(self) as *const ();
        RawWaker::new(w, &VTABLE)
    }
    pub fn waker(self: &Arc<Self>) -> Waker {
        unsafe { Waker::from_raw(self.raw_waker()) }
    }
}

const VTABLE: RawWakerVTable = RawWakerVTable::new(
    |w| -> RawWaker {
        unsafe {
            let w = Wakers::from_raw(w);
            mem::forget(w.clone());
            let c = w.raw_waker();
            mem::forget(w);
            c
        }
    },
    |w| {
        unsafe { Wakers::from_raw(w).wake() };
    },
    |w| {
        unsafe { Wakers::from_raw(w).wake_by_ref() };
    },
    |w| {
        unsafe {  Wakers::from_raw(w) };
    },
);