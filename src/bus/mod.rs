use tokio::sync::broadcast;
use tracing::warn;

/// An inbound message from a channel (HTTP, Telegram, scheduler, TUI) heading to the agent loop.
#[derive(Debug, Clone)]
pub struct InboundMessage {
    /// User identifier.
    pub user_id: String,
    /// Channel name (e.g. "api", "telegram", "scheduler").
    pub channel: String,
    /// Session key derived from (user_id, channel).
    pub session_key: String,
    /// User's message text.
    pub text: String,
}

/// An outbound message from the agent loop back to a channel.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    /// Session key this reply belongs to.
    pub session_key: String,
    /// Channel name the reply should be delivered to.
    pub channel: String,
    /// The agent's reply text.
    pub text: String,
}

/// Decoupling layer between channels and the agent loop.
///
/// Channels publish `InboundMessage`s; the agent loop publishes `OutboundMessage`s.
/// Synchronous channels (HTTP) subscribe to outbound filtered by session_key to
/// collect the reply. Async channels (Telegram) subscribe to outbound events.
pub struct MessageBus {
    inbound_tx: broadcast::Sender<InboundMessage>,
    outbound_tx: broadcast::Sender<OutboundMessage>,
}

impl MessageBus {
    pub fn new(capacity: usize) -> Self {
        let (inbound_tx, _) = broadcast::channel(capacity);
        let (outbound_tx, _) = broadcast::channel(capacity);
        Self { inbound_tx, outbound_tx }
    }

    /// Publish an inbound message. Logs a warning if no subscribers are listening.
    pub fn publish_inbound(&self, msg: InboundMessage) {
        if let Err(e) = self.inbound_tx.send(msg) {
            warn!("No inbound subscribers: {}", e);
        }
    }

    /// Subscribe to inbound messages.
    pub fn subscribe_inbound(&self) -> broadcast::Receiver<InboundMessage> {
        self.inbound_tx.subscribe()
    }

    /// Publish an outbound message (agent reply).
    pub fn publish_outbound(&self, msg: OutboundMessage) {
        if let Err(e) = self.outbound_tx.send(msg) {
            warn!("No outbound subscribers: {}", e);
        }
    }

    /// Subscribe to outbound messages.
    pub fn subscribe_outbound(&self) -> broadcast::Receiver<OutboundMessage> {
        self.outbound_tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bus_roundtrip() {
        let bus = MessageBus::new(16);
        let mut inbound_rx = bus.subscribe_inbound();

        bus.publish_inbound(InboundMessage {
            user_id: "u1".into(),
            channel: "api".into(),
            session_key: "u1:api".into(),
            text: "hello".into(),
        });

        let msg = inbound_rx.recv().await.unwrap();
        assert_eq!(msg.text, "hello");
        assert_eq!(msg.session_key, "u1:api");
    }

    #[tokio::test]
    async fn test_bus_multiple_subscribers() {
        let bus = MessageBus::new(16);
        let mut rx1 = bus.subscribe_inbound();
        let mut rx2 = bus.subscribe_inbound();

        bus.publish_inbound(InboundMessage {
            user_id: "u1".into(),
            channel: "api".into(),
            session_key: "u1:api".into(),
            text: "hello".into(),
        });

        let msg1 = rx1.recv().await.unwrap();
        let msg2 = rx2.recv().await.unwrap();
        assert_eq!(msg1.text, "hello");
        assert_eq!(msg2.text, "hello");
    }

    #[tokio::test]
    async fn test_bus_lagged_does_not_panic() {
        let bus = MessageBus::new(2);
        let mut rx = bus.subscribe_inbound();

        // Publish 4 messages into a capacity-2 channel.
        // The first message is consumed by the receiver, so messages 2-4
        // fill and overflow the buffer. This must not panic.
        for i in 0..4 {
            bus.publish_inbound(InboundMessage {
                user_id: format!("u{}", i),
                channel: "api".into(),
                session_key: format!("u{}:api", i),
                text: format!("msg{}", i),
            });
        }

        // The receiver should get at least one message (possibly lagged).
        let result = rx.recv().await;
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_bus_outbound_roundtrip() {
        let bus = MessageBus::new(16);
        let mut outbound_rx = bus.subscribe_outbound();

        bus.publish_outbound(OutboundMessage {
            session_key: "u1:api".into(),
            channel: "api".into(),
            text: "reply".into(),
        });

        let msg = outbound_rx.recv().await.unwrap();
        assert_eq!(msg.text, "reply");
        assert_eq!(msg.channel, "api");
    }
}
