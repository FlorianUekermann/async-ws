use crate::connection::WsConnectionInner;
use futures::{AsyncRead, AsyncWrite};
use std::mem::{take, ManuallyDrop};
use std::sync::{Mutex, Weak};
use std::task::{Poll, RawWaker, RawWakerVTable, Waker};

#[derive(Default)]
pub(crate) struct Wakers {
    pub stream_waker: Option<Waker>,
    pub send_waker: Option<Waker>,
    pub writer_waker: Option<Waker>,
    pub reader_waker: Option<Waker>,
}

impl Wakers {
    pub(crate) fn wake(&mut self) {
        fn take_and_wake(o: &mut Option<Waker>) {
            if let Some(w) = o.take() {
                w.wake();
            }
        }
        take_and_wake(&mut self.send_waker);
        take_and_wake(&mut self.stream_waker);
        take_and_wake(&mut self.writer_waker);
        take_and_wake(&mut self.reader_waker);
    }
    pub(crate) fn wake_on_err<O, E>(&mut self, p: &Poll<Result<O, E>>) {
        if let Poll::Ready(Err(_)) = &p {
            self.wake()
        }
    }
}

pub(crate) fn new_waker<T: AsyncRead + AsyncWrite + Unpin>(
    data: Weak<Mutex<(WsConnectionInner<T>, Wakers)>>,
) -> Waker {
    unsafe fn clone_waker<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) -> RawWaker {
        let weak = ManuallyDrop::new(Weak::from_raw(
            raw as *const Mutex<(WsConnectionInner<T>, Wakers)>,
        ));
        let clone = ManuallyDrop::into_inner(weak.clone());
        RawWaker::new(
            Weak::into_raw(clone) as *const (),
            &RawWakerVTable::new(
                clone_waker::<T>,
                wake::<T>,
                wake_by_ref::<T>,
                drop_waker::<T>,
            ),
        )
    }

    unsafe fn wake<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) {
        wake_by_ref::<T>(raw);
        drop_waker::<T>(raw);
    }

    unsafe fn wake_by_ref<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) {
        let weak = ManuallyDrop::new(Weak::from_raw(
            raw as *const Mutex<(WsConnectionInner<T>, Wakers)>,
        ));
        if let Some(strong) = weak.upgrade() {
            let mut guard = strong.lock().unwrap();
            let mut wakers = take(&mut guard.1);
            drop(guard);
            wakers.wake();
        }
    }

    unsafe fn drop_waker<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) {
        let weak = Weak::from_raw(raw as *const Mutex<(WsConnectionInner<T>, Wakers)>);
        drop(weak);
    }

    let raw_waker = RawWaker::new(
        Weak::into_raw(data) as *const (),
        &RawWakerVTable::new(
            clone_waker::<T>,
            wake::<T>,
            wake_by_ref::<T>,
            drop_waker::<T>,
        ),
    );
    unsafe { Waker::from_raw(raw_waker) }
}
