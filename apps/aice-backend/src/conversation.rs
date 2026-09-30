//! Short-term conversation context per session.
//!
//! Each turn used to reach the model alone, so "no, actually a unicorn" after
//! a dragon story meant nothing. The log keeps the last few exchanges of each
//! session in memory (never on disk) so chat replies see them. It is tied to
//! the session's memory scope: a new stay (another guest in the room) starts
//! with an empty log. It also empties after a quiet spell, when a new
//! conversation begins. When the guest talks over an answer, the answer is cut
//! where they interrupted and marked, so the model knows what was heard.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::memory_scope::MemoryScope;

/// A conversation ends after this long without a turn.
pub const CONVERSATION_IDLE: Duration = Duration::from_secs(600);

/// One request and the answer given to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    pub user: String,
    /// The answer, cut to what was spoken when the guest interrupted it.
    pub assistant: String,
    pub interrupted: bool,
}

struct Conversation {
    scope: MemoryScope,
    exchanges: VecDeque<Exchange>,
    last_at: Instant,
}

/// Recent exchanges per session (a room pod's device, a desktop client).
pub struct ConversationLog {
    max_exchanges: usize,
    idle: Duration,
    sessions: Mutex<HashMap<String, Conversation>>,
}

impl ConversationLog {
    pub fn new(max_exchanges: usize, idle: Duration) -> Self {
        Self {
            max_exchanges,
            idle,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The session's recent exchanges, oldest first. Empty for another scope
    /// (the room's stay changed) or after a quiet spell; either drops the old log.
    pub fn recent(&self, session: &str, scope: &MemoryScope, now: Instant) -> Vec<Exchange> {
        let Ok(mut sessions) = self.sessions.lock() else {
            return Vec::new();
        };
        let current = sessions.get(session).is_some_and(|c| {
            &c.scope == scope && now.saturating_duration_since(c.last_at) <= self.idle
        });
        if !current {
            sessions.remove(session);
            return Vec::new();
        }
        sessions
            .get(session)
            .map(|c| c.exchanges.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Adds an exchange; the oldest falls off beyond the limit.
    pub fn record(
        &self,
        session: &str,
        scope: &MemoryScope,
        user: &str,
        assistant: &str,
        now: Instant,
    ) {
        if self.max_exchanges == 0 {
            return;
        }
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let conversation = sessions
            .entry(session.to_string())
            .or_insert_with(|| Conversation {
                scope: scope.clone(),
                exchanges: VecDeque::new(),
                last_at: now,
            });
        let stale = now.saturating_duration_since(conversation.last_at) > self.idle;
        if &conversation.scope != scope || stale {
            conversation.scope = scope.clone();
            conversation.exchanges.clear();
        }
        conversation.exchanges.push_back(Exchange {
            user: user.trim().to_string(),
            assistant: assistant.trim().to_string(),
            interrupted: false,
        });
        while conversation.exchanges.len() > self.max_exchanges {
            conversation.exchanges.pop_front();
        }
        conversation.last_at = now;
    }

    /// The guest talked over the latest answer after `spoken_ratio` (0 to 1)
    /// of it was played: keep only the words that were heard.
    pub fn interrupted(&self, session: &str, spoken_ratio: f32) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let Some(latest) = sessions
            .get_mut(session)
            .and_then(|c| c.exchanges.back_mut())
        else {
            return;
        };
        latest.assistant = spoken_part(&latest.assistant, spoken_ratio);
        latest.interrupted = true;
    }
}

/// The first `ratio` of `text` by characters, cut back to the last whole word.
fn spoken_part(text: &str, ratio: f32) -> String {
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        1.0
    };
    let total = text.chars().count();
    let keep = (total as f32 * ratio).round() as usize;
    if keep >= total {
        return text.to_string();
    }
    let cut: String = text.chars().take(keep).collect();
    // Drop a word that was only partly spoken.
    let whole = match text.chars().nth(keep) {
        Some(next) if !next.is_whitespace() => cut.rsplit_once(' ').map_or("", |(head, _)| head),
        _ => cut.as_str(),
    };
    whole.trim_end().to_string()
}

/// Exchanges as (user, assistant) chat history for the model.
pub fn chat_history(exchanges: &[Exchange]) -> Vec<(String, String)> {
    exchanges
        .iter()
        .map(|exchange| {
            let assistant = if exchange.interrupted {
                format!("{} ... (the user interrupted here)", exchange.assistant)
            } else {
                exchange.assistant.clone()
            };
            (exchange.user.clone(), assistant)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{chat_history, ConversationLog};
    use crate::memory_scope::MemoryScope;
    use std::time::{Duration, Instant};

    const IDLE: Duration = Duration::from_secs(600);

    fn stay(id: &str) -> MemoryScope {
        MemoryScope::Stay(id.to_string())
    }

    #[test]
    fn the_last_exchanges_come_back_in_order() {
        let log = ConversationLog::new(2, IDLE);
        let now = Instant::now();
        log.record("pod-1", &stay("s1"), "hello", "Hi!", now);
        log.record(
            "pod-1",
            &stay("s1"),
            "tell me a story",
            "Once upon a time...",
            now,
        );
        log.record("pod-1", &stay("s1"), "another", "There was a fox.", now);
        let recent = log.recent("pod-1", &stay("s1"), now);
        let users: Vec<&str> = recent.iter().map(|e| e.user.as_str()).collect();
        assert_eq!(
            users,
            vec!["tell me a story", "another"],
            "keeps the newest two"
        );
    }

    #[test]
    fn a_new_guest_in_the_room_does_not_see_the_last_guests_conversation() {
        let log = ConversationLog::new(6, IDLE);
        let now = Instant::now();
        log.record("pod-1", &stay("s1"), "my name is Ann", "Hello Ann.", now);
        assert!(log.recent("pod-1", &stay("s2"), now).is_empty());
        assert!(
            log.recent("pod-1", &stay("s1"), now).is_empty(),
            "the old stay is gone too"
        );
    }

    #[test]
    fn other_rooms_have_their_own_conversation() {
        let log = ConversationLog::new(6, IDLE);
        let now = Instant::now();
        log.record("pod-1", &stay("s1"), "hello", "Hi!", now);
        assert!(log.recent("pod-2", &stay("s1"), now).is_empty());
    }

    #[test]
    fn a_quiet_spell_starts_a_new_conversation() {
        let log = ConversationLog::new(6, IDLE);
        let then = Instant::now();
        log.record("pod-1", &stay("s1"), "hello", "Hi!", then);
        assert!(log
            .recent("pod-1", &stay("s1"), then + IDLE + Duration::from_secs(1))
            .is_empty());
    }

    #[test]
    fn an_interrupted_answer_keeps_only_what_was_spoken() {
        let log = ConversationLog::new(6, IDLE);
        let now = Instant::now();
        let story = "Once upon a time there was a dragon who guarded a lake of gold.";
        log.record(
            "pod-1",
            &stay("s1"),
            "tell me a story about a dragon",
            story,
            now,
        );
        log.interrupted("pod-1", 0.4);
        let recent = log.recent("pod-1", &stay("s1"), now);
        let answer = &recent[0].assistant;
        assert!(recent[0].interrupted);
        assert!(story.starts_with(answer.as_str()), "{answer:?}");
        assert!(answer.len() < story.len() / 2 + 10, "{answer:?}");
        assert!(!answer.ends_with(' '), "cut on a word boundary: {answer:?}");
    }

    #[test]
    fn the_model_is_told_where_the_guest_cut_in() {
        let log = ConversationLog::new(6, IDLE);
        let now = Instant::now();
        log.record(
            "pod-1",
            &stay("s1"),
            "tell me a story about a dragon",
            "Once upon a time there was a dragon.",
            now,
        );
        log.interrupted("pod-1", 0.5);
        let history = chat_history(&log.recent("pod-1", &stay("s1"), now));
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].0, "tell me a story about a dragon");
        assert!(history[0].1.contains("interrupted"), "{:?}", history[0].1);
    }
}
