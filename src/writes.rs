//! Bounded command/ack bridge. Commands own text/image buffers, never Win32 handles.
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

#[derive(Clone, Copy, Debug, Serialize)]
pub struct WriteReceipt {
    pub operation: u64,
    pub sequence: Option<u32>,
}

/// Shared admission checks prevent queued work outliving pause/network authority.
#[derive(Clone)]
pub(crate) struct RequestGuard {
    sharing: crate::sharing::SharingControl,
    generation: u64,
    gate: Option<(crate::tailnet::Gate, u64)>,
}
impl RequestGuard {
    pub fn new(
        sharing: crate::sharing::SharingControl,
        gate: Option<crate::tailnet::Gate>,
    ) -> Self {
        let generation = sharing.generation();
        Self {
            sharing,
            generation,
            gate: gate.map(|g| {
                let epoch = g.generation();
                (g, epoch)
            }),
        }
    }
    pub fn valid(&self) -> bool {
        !self.sharing.is_paused()
            && self.sharing.generation() == self.generation
            && self.gate.as_ref().is_none_or(|(g, epoch)| g.valid(*epoch))
    }
    pub async fn invalidated(&self) {
        tokio::select! {biased;
            _=self.sharing.paused_since(self.generation)=>{},
            _=async {if let Some((g,epoch))=&self.gate{g.invalidated_since(*epoch).await}else{std::future::pending::<()>().await}}=>{},
        }
    }
}

pub struct WriteCommand {
    pub text: String,
    pub image: Option<crate::image::Prepared>,
    _image_lease: Option<tokio::sync::OwnedSemaphorePermit>,
    /// Local setup export only; never selected by an HTTP request.
    pub private: bool,
    pub deadline: Instant,
    pub ack: oneshot::Sender<Result<WriteReceipt, WriteError>>,
    cancelled: Arc<AtomicBool>,
    guard: Option<RequestGuard>,
}
impl WriteCommand {
    pub fn expired(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self.ack.is_closed()
            || Instant::now() >= self.deadline
            || self.guard.as_ref().is_some_and(|g| !g.valid())
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
    guard: Option<RequestGuard>,
}

impl WriteClient {
    pub fn new(sender: mpsc::Sender<WriteCommand>, wake: fn() -> bool) -> Self {
        Self {
            sender,
            wake,
            timeout: WRITE_TIMEOUT,
            guard: None,
        }
    }

    pub async fn write(&self, text: String) -> Result<WriteReceipt, WriteError> {
        self.submit(text, false, None, None).await
    }
    pub(crate) fn guarded(&self, guard: RequestGuard) -> Self {
        Self {
            guard: Some(guard),
            ..self.clone()
        }
    }

    pub(crate) async fn copy_private(&self, text: String) -> Result<WriteReceipt, WriteError> {
        self.submit(text, true, None, None).await
    }
    pub async fn write_image(
        &self,
        image: crate::image::Prepared,
        lease: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<WriteReceipt, WriteError> {
        self.submit(String::new(), false, Some(image), Some(lease))
            .await
    }

    async fn submit(
        &self,
        text: String,
        private: bool,
        image: Option<crate::image::Prepared>,
        lease: Option<tokio::sync::OwnedSemaphorePermit>,
    ) -> Result<WriteReceipt, WriteError> {
        if self.guard.as_ref().is_some_and(|g| !g.valid()) {
            return Err(WriteError::Unavailable);
        }
        validate_text(&text)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let _cancel = CancelOnDrop(cancelled.clone());
        let (ack, response) = oneshot::channel();
        let command = WriteCommand {
            text,
            image,
            _image_lease: lease,
            private,
            deadline: Instant::now() + self.timeout,
            ack,
            cancelled,
            guard: self.guard.clone(),
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
    async fn private_setup_export_is_explicit_and_uses_the_same_bounded_ack_queue() {
        let (sender, mut receiver) = mpsc::channel(QUEUE_CAPACITY);
        let client = WriteClient::new(sender, || true);
        for private in [false, true] {
            let task = tokio::spawn({
                let client = client.clone();
                async move {
                    if private {
                        client.copy_private("disposable export".into()).await
                    } else {
                        client.write("disposable export".into()).await
                    }
                }
            });
            let command = receiver.recv().await.unwrap();
            assert_eq!(command.private, private);
            assert!(!task.is_finished());
            command.ack.send(Err(WriteError::Failed)).ok();
            assert!(matches!(task.await.unwrap(), Err(WriteError::Failed)));
        }
    }

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
            guard: None,
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
                    image: None,
                    _image_lease: None,
                    private: false,
                    deadline: Instant::now() + WRITE_TIMEOUT,
                    ack,
                    cancelled: Arc::new(AtomicBool::new(false)),
                    guard: None,
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
