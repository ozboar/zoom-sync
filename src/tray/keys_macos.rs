//! Global keypress stream for reactive mode on macOS.
//!
//! Uses a listen-only CGEventTap attached to the main run loop, which requires
//! the Input Monitoring permission. The tray's AppKit event pump drives the run
//! loop, so key events are delivered during the regular UI polling tick.

use std::ffi::c_void;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::Stream;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::{StreamExt, Timeout};

type CFMachPortRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFStringRef = *const c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type CGEventType = u32;
type CGEventMask = u64;
type CGEventTapCallBack =
    extern "C" fn(CGEventTapProxy, CGEventType, CGEventRef, *mut c_void) -> CGEventRef;

const K_CG_SESSION_EVENT_TAP: u32 = 1;
const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;
const K_CG_EVENT_KEY_DOWN: CGEventType = 10;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: CGEventType = 0xFFFF_FFFE;
const K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT: CGEventType = 0xFFFF_FFFF;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFStringRef;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFMachPortInvalidate(port: CFMachPortRef);
    fn CFRunLoopGetMain() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRelease(cf: *const c_void);
}

/// State shared with the event tap callback
struct TapState {
    tx: UnboundedSender<std::io::Result<()>>,
    port: CFMachPortRef,
}

extern "C" fn tap_callback(
    _proxy: CGEventTapProxy,
    event_type: CGEventType,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    // SAFETY: user_info points to the boxed TapState owned by KeyStream, which
    // removes the tap from the run loop before the state is dropped.
    let state = unsafe { &*(user_info as *const TapState) };
    match event_type {
        K_CG_EVENT_KEY_DOWN => {
            let _ = state.tx.send(Ok(()));
        },
        // The system disables taps that are slow to respond; turn it back on
        K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT | K_CG_EVENT_TAP_DISABLED_BY_USER_INPUT => unsafe {
            CGEventTapEnable(state.port, true);
        },
        _ => {},
    }
    event
}

/// Stream of global keypresses with an idle timeout. Removes the event tap when dropped.
pub struct KeyStream {
    inner: Pin<Box<Timeout<UnboundedReceiverStream<std::io::Result<()>>>>>,
    source: CFRunLoopSourceRef,
    state: Box<TapState>,
}

impl Stream for KeyStream {
    type Item = <Timeout<UnboundedReceiverStream<std::io::Result<()>>> as Stream>::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl Drop for KeyStream {
    fn drop(&mut self) {
        unsafe {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), self.source, kCFRunLoopCommonModes);
            CFMachPortInvalidate(self.state.port);
            CFRelease(self.source);
            CFRelease(self.state.port);
        }
    }
}

/// Start listening for keypresses. Must be called on the main thread.
///
/// Returns `None` if Input Monitoring has not been granted yet. The first call
/// asks macOS to show the permission prompt.
pub fn open(idle_timeout: Duration) -> Option<KeyStream> {
    unsafe {
        if !CGPreflightListenEventAccess() {
            CGRequestListenEventAccess();
            eprintln!(
                "reactive mode: allow zoom-sync under System Settings > Privacy & Security > \
                 Input Monitoring, then restart zoom-sync"
            );
            return None;
        }

        let (tx, rx) = unbounded_channel();
        let mut state = Box::new(TapState {
            tx,
            port: std::ptr::null_mut(),
        });
        let port = CGEventTapCreate(
            K_CG_SESSION_EVENT_TAP,
            K_CG_HEAD_INSERT_EVENT_TAP,
            K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
            1 << K_CG_EVENT_KEY_DOWN,
            tap_callback,
            &mut *state as *mut TapState as *mut c_void,
        );
        if port.is_null() {
            eprintln!("reactive mode: failed to create keyboard event tap");
            return None;
        }
        state.port = port;

        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
        CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(port, true);

        Some(KeyStream {
            inner: Box::pin(UnboundedReceiverStream::new(rx).timeout(idle_timeout)),
            source,
            state,
        })
    }
}
