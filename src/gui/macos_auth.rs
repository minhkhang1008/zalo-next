//! macOS owner authentication for App Lock: Touch ID, or the account password as a fallback
//! (`LAContext evaluatePolicy:LAPolicyDeviceOwnerAuthentication`).
//!
//! The system shows its own sheet and calls the reply block on a private queue; the result is
//! handed back through `done`, which must only set state and request a repaint.

use std::sync::Mutex;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, msg_send, msg_send_id};
use objc2_foundation::NSString;

#[link(name = "LocalAuthentication", kind = "framework")]
extern "C" {}

/// `LAPolicyDeviceOwnerAuthentication`: biometrics, Apple Watch or the account password.
const LA_POLICY_DEVICE_OWNER_AUTHENTICATION: isize = 2;

/// Whether the system can authenticate the owner at all (false e.g. without a login password).
pub(crate) fn available() -> bool {
    // SAFETY: LAContext is a plain NSObject subclass; `alloc`/`init` return a +1 object that
    // `Retained` releases. `canEvaluatePolicy:error:` takes an NSInteger policy and a nullable
    // out-pointer for the error, which we do not request.
    unsafe {
        let context: Retained<AnyObject> =
            msg_send_id![msg_send_id![class!(LAContext), alloc], init];
        let ok: Bool = msg_send![
            &*context,
            canEvaluatePolicy: LA_POLICY_DEVICE_OWNER_AUTHENTICATION,
            error: std::ptr::null_mut::<*mut AnyObject>()
        ];
        ok.as_bool()
    }
}

/// Show the system prompt with `reason`. `done(true)` once the owner is verified, `done(false)`
/// on cancel or failure. `done` runs on a system queue, at most once.
pub(crate) fn authenticate(reason: &str, done: impl FnOnce(bool) + Send + 'static) {
    // The system prompt belongs to the frontmost app. Make sure that is us right before asking;
    // otherwise (seen with an unbundled `cargo run` binary) the prompt can wait in the background
    // until the user switches apps.
    if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
        #[allow(deprecated)]
        objc2_app_kit::NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
    // SAFETY: as in `available`.
    let context: Retained<AnyObject> =
        unsafe { msg_send_id![msg_send_id![class!(LAContext), alloc], init] };
    // An LAContext that is deallocated cancels its evaluation, so the reply block owns a
    // reference until the system has answered (AppKit releases the copied block afterwards).
    let keep_alive = Mutex::new(Some(context.clone()));
    let done = Mutex::new(Some(done));
    let reply = RcBlock::new(move |success: Bool, _error: *mut AnyObject| {
        let callback = done.lock().ok().and_then(|mut slot| slot.take());
        if let Some(callback) = callback {
            callback(success.as_bool());
        }
        if let Ok(mut slot) = keep_alive.lock() {
            slot.take();
        }
    });
    let reason = NSString::from_str(reason);
    // SAFETY: `evaluatePolicy:localizedReason:reply:` copies the block; the reply signature is
    // `void (^)(BOOL, NSError *)`, matching the block above. The NSString outlives the call.
    unsafe {
        let _: () = msg_send![
            &*context,
            evaluatePolicy: LA_POLICY_DEVICE_OWNER_AUTHENTICATION,
            localizedReason: &*reason,
            reply: &*reply
        ];
    }
}

#[cfg(test)]
mod tests {
    /// Exercises the FFI without prompting: objc2 checks the method encodings in debug builds,
    /// so a wrong selector or argument type fails here instead of in the app.
    #[test]
    fn can_query_the_owner_authentication_policy() {
        let _ = super::available();
    }
}
