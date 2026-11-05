//! Platform-independent validation and shared latest-text storage.
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

/// Limit UTF-16 source bytes (excluding NUL) and stored UTF-8 to 1 MiB each.
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum TextError {
    TooLarge,
    MissingTerminator,
    InvalidUtf16,
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "clipboard text exceeds the 1 MiB limit",
            Self::MissingTerminator => "clipboard text has no NUL terminator",
            Self::InvalidUtf16 => "clipboard text contains invalid UTF-16",
        })
    }
}

pub fn decode_text(units: &[u16]) -> Result<String, TextError> {
    let end = units
        .iter()
        .take(MAX_TEXT_BYTES / 2 + 1)
        .position(|&u| u == 0);
    let end = match end {
        Some(end) => end,
        None if units.len() > MAX_TEXT_BYTES / 2 => return Err(TextError::TooLarge),
        None => return Err(TextError::MissingTerminator),
    };
    // Preflight UTF-8 size before allocating the output String.
    let mut bytes = 0;
    for ch in char::decode_utf16(units[..end].iter().copied()) {
        bytes += ch.map_err(|_| TextError::InvalidUtf16)?.len_utf8();
        if bytes > MAX_TEXT_BYTES {
            return Err(TextError::TooLarge);
        }
    }
    String::from_utf16(&units[..end]).map_err(|_| TextError::InvalidUtf16)
}

/// Process-session scoped identity; intentionally not a durable device/wire ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionId {
    pub started_at: SystemTime,
    pub process_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    WindowsClipboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationId {
    pub session: SessionId,
    pub observation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    NotRead,
    Pending,
    TemporaryFailure,
    UnsupportedFormat,
    MalformedData,
    Oversized,
    RetriesExhausted,
    SequenceUnknown,
    ChangedDuringRead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardText {
    pub text: String,
    pub character_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardState {
    Empty,
    Available(ClipboardText),
    Unavailable(UnavailableReason),
}

#[derive(Clone, Debug)]
pub struct ClipboardSnapshot {
    pub state: ClipboardState,
    pub updated_at: SystemTime,
    /// Local revision of this store, not an ordering of Windows sequence numbers.
    pub version: u64,
    /// None means Windows returned zero (sequence unknown).
    pub sequence: Option<u32>,
    pub origin: Origin,
    pub operation_id: OperationId,
}

struct Inner {
    snapshot: ClipboardSnapshot,
    // Comparison only: never returned by snapshot() or used as readable fallback.
    last_successful_text: Option<String>,
}

/// Owned point-in-time snapshots; callers never receive a lock or mutation guard.
#[derive(Clone)]
pub struct ClipboardStore {
    inner: Arc<RwLock<Inner>>,
}

/// A store-issued token. Superseded tokens cannot publish or invalidate state.
pub struct Observation {
    operation_id: OperationId,
    sequence: Option<u32>,
}

impl Observation {
    pub fn sequence(&self) -> Option<u32> {
        self.sequence
    }
}

pub enum ReadValue {
    Empty,
    Text(String),
    Unavailable(UnavailableReason),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Update {
    Changed,
    Duplicate,
    Superseded,
}

impl Default for ClipboardStore {
    fn default() -> Self {
        let now = SystemTime::now();
        Self {
            inner: Arc::new(RwLock::new(Inner {
                last_successful_text: None,
                snapshot: ClipboardSnapshot {
                    state: ClipboardState::Unavailable(UnavailableReason::NotRead),
                    updated_at: now,
                    version: 0,
                    sequence: None,
                    origin: Origin::WindowsClipboard,
                    operation_id: OperationId {
                        session: SessionId {
                            started_at: now,
                            process_id: std::process::id(),
                        },
                        observation: 0,
                    },
                },
            })),
        }
    }
}

impl ClipboardStore {
    pub fn snapshot(&self) -> ClipboardSnapshot {
        self.inner
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .snapshot
            .clone()
    }

    /// Invalidate before accessing Win32, including the initial startup read.
    /// Sequence numbers are equality-only tokens: never numerically ordered.
    pub fn begin_observation(&self, sequence: Option<u32>) -> Observation {
        let mut inner = self.inner.write().unwrap_or_else(|p| p.into_inner());
        let snapshot = &mut inner.snapshot;
        snapshot.state = ClipboardState::Unavailable(UnavailableReason::Pending);
        snapshot.version += 1;
        snapshot.operation_id.observation += 1;
        snapshot.updated_at = SystemTime::now();
        snapshot.sequence = sequence;
        Observation {
            operation_id: snapshot.operation_id,
            sequence,
        }
    }

    /// Every outcome is published through this validation boundary. Oversized
    /// updates invalidate the current observation instead of retaining old text.
    pub fn complete(
        &self,
        observation: &mut Observation,
        value: ReadValue,
    ) -> Result<Update, TextError> {
        let mut inner = self.inner.write().unwrap_or_else(|p| p.into_inner());
        let Inner {
            snapshot,
            last_successful_text,
        } = &mut *inner;
        if snapshot.operation_id != observation.operation_id {
            return Ok(Update::Superseded);
        }
        let mut validation_error = None;
        let state = match value {
            ReadValue::Empty => ClipboardState::Empty,
            ReadValue::Unavailable(reason) => ClipboardState::Unavailable(reason),
            ReadValue::Text(text) => {
                if text.len() > MAX_TEXT_BYTES || text.encode_utf16().count() > MAX_TEXT_BYTES / 2 {
                    validation_error = Some(TextError::TooLarge);
                    ClipboardState::Unavailable(UnavailableReason::Oversized)
                } else {
                    ClipboardState::Available(ClipboardText {
                        character_count: text.chars().count(),
                        text,
                    })
                }
            }
        };
        let duplicate = match &state {
            ClipboardState::Available(text) => last_successful_text.as_ref() == Some(&text.text),
            _ => snapshot.state == state,
        };
        match &state {
            ClipboardState::Available(text) => {
                if !duplicate {
                    *last_successful_text = Some(text.text.clone());
                }
            }
            // Uncertainty invalidates readable state, but says nothing about
            // whether the clipboard's text actually changed. Keep comparison
            // history across retries and observation/sequence restarts.
            ClipboardState::Unavailable(
                UnavailableReason::Pending
                | UnavailableReason::TemporaryFailure
                | UnavailableReason::SequenceUnknown
                | UnavailableReason::ChangedDuringRead,
            ) => {}
            // Settled non-text/rejected outcomes (and exhausted retries) break
            // the run of successful text copies and release the private text.
            _ => *last_successful_text = None,
        }
        snapshot.state = state;
        snapshot.updated_at = SystemTime::now();
        snapshot.version += 1;
        if let Some(error) = validation_error {
            return Err(error);
        }
        Ok(if duplicate {
            Update::Duplicate
        } else {
            Update::Changed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(store: &ClipboardStore, seq: u32, value: ReadValue) -> Result<Update, TextError> {
        let mut observation = store.begin_observation(Some(seq));
        store.complete(&mut observation, value)
    }

    fn text(store: &ClipboardStore, seq: u32, value: &str) -> Update {
        publish(store, seq, ReadValue::Text(value.into())).unwrap()
    }

    #[test]
    fn identical_text_after_temporary_failures_is_duplicate() {
        let store = ClipboardStore::default();
        assert_eq!(text(&store, 1, "same"), Update::Changed);
        for (sequence, failures) in [(2, 1), (3, 3), (4, 6)] {
            let mut observation = store.begin_observation(Some(sequence));
            let started = store.snapshot();
            let mut version = started.version;
            for _ in 0..failures {
                store
                    .complete(
                        &mut observation,
                        ReadValue::Unavailable(UnavailableReason::TemporaryFailure),
                    )
                    .unwrap();
                let snapshot = store.snapshot();
                assert_eq!(
                    snapshot.state,
                    ClipboardState::Unavailable(UnavailableReason::TemporaryFailure)
                );
                assert!(snapshot.version > version);
                version = snapshot.version;
            }
            assert_eq!(
                store
                    .complete(&mut observation, ReadValue::Text("same".into()))
                    .unwrap(),
                Update::Duplicate
            );
            let recovered = store.snapshot();
            assert!(
                matches!(recovered.state, ClipboardState::Available(value) if value.text == "same")
            );
            assert!(recovered.version > version);
            assert_eq!(recovered.sequence, Some(sequence));
            assert_eq!(recovered.operation_id, started.operation_id);
        }
    }

    #[test]
    fn deduplicates_consecutive_text_and_shares_latest() {
        let store = ClipboardStore::default();
        let reader = store.clone();
        assert_eq!(text(&store, 1, "hello"), Update::Changed);
        assert_eq!(text(&store, 2, "hello"), Update::Duplicate);
        assert_eq!(text(&store, 3, "other"), Update::Changed);
        assert_eq!(text(&store, 4, "hello"), Update::Changed);
        assert!(
            matches!(reader.snapshot().state, ClipboardState::Available(value) if value.text == "hello")
        );
        assert_eq!(reader.snapshot().sequence, Some(4));
    }

    #[test]
    fn empty_and_unicode_text() {
        let store = ClipboardStore::default();
        assert_eq!(decode_text(&[0]).unwrap(), "");
        // An actual zero-length Unicode payload remains Available, distinct from no formats.
        assert_eq!(text(&store, 1, ""), Update::Changed);
        assert_eq!(text(&store, 2, ""), Update::Duplicate);
        let value = "A😀é";
        let units: Vec<_> = value.encode_utf16().chain([0]).collect();
        assert_eq!(decode_text(&units).unwrap(), value);
        text(&store, 3, value);
        assert!(
            matches!(store.snapshot().state, ClipboardState::Available(value) if value.character_count == 3)
        );
    }

    #[test]
    fn validates_termination_encoding_and_size_limits() {
        assert_eq!(decode_text(&[65]), Err(TextError::MissingTerminator));
        assert_eq!(decode_text(&[0xd800, 0]), Err(TextError::InvalidUtf16));
        let mut units = vec![65; MAX_TEXT_BYTES / 2];
        units.push(0);
        assert!(decode_text(&units).is_ok());
        units.insert(0, 65);
        assert_eq!(decode_text(&units), Err(TextError::TooLarge));
        let wide = "界".repeat(MAX_TEXT_BYTES / 3 + 1);
        let encoded: Vec<_> = wide.encode_utf16().chain([0]).collect();
        assert_eq!(decode_text(&encoded), Err(TextError::TooLarge));
        let store = ClipboardStore::default();
        text(&store, 1, "old");
        assert_eq!(
            publish(&store, 2, ReadValue::Text(wide)),
            Err(TextError::TooLarge)
        );
        assert_eq!(
            store.snapshot().state,
            ClipboardState::Unavailable(UnavailableReason::Oversized)
        );
        assert_eq!(text(&store, 3, "old"), Update::Changed);
        assert_eq!(
            publish(
                &store,
                4,
                ReadValue::Text("a".repeat(MAX_TEXT_BYTES / 2 + 1))
            ),
            Err(TextError::TooLarge)
        );
        assert_eq!(
            store.snapshot().state,
            ClipboardState::Unavailable(UnavailableReason::Oversized)
        );
    }

    #[test]
    fn settled_non_text_outcomes_remove_readable_text_and_reset_dedup() {
        let store = ClipboardStore::default();
        let outcomes = [
            ReadValue::Empty,
            ReadValue::Unavailable(UnavailableReason::UnsupportedFormat),
            ReadValue::Unavailable(UnavailableReason::MalformedData),
            ReadValue::Unavailable(UnavailableReason::RetriesExhausted),
            ReadValue::Unavailable(UnavailableReason::Oversized),
        ];
        for (index, outcome) in outcomes.into_iter().enumerate() {
            text(&store, index as u32, "same");
            publish(&store, index as u32, outcome).unwrap();
            assert!(!matches!(
                store.snapshot().state,
                ClipboardState::Available(_)
            ));
            assert_eq!(text(&store, index as u32, "same"), Update::Changed);
        }
    }

    #[test]
    fn startup_is_unknown_then_pending_then_available_or_empty() {
        let store = ClipboardStore::default();
        assert_eq!(
            store.snapshot().state,
            ClipboardState::Unavailable(UnavailableReason::NotRead)
        );
        let mut initial = store.begin_observation(Some(10));
        assert_eq!(
            store.snapshot().state,
            ClipboardState::Unavailable(UnavailableReason::Pending)
        );
        store
            .complete(&mut initial, ReadValue::Text("preexisting".into()))
            .unwrap();
        assert_eq!(text(&store, 10, "preexisting"), Update::Duplicate);
        let empty = ClipboardStore::default();
        publish(&empty, 11, ReadValue::Empty).unwrap();
        assert_eq!(empty.snapshot().state, ClipboardState::Empty);
    }

    #[test]
    fn superseded_retries_cannot_publish_invalidate_or_reject_newer_state() {
        let store = ClipboardStore::default();
        let mut old = store.begin_observation(Some(u32::MAX));
        let old_id = store.snapshot().operation_id;
        let mut new = store.begin_observation(Some(1)); // Windows counter wrap.
        store
            .complete(&mut new, ReadValue::Text("new".into()))
            .unwrap();
        let current = store.snapshot();
        for outcome in [
            ReadValue::Text("old".into()),
            ReadValue::Unavailable(UnavailableReason::RetriesExhausted),
            ReadValue::Text("a".repeat(MAX_TEXT_BYTES)),
        ] {
            assert_eq!(
                store.complete(&mut old, outcome).unwrap(),
                Update::Superseded
            );
            assert_eq!(store.snapshot().version, current.version);
            assert_eq!(store.snapshot().state, current.state);
        }
        assert_ne!(old_id, current.operation_id);
        assert_eq!(old_id.session, current.operation_id.session);
        assert_eq!(current.origin, Origin::WindowsClipboard);
        assert_eq!(text(&store, 2, "new"), Update::Duplicate);
    }

    #[test]
    fn pending_and_failed_retry_do_not_expose_previous_text() {
        let store = ClipboardStore::default();
        text(&store, 1, "secret");
        let before = store.snapshot().version;
        let mut retry = store.begin_observation(Some(2));
        assert!(store.snapshot().version > before);
        assert_eq!(
            store.snapshot().state,
            ClipboardState::Unavailable(UnavailableReason::Pending)
        );
        store
            .complete(
                &mut retry,
                ReadValue::Unavailable(UnavailableReason::TemporaryFailure),
            )
            .unwrap();
        assert_eq!(
            store
                .complete(&mut retry, ReadValue::Text("secret".into()))
                .unwrap(),
            Update::Duplicate
        );
    }

    #[test]
    fn genuine_text_change_after_failed_retries_is_changed() {
        let store = ClipboardStore::default();
        text(&store, 1, "before");
        let mut observation = store.begin_observation(Some(2));
        for _ in 0..3 {
            store
                .complete(
                    &mut observation,
                    ReadValue::Unavailable(UnavailableReason::TemporaryFailure),
                )
                .unwrap();
        }
        assert_eq!(
            store
                .complete(&mut observation, ReadValue::Text("after".into()))
                .unwrap(),
            Update::Changed
        );
        assert_eq!(text(&store, 3, "after"), Update::Duplicate);
        assert_eq!(text(&store, 4, "before"), Update::Changed);
    }

    #[test]
    fn uncertainty_and_new_observations_preserve_only_private_comparison_history() {
        let store = ClipboardStore::default();
        text(&store, 1, "same");
        for reason in [
            UnavailableReason::TemporaryFailure,
            UnavailableReason::SequenceUnknown,
            UnavailableReason::ChangedDuringRead,
        ] {
            publish(&store, 2, ReadValue::Unavailable(reason)).unwrap();
            assert_eq!(store.snapshot().state, ClipboardState::Unavailable(reason));
        }
        let mut restarted = store.begin_observation(Some(3));
        assert_eq!(
            store.snapshot().state,
            ClipboardState::Unavailable(UnavailableReason::Pending)
        );
        assert_eq!(
            store
                .complete(&mut restarted, ReadValue::Text("same".into()))
                .unwrap(),
            Update::Duplicate
        );
    }
}
