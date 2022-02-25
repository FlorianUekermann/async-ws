use std::task::{RawWakerVTable, RawWaker, Waker};
use std::sync::{Mutex, Weak};
use crate::connection::WsConnectionInner;
use futures::{AsyncRead, AsyncWrite};
use std::mem::ManuallyDrop;

pub(crate) fn new_waker<T: AsyncRead + AsyncWrite + Unpin>(data: Weak<Mutex<WsConnectionInner<T>>>) -> Waker {
    unsafe fn clone_waker<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) -> RawWaker {
        let weak = ManuallyDrop::new(Weak::from_raw(raw as *const Mutex<WsConnectionInner<T>>));
        let clone= ManuallyDrop::into_inner(weak.clone());
        RawWaker::new(
            Weak::into_raw(clone) as *const (),
            &RawWakerVTable::new(clone_waker::<T>, wake::<T>, wake_by_ref::<T>, drop_waker::<T>),
        )
    }

    unsafe fn wake<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) {
        wake_by_ref::<T>(raw);
        drop_waker::<T>(raw);
    }

    unsafe fn wake_by_ref<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) {
        let weak = ManuallyDrop::new(Weak::from_raw(raw as *const Mutex<WsConnectionInner<T>>));
        if let Some(strong) = weak.upgrade() {
            let mut inner = strong.lock().unwrap();
            let wakers = [inner.reader_waker.take(), inner.stream_waker.take(), inner.writer_waker.take(), inner.send_waker.take()];
            drop(inner);
            for w in wakers {
                if let Some(waker) = w {
                    waker.wake()
                }
            }
        }
    }

    unsafe fn drop_waker<T: AsyncRead + AsyncWrite + Unpin>(raw: *const ()) {
        let weak = Weak::from_raw(raw as *const Mutex<WsConnectionInner<T>>);
        // dbg!(weak.strong_count()+weak.weak_count()-1);
        drop(weak);
    }

    let raw_waker = RawWaker::new(
        Weak::into_raw(data) as *const (),
        &RawWakerVTable::new(clone_waker::<T>, wake::<T>, wake_by_ref::<T>, drop_waker::<T>),
    );
    unsafe { Waker::from_raw(raw_waker) }
}