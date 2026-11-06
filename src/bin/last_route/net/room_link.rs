//! The transport seam under [`super::session::CoopSession`]: one relay connection, host or guest, whatever carries it.
//!
//! Three implementations exist:
//! * [`super::relay::RelayLink`]: the native UDP LRR1 relay (unchanged).
//! * [`super::ws_link::WsLink`]: the browser build's WebSocket relay (`/api/ws`, the same hub the Godot web build
//!   joins), reached through the page's WebSocket API in dist/web/index.html.
//! * [`super::ws_native::NativeWsLink`]: the same WebSocket relay from a native process (the dedicated room host, native guests).
//!
//! All report through [`RelayEvent`] and carry opaque session datagrams, so the co-op layer above does not care
//! which one it has.

use super::relay::{RelayEvent, Role};

/// `Send`: a session is built on a request thread (native) and then handed to the frame loop.
pub trait RoomLink: Send {
    /// Host or guest.
    fn role(&self) -> Role;
    /// True once the relay accepted this connection (HOST_ACK / welcome).
    fn ready(&self) -> bool;
    /// The largest payload one [`RoomLink::send`] carries: a UDP relay datagram holds [`super::relay::MAX_PAYLOAD_BYTES`], a
    /// WebSocket frame up to 65535 bytes. The co-op layer sizes its messages by it (a big hunter frame is not split).
    fn max_payload(&self) -> usize {
        super::relay::MAX_PAYLOAD_BYTES
    }
    /// False once the connection is over for good (the socket closed, the room ended or the relay refused us). A link
    /// that cannot tell says true. A host process uses it to decide to reconnect.
    fn alive(&self) -> bool {
        true
    }
    /// A guest's relay session (or welcome id); 0 for a host.
    fn session(&self) -> u32;
    /// The room code.
    fn code(&self) -> &str;
    /// Host: to guest `session`. Guest: to the host (`session` is ignored). False when it was not sent.
    fn send(&self, session: u32, payload: &[u8]) -> bool;
    /// Host: removes guest `session` from the room (the WebSocket relay closes its socket with 4005; the guest hears
    /// YOU WERE REMOVED FROM THE ROOM). False when this link cannot, or the request was not sent.
    fn kick(&self, _session: u32) -> bool {
        false
    }
    /// Drains what arrived since the last call and keeps the connection alive.
    fn poll(&mut self) -> Vec<RelayEvent>;
    /// The host token changed (a heartbeat rotated it).
    fn update_host_token(&mut self, token: &str);
    /// Ends the connection.
    fn close(&mut self);
}

/// A boxed link is a link (the co-op layer holds one as `Box<dyn RoomLink>`).
impl<T: RoomLink + ?Sized> RoomLink for Box<T> {
    fn role(&self) -> Role {
        (**self).role()
    }
    fn ready(&self) -> bool {
        (**self).ready()
    }
    fn alive(&self) -> bool {
        (**self).alive()
    }
    fn max_payload(&self) -> usize {
        (**self).max_payload()
    }
    fn session(&self) -> u32 {
        (**self).session()
    }
    fn code(&self) -> &str {
        (**self).code()
    }
    fn send(&self, session: u32, payload: &[u8]) -> bool {
        (**self).send(session, payload)
    }
    fn kick(&self, session: u32) -> bool {
        (**self).kick(session)
    }
    fn poll(&mut self) -> Vec<RelayEvent> {
        (**self).poll()
    }
    fn update_host_token(&mut self, token: &str) {
        (**self).update_host_token(token)
    }
    fn close(&mut self) {
        (**self).close()
    }
}

impl RoomLink for super::relay::RelayLink {
    fn role(&self) -> Role {
        super::relay::RelayLink::role(self)
    }
    fn ready(&self) -> bool {
        super::relay::RelayLink::ready(self)
    }
    fn session(&self) -> u32 {
        super::relay::RelayLink::session(self)
    }
    fn code(&self) -> &str {
        super::relay::RelayLink::code(self)
    }
    fn send(&self, session: u32, payload: &[u8]) -> bool {
        super::relay::RelayLink::send(self, session, payload)
    }
    fn poll(&mut self) -> Vec<RelayEvent> {
        super::relay::RelayLink::poll(self)
    }
    fn update_host_token(&mut self, token: &str) {
        super::relay::RelayLink::update_host_token(self, token)
    }
    fn close(&mut self) {
        super::relay::RelayLink::close(self)
    }
}
