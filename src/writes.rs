//! Bounded command/ack bridge. Commands contain owned Rust text, never Win32 handles.
use crate::clipboard::MAX_TEXT_BYTES;
use serde::Serialize;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub const QUEUE_CAPACITY: usize = 4;
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(3);
pub const MAX_JSON_BYTES: usize = MAX_TEXT_BYTES * 6 + 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteError {
    Invalid,
    TooLarge,
    Busy,
    Failed,
    Timeout,
    Unavailable,
}

pub fn validate_text(text: &str) -> Result<(), WriteError> {
    if text.len() > MAX_TEXT_BYTES || text.encode_utf16().count() > MAX_TEXT_BYTES / 2 {
        return Err(WriteError::TooLarge);
    }
    if text.contains('\0') {
        return Err(WriteError::Invalid);
    }
    Ok(())
}

#[derive(Clone, Copy, Serialize)]
pub struct WriteReceipt {
    pub operation: u64,
    pub sequence: Option<u32>,
}

pub struct WriteCommand {
    pub text: String,
    pub deadline: Instant,
    pub ack: oneshot::Sender<Result<WriteReceipt, WriteError>>,
    cancelled: Arc<AtomicBool>,
}
impl WriteCommand {
    pub fn expired(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self.ack.is_closed()
            || Instant::now() >= self.deadline
    }
}

struct CancelOnDrop(Arc<AtomicBool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct WriteClient {
    sender: mpsc::Sender<WriteCommand>,
    wake: fn() -> bool,
    timeout: Duration,
}

impl WriteClient {
    pub fn new(sender: mpsc::Sender<WriteCommand>, wake: fn() -> bool) -> Self {
        Self {
            sender,
            wake,
            timeout: WRITE_TIMEOUT,
        }
    }

    pub async fn write(&self, text: String) -> Result<WriteReceipt, WriteError> {
        validate_text(&text)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(cancelled.clone());
        let (ack, response) = oneshot::channel();
        let command = WriteCommand {
            text,
            deadline: Instant::now() + self.timeout,
            ack,
            cancelled,
        };
        self.sender.try_send(command).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => WriteError::Busy,
            mpsc::error::TrySendError::Closed(_) => WriteError::Unavailable,
        })?;
        if !(self.wake)() {
            return Err(WriteError::Unavailable);
        }
        match tokio::time::timeout(self.timeout, response).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(WriteError::Unavailable),
            Err(_) => Err(WriteError::Timeout),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn success_requires_acknowledgement_and_failures_propagate() {
        let (sender, mut receiver) = mpsc::channel(QUEUE_CAPACITY);
        let client = WriteClient::new(sender, || true);
        let request = tokio::spawn({
            let client = client.clone();
            async move { client.write("disposable".into()).await }
        });
        let command = receiver.recv().await.unwrap();
        assert!(!request.is_finished());
        command
            .ack
            .send(Ok(WriteReceipt {
                operation: 5,
                sequence: Some(7),
            }))
            .ok();
        let receipt = request.await.unwrap().unwrap();
        assert_eq!(receipt.operation, 5);
        assert_eq!(receipt.sequence, Some(7));
        for error in [
            WriteError::Busy,
            WriteError::Failed,
            WriteError::Timeout,
            WriteError::Unavailable,
        ] {
            let request = tokio::spawn({
                let client = client.clone();
                async move { client.write("disposable".into()).await }
            });
            receiver.recv().await.unwrap().ack.send(Err(error)).ok();
            assert!(matches!(request.await.unwrap(), Err(actual) if actual == error));
        }
    }

    #[tokio::test]
    async fn timeout_and_disconnected_handlers_cancel_queued_commands() {
        let (sender, mut receiver) = mpsc::channel(QUEUE_CAPACITY);
        let client = WriteClient {
            sender,
            wake: || true,
            timeout: Duration::from_millis(15),
        };
        assert!(matches!(
            client.write("disposable".into()).await,
            Err(WriteError::Timeout)
        ));
        assert!(receiver.recv().await.unwrap().expired());
        let request = tokio::spawn(async move { client.write("disposable".into()).await });
        let command = receiver.recv().await.unwrap();
        request.abort();
        let _ = request.await;
        assert!(command.expired());
    }

    #[tokio::test]
    async fn shutdown_and_full_queue_are_bounded() {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let client = WriteClient::new(sender.clone(), || true);
        for _ in 0..QUEUE_CAPACITY {
            let (ack, _) = oneshot::channel();
            sender
                .try_send(WriteCommand {
                    text: String::new(),
                    deadline: Instant::now() + WRITE_TIMEOUT,
                    ack,
                    cancelled: Arc::new(AtomicBool::new(false)),
                })
                .ok()
                .unwrap();
        }
        assert!(matches!(
            client.write(String::new()).await,
            Err(WriteError::Busy)
        ));
        drop(receiver);
        assert!(matches!(
            client.write(String::new()).await,
            Err(WriteError::Unavailable)
        ));
    }

    #[test]
    fn invalid_nul_and_both_size_limits_are_rejected() {
        assert_eq!(validate_text("a\0b"), Err(WriteError::Invalid));
        assert_eq!(
            validate_text(&"a".repeat(MAX_TEXT_BYTES / 2 + 1)),
            Err(WriteError::TooLarge)
        );
        assert_eq!(
            validate_text(&"界".repeat(MAX_TEXT_BYTES / 3 + 1)),
            Err(WriteError::TooLarge)
        );
        assert!(validate_text("😀\n\r\né").is_ok());
    }
}
