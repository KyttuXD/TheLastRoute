//! The browser build's bridge to the page: the page's WebSocket (the room relay) and its synchronous HTTP (the
//! directory calls). The page registers the functions as the `lr_net` miniquad plugin in dist/web/index.html; this
//! module declares them as `extern "C"` imports and wraps them in safe functions.
//!
//! Native builds have no page. Every function here answers "not available" there, and nothing on the native path
//! reaches them (the UDP relay and the threaded HTTP client serve the native game).
//!
//! Handles are small integers owned by the page. A WebSocket handle is closed by [`ws_close`] and forgotten by the page
//! once its close has been read. An HTTP answer is released by [`http_request`] itself.

/// The answer to one HTTP request. `status` 0 means no answer came back (the page could not reach the server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebReply {
    pub status: u16,
    /// The response headers as `name: value` lines.
    pub headers: String,
    pub body: Vec<u8>,
}

/// One message the page queued for a WebSocket.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsItem {
    Text(Vec<u8>),
    Binary(Vec<u8>),
    /// The socket closed: the close code and reason.
    Closed { code: u16, reason: String },
}

#[cfg(target_arch = "wasm32")]
mod ffi {
    extern "C" {
        /// Copies the page's API origin (`window.LR_API_BASE`, else the page origin) into `dst`; returns its length.
        pub fn lr_api_base(dst: *mut u8, cap: u32) -> i32;
        /// Opens a WebSocket to `url` and sends `hello` as its first text message once open. 0 when the page refuses.
        pub fn lr_ws_open(url: *const u8, url_len: u32, hello: *const u8, hello_len: u32) -> u32;
        /// Kind of the oldest queued message: 0 none, 1 text, 2 binary, 3 closed.
        pub fn lr_ws_peek_kind(handle: u32) -> i32;
        /// Byte length of the oldest queued message.
        pub fn lr_ws_peek_len(handle: u32) -> i32;
        /// Copies and removes the oldest queued message (at most `cap` bytes); returns its length or -1.
        pub fn lr_ws_pop(handle: u32, dst: *mut u8, cap: u32) -> i32;
        /// Sends bytes (binary frame) or UTF-8 text (`text` != 0). 1 when sent, 0 when the socket is not open.
        pub fn lr_ws_send(handle: u32, ptr: *const u8, len: u32, text: i32) -> i32;
        /// The socket's bufferedAmount in bytes (what is queued and not yet written).
        pub fn lr_ws_buffered(handle: u32) -> i32;
        pub fn lr_ws_close(handle: u32, code: u32, reason: *const u8, reason_len: u32);
        /// A synchronous request. Returns a result id (>0); status 0 in the result means no answer.
        pub fn lr_http_send(
            method: *const u8,
            method_len: u32,
            url: *const u8,
            url_len: u32,
            headers: *const u8,
            headers_len: u32,
            body: *const u8,
            body_len: u32,
            has_body: i32,
        ) -> u32;
        pub fn lr_http_status(id: u32) -> i32;
        pub fn lr_http_headers(id: u32, dst: *mut u8, cap: u32) -> i32;
        pub fn lr_http_body(id: u32, dst: *mut u8, cap: u32) -> i32;
        pub fn lr_http_free(id: u32);
    }
}

/// Reads a page blob: the first try uses a 64 KB buffer, a longer blob is read again at its own length.
#[cfg(target_arch = "wasm32")]
fn read_blob(get: impl Fn(*mut u8, u32) -> i32) -> Vec<u8> {
    let mut cap = 64 * 1024usize;
    for _ in 0..2 {
        let mut buf = vec![0u8; cap];
        let n = get(buf.as_mut_ptr(), cap as u32);
        if n < 0 {
            return Vec::new();
        }
        let n = n as usize;
        if n <= cap {
            buf.truncate(n);
            return buf;
        }
        cap = n;
    }
    Vec::new()
}

/// The API origin the directory calls go to (`https://host` on a deployed page). Empty when the page does not say.
#[cfg(target_arch = "wasm32")]
pub fn api_base() -> String {
    let bytes = read_blob(|p, c| unsafe { ffi::lr_api_base(p, c) });
    String::from_utf8(bytes).unwrap_or_default().trim_end_matches('/').to_string()
}

/// Opens a relay WebSocket. `None` when the page refused to open it.
#[cfg(target_arch = "wasm32")]
pub fn ws_open(url: &str, hello: &str) -> Option<u32> {
    // SAFETY: both slices are live for the whole call and their lengths are passed with them.
    let handle = unsafe { ffi::lr_ws_open(url.as_ptr(), url.len() as u32, hello.as_ptr(), hello.len() as u32) };
    (handle != 0).then_some(handle)
}

/// The oldest queued message of a socket, if any.
#[cfg(target_arch = "wasm32")]
pub fn ws_next(handle: u32) -> Option<WsItem> {
    // SAFETY: plain calls; the buffer is `len` bytes long for the pop.
    let kind = unsafe { ffi::lr_ws_peek_kind(handle) };
    if kind <= 0 {
        return None;
    }
    let len = unsafe { ffi::lr_ws_peek_len(handle) }.max(0) as usize;
    let mut buf = vec![0u8; len];
    let n = unsafe { ffi::lr_ws_pop(handle, buf.as_mut_ptr(), len as u32) };
    if n < 0 {
        return None;
    }
    buf.truncate(n as usize);
    Some(match kind {
        1 => WsItem::Text(buf),
        2 => WsItem::Binary(buf),
        _ => {
            // the page writes a closed item as "CODE" newline "REASON"
            let text = String::from_utf8_lossy(&buf).to_string();
            let (code, reason) = text.split_once('\n').unwrap_or((text.as_str(), ""));
            WsItem::Closed { code: code.trim().parse().unwrap_or(1006), reason: reason.to_string() }
        }
    })
}

/// Sends one binary frame. False when the socket is not open.
#[cfg(target_arch = "wasm32")]
pub fn ws_send(handle: u32, bytes: &[u8]) -> bool {
    // SAFETY: the slice is live for the whole call and its length is passed with it.
    unsafe { ffi::lr_ws_send(handle, bytes.as_ptr(), bytes.len() as u32, 0) == 1 }
}

/// Sends one text message. False when the socket is not open.
#[cfg(target_arch = "wasm32")]
pub fn ws_send_text(handle: u32, text: &str) -> bool {
    // SAFETY: as in ws_send.
    unsafe { ffi::lr_ws_send(handle, text.as_ptr(), text.len() as u32, 1) == 1 }
}

/// Bytes queued in the page and not yet written.
#[cfg(target_arch = "wasm32")]
pub fn ws_buffered(handle: u32) -> u32 {
    unsafe { ffi::lr_ws_buffered(handle) }.max(0) as u32
}

#[cfg(target_arch = "wasm32")]
pub fn ws_close(handle: u32, code: u16, reason: &str) {
    // SAFETY: as in ws_send.
    unsafe { ffi::lr_ws_close(handle, code as u32, reason.as_ptr(), reason.len() as u32) }
}

/// One synchronous HTTP request through the page (`XMLHttpRequest`, bytes both ways).
#[cfg(target_arch = "wasm32")]
pub fn http_request(method: &str, url: &str, headers: &[(String, String)], body: Option<&[u8]>) -> WebReply {
    let header_text: String = headers.iter().map(|(k, v)| format!("{k}: {v}\n")).collect();
    let empty: &[u8] = &[];
    let body_bytes = body.unwrap_or(empty);
    // SAFETY: every slice is live for the whole call and its length is passed with it.
    let id = unsafe {
        ffi::lr_http_send(
            method.as_ptr(),
            method.len() as u32,
            url.as_ptr(),
            url.len() as u32,
            header_text.as_ptr(),
            header_text.len() as u32,
            body_bytes.as_ptr(),
            body_bytes.len() as u32,
            body.is_some() as i32,
        )
    };
    if id == 0 {
        return WebReply { status: 0, headers: String::new(), body: Vec::new() };
    }
    let status = unsafe { ffi::lr_http_status(id) }.max(0) as u16;
    let headers = String::from_utf8_lossy(&read_blob(|p, c| unsafe { ffi::lr_http_headers(id, p, c) })).to_string();
    let body = read_blob(|p, c| unsafe { ffi::lr_http_body(id, p, c) });
    unsafe { ffi::lr_http_free(id) };
    WebReply { status, headers, body }
}

// ---- native: there is no page ----

#[cfg(not(target_arch = "wasm32"))]
pub fn api_base() -> String {
    String::new()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ws_open(_url: &str, _hello: &str) -> Option<u32> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ws_next(_handle: u32) -> Option<WsItem> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ws_send(_handle: u32, _bytes: &[u8]) -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ws_send_text(_handle: u32, _text: &str) -> bool {
    false
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ws_buffered(_handle: u32) -> u32 {
    0
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ws_close(_handle: u32, _code: u16, _reason: &str) {}

#[cfg(not(target_arch = "wasm32"))]
pub fn http_request(_method: &str, _url: &str, _headers: &[(String, String)], _body: Option<&[u8]>) -> WebReply {
    WebReply { status: 0, headers: String::new(), body: Vec::new() }
}
