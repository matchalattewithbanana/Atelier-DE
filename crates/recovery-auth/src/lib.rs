//! Linux-PAM authentication primitive for a future dedicated recovery locker.
//! Not a locker, unlock authority, or installation mechanism.
#![cfg(all(target_os = "linux", feature = "pam"))]

use libc::{c_char, c_int, c_void};
use std::{ffi::{CStr, CString}, fs, os::unix::fs::MetadataExt, path::Path, ptr, sync::atomic::{compiler_fence, Ordering}};

// Linux-PAM ABI: security/_pam_types.h and security/_pam_appl.h.
const SUCCESS: c_int = 0;
const BUF_ERR: c_int = 5;
const CONV_ERR: c_int = 19;
const DISALLOW_NULL_AUTHTOK: c_int = 1;
const USER: c_int = 2;
const ECHO_OFF: c_int = 1;
const ERROR_MSG: c_int = 3;
const TEXT_INFO: c_int = 4;
const MAX_MESSAGES: usize = 32;
const MAX_SECRET: usize = 4096;
const SERVICE: &[u8] = b"atelier-recovery-locker\0";

#[repr(C)]
struct Message { style: c_int, text: *const c_char }
#[repr(C)]
struct Response { value: *mut c_char, retcode: c_int }
#[repr(C)]
struct Conversation {
    callback: unsafe extern "C" fn(c_int, *mut *const Message, *mut *mut Response, *mut c_void) -> c_int,
    data: *mut c_void,
}

#[link(name = "pam")]
extern "C" {
    fn pam_start(service: *const c_char, user: *const c_char, conv: *const Conversation, handle: *mut *mut c_void) -> c_int;
    fn pam_authenticate(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_acct_mgmt(handle: *mut c_void, flags: c_int) -> c_int;
    fn pam_get_item(handle: *const c_void, kind: c_int, item: *mut *const c_void) -> c_int;
    fn pam_end(handle: *mut c_void, status: c_int) -> c_int;
}

struct Secret<'a>(&'a mut [u8]);
impl Drop for Secret<'_> {
    fn drop(&mut self) {
        for byte in self.0.iter_mut() {
            // Clear the caller's buffer even on error. PAM owns its copies.
            unsafe { ptr::write_volatile(byte, 0) };
        }
        compiler_fence(Ordering::SeqCst);
    }
}
struct Attempt<'a> {
    secret: &'a [u8],
    used: bool,
    failed: bool,
    allow_secret: bool,
}

// One fixed password-only conversation. No username changes, repeated
// prompts, arbitrary interactive challenges or display of PAM text.
unsafe extern "C" fn converse(count: c_int, messages: *mut *const Message,
    output: *mut *mut Response, data: *mut c_void) -> c_int {
    if output.is_null() || data.is_null() { return CONV_ERR; }
    *output = ptr::null_mut();
    let attempt = &mut *data.cast::<Attempt<'_>>();
    let result = responses(count, messages, output, attempt);
    if result != SUCCESS { attempt.failed = true; }
    result
}

unsafe fn responses(count: c_int, messages: *mut *const Message,
    output: *mut *mut Response, attempt: &mut Attempt<'_>) -> c_int {
    if count <= 0 || count as usize > MAX_MESSAGES || messages.is_null() || attempt.failed {
        return CONV_ERR;
    }
    // Validate the entire batch before allocating or copying any secret.
    let mut prompt = None;
    for i in 0..count as usize {
        let message = *messages.add(i);
        if message.is_null() { return CONV_ERR; }
        match (*message).style {
            ECHO_OFF if attempt.allow_secret && !attempt.used && prompt.is_none() => prompt = Some(i),
            ERROR_MSG | TEXT_INFO => {},
            _ => return CONV_ERR,
        }
    }
    let array = libc::calloc(count as usize, std::mem::size_of::<Response>()).cast::<Response>();
    if array.is_null() { return BUF_ERR; }
    if let Some(i) = prompt {
        let value = libc::malloc(attempt.secret.len() + 1).cast::<u8>();
        if value.is_null() {
            libc::free(array.cast());
            return BUF_ERR;
        }
        ptr::copy_nonoverlapping(attempt.secret.as_ptr(), value, attempt.secret.len());
        *value.add(attempt.secret.len()) = 0;
        (*array.add(i)).value = value.cast();
        attempt.used = true;
    }
    // Linux-PAM takes ownership of calloc/malloc allocations on success.
    *output = array;
    SUCCESS
}

// Refuse Linux-PAM's fallback to the "other" service when the dedicated
// administrator-installed policy is missing. This does not audit its modules.
fn service_present() -> bool {
    let path = Path::new("/etc/pam.d/atelier-recovery-locker");
    let Ok(file) = fs::symlink_metadata(path) else { return false; };
    file.is_file() && file.uid() == 0 && file.mode() & 0o022 == 0 &&
        path.parent().is_some_and(|parent| parent.ancestors().all(|p|
            fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && m.uid() == 0 && m.mode() & 0o022 == 0)))
}

fn current_user() -> Option<CString> {
    let uid = unsafe { libc::getuid() };
    if uid == 0 || uid != unsafe { libc::geteuid() } { return None; }
    let mut size = 16 * 1024;
    loop {
        let mut buffer = vec![0u8; size];
        let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result = ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(uid, &mut entry, buffer.as_mut_ptr().cast(), buffer.len(), &mut result)
        };
        if status == libc::ERANGE && size < 1024 * 1024 { size *= 2; continue; }
        if status != 0 || result.is_null() || entry.pw_uid != uid || entry.pw_name.is_null() {
            return None;
        }
        let name = unsafe { CStr::from_ptr(entry.pw_name) };
        if name.to_bytes().is_empty() { return None; }
        return Some(name.to_owned());
    }
}

unsafe fn same_user(handle: *mut c_void, expected: &CStr) -> bool {
    let mut item = ptr::null();
    pam_get_item(handle, USER, &mut item) == SUCCESS &&
        !item.is_null() && CStr::from_ptr(item.cast()) == expected
}

/// Authenticate only the real, non-root desktop UID using the fixed system
/// PAM service. Returns a decision, not permission to unlock any compositor.
/// Call from an isolated locker worker, never the compositor event loop.
/// Consumes the supplied mutable password buffer by clearing it on return.
pub fn authenticate(password: &mut [u8]) -> bool {
    let secret = Secret(password);
    if secret.0.is_empty() || secret.0.len() > MAX_SECRET || secret.0.contains(&0) {
        return false;
    }
    if !service_present() { return false; }
    let Some(user) = current_user() else { return false; };
    let mut attempt = Attempt { secret: &*secret.0, used: false, failed: false, allow_secret: false };
    let conv = Conversation { callback: converse, data: (&mut attempt as *mut Attempt<'_>).cast() };
    let mut handle = ptr::null_mut();
    unsafe {
        let start = pam_start(SERVICE.as_ptr().cast(), user.as_ptr(), &conv, &mut handle);
        if start != SUCCESS || handle.is_null() { return false; }
        attempt.allow_secret = true;
        let mut status = if attempt.failed { CONV_ERR } else { pam_authenticate(handle, DISALLOW_NULL_AUTHTOK) };
        attempt.allow_secret = false;
        if status == SUCCESS && (!attempt.used || attempt.failed || !same_user(handle, &user)) {
            status = CONV_ERR;
        }
        if status == SUCCESS {
            status = pam_acct_mgmt(handle, DISALLOW_NULL_AUTHTOK);
            if status == SUCCESS && (attempt.failed || !same_user(handle, &user)) {
                status = CONV_ERR;
            }
        }
        let end = pam_end(handle, status);
        status == SUCCESS && end == SUCCESS && !attempt.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn call(attempt: &mut Attempt<'_>, styles: &[c_int]) -> (c_int, *mut Response) {
        let messages: Vec<_> = styles.iter().map(|&style| Message { style, text: ptr::null() }).collect();
        let mut pointers: Vec<_> = messages.iter().map(|m| m as *const Message).collect();
        let mut output = ptr::null_mut();
        let code = unsafe { converse(pointers.len() as c_int, pointers.as_mut_ptr(), &mut output, (attempt as *mut Attempt<'_>).cast()) };
        (code, output)
    }
    fn attempt() -> Attempt<'static> {
        Attempt { secret: b"synthetic-test", used: false, failed: false, allow_secret: true }
    }
    #[test]
    fn single_hidden_prompt_and_information() {
        let mut a = attempt();
        let (code, array) = call(&mut a, &[TEXT_INFO, ECHO_OFF, ERROR_MSG]);
        assert_eq!(code, SUCCESS);
        unsafe {
            assert!((*array).value.is_null());
            assert_eq!(CStr::from_ptr((*array.add(1)).value).to_bytes(), a.secret);
            assert!((*array.add(2)).value.is_null());
            libc::free((*array.add(1)).value.cast());
            libc::free(array.cast());
        }
        assert!(a.used);
        let (code, array) = call(&mut a, &[ECHO_OFF]);
        assert_eq!(code, CONV_ERR);
        assert!(array.is_null() && a.failed);
    }
    #[test]
    fn reject_entire_unsupported_batch_without_copying_secret() {
        for styles in [vec![ECHO_OFF, ECHO_OFF], vec![ECHO_OFF, 2], vec![99], vec![]] {
            let mut a = attempt();
            let (code, array) = call(&mut a, &styles);
            assert_eq!(code, CONV_ERR);
            assert!(array.is_null() && a.failed && !a.used);
        }
    }
    #[test]
    fn account_phase_cannot_request_password() {
        let mut a = attempt();
        a.allow_secret = false;
        let (code, array) = call(&mut a, &[ECHO_OFF]);
        assert_eq!(code, CONV_ERR);
        assert!(array.is_null() && a.failed);
    }
    #[test]
    fn invalid_password_is_cleared_without_pam() {
        let mut password = [1, 0, 2];
        assert!(!authenticate(&mut password));
        assert_eq!(password, [0; 3]);
        let mut password = vec![1; MAX_SECRET + 1];
        assert!(!authenticate(&mut password));
        assert!(password.iter().all(|&b| b == 0));
    }
}
