//! A message whose whole text is one phone number is shown as a contact card (owner request
//! 2026-11-28). The number is looked up lazily with Zalo's phone search for a name and avatar;
//! results are cached per account and number for the session. Messages with any other text stay
//! plain text.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::chat::{ChatMessage, ContactCard, PhoneLookupResult};
use crate::message_model::MessageKind;

use super::{WorkerCmd, WORKER_TX};

/// Zalo limits phone searches per account, so a session looks up at most this many numbers on
/// its own; the remaining cards show just the number.
const AUTO_LOOKUP_CAP: usize = 30;
/// Gap between two automatic lookups.
const LOOKUP_INTERVAL: Duration = Duration::from_millis(1500);
/// A failed lookup is retried (while its card is on screen) after this long.
const RETRY_AFTER: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
enum Entry {
    Pending,
    Found(PhoneLookupResult),
    NotFound,
    Failed(Instant),
}

#[derive(Default)]
pub(super) struct PhoneCards {
    /// Keyed by `"<account uin>:<number>"` so another account never sees these results.
    entries: HashMap<String, Entry>,
    /// Numbers seen while rendering that still need a lookup.
    queued: Vec<String>,
    started: usize,
    last_sent: Option<Instant>,
}

/// The number in local form (`0xxxxxxxxx`) when `text` is only a Vietnamese phone number:
/// `0` + 9–10 digits or `+84`/`84` + 9–10 digits, with optional spaces, dots or dashes between
/// digit groups.
pub(super) fn phone_only_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 24 {
        return None;
    }
    let (plus, rest) = match trimmed.strip_prefix('+') {
        Some(rest) => (true, rest),
        None => (false, trimmed),
    };
    let mut digits = String::with_capacity(rest.len());
    let mut last_was_separator = false;
    for ch in rest.chars() {
        match ch {
            '0'..='9' => {
                digits.push(ch);
                last_was_separator = false;
            }
            ' ' | '.' | '-' if !digits.is_empty() && !last_was_separator => {
                last_was_separator = true;
            }
            _ => return None,
        }
    }
    if last_was_separator {
        return None;
    }
    let national = if let Some(national) = digits
        .strip_prefix("84")
        .filter(|_| plus || digits.len() >= 11)
    {
        national
    } else if plus {
        return None;
    } else {
        digits.strip_prefix('0')?
    };
    // Mobile numbers have 9 national digits, landlines 10; neither starts with 0.
    if !(9..=10).contains(&national.len()) || national.starts_with('0') {
        return None;
    }
    Some(format!("0{national}"))
}

fn key(uin: &str, phone: &str) -> String {
    format!("{uin}:{phone}")
}

fn card_from(phone: &str, entry: Option<&Entry>) -> ContactCard {
    match entry {
        Some(Entry::Found(profile)) => {
            let name = [&profile.display_name, &profile.zalo_name]
                .into_iter()
                .map(|name| name.trim())
                .find(|name| !name.is_empty())
                .unwrap_or(phone)
                .to_string();
            let avatar = Some(profile.avatar.trim())
                .filter(|url| !url.is_empty())
                .map(str::to_string);
            ContactCard {
                uid: profile.user_id.clone(),
                name,
                phone: Some(phone.to_string()),
                avatar,
                is_oa: false,
            }
        }
        _ => ContactCard {
            uid: String::new(),
            name: phone.to_string(),
            phone: Some(phone.to_string()),
            avatar: None,
            is_oa: false,
        },
    }
}

impl PhoneCards {
    /// The card to draw for `msg`, if its text is only a phone number.
    pub(super) fn card_for(&self, uin: &str, msg: &ChatMessage) -> Option<ContactCard> {
        let phone = Self::phone_of(msg)?;
        Some(card_from(&phone, self.entries.get(&key(uin, &phone))))
    }

    /// Like [`PhoneCards::card_for`] but returns the message with the card attached; used when
    /// handling a click.
    pub(super) fn with_card(&self, uin: &str, mut msg: ChatMessage) -> ChatMessage {
        if let Some(card) = self.card_for(uin, &msg) {
            msg.contact_card = Some(card);
        }
        msg
    }

    /// The row of `msg` is on screen: queue a lookup for its number if none is known yet (or a
    /// failed one may be retried).
    pub(super) fn request(&mut self, uin: &str, msg: &ChatMessage, now: Instant) {
        let Some(phone) = Self::phone_of(msg) else {
            return;
        };
        let wanted = match self.entries.get(&key(uin, &phone)) {
            None => true,
            Some(Entry::Failed(at)) => now.saturating_duration_since(*at) >= RETRY_AFTER,
            Some(_) => false,
        };
        if wanted
            && !uin.is_empty()
            && self.started + self.queued.len() < AUTO_LOOKUP_CAP
            && !self.queued.contains(&phone)
        {
            self.queued.push(phone);
        }
    }

    fn phone_of(msg: &ChatMessage) -> Option<String> {
        // Only plain text messages: no media, links, cards or polls (an imported photo can keep
        // a numeric title with only a thumbnail). Recalled messages carry a "[Đã thu hồi]" text.
        if !matches!(msg.kind, MessageKind::Text | MessageKind::Unknown)
            || msg.contact_card.is_some()
            || msg.link_url.is_some()
            || msg.link_thumb.is_some()
            || msg.link_src.is_some()
            || msg.media_size.is_some()
            || msg.sticker_id.is_some()
            || msg.poll_id.is_some()
        {
            return None;
        }
        phone_only_text(&msg.text)
    }

    /// Send at most one queued lookup per [`LOOKUP_INTERVAL`] to the worker, so opening a
    /// conversation full of numbers does not burst Zalo's phone search. Returns `true` while
    /// lookups are still waiting (the caller schedules a repaint).
    pub(super) fn flush(&mut self, uin: &str, now: Instant) -> bool {
        if uin.is_empty() {
            self.queued.clear();
            return false;
        }
        if self.queued.is_empty() {
            return false;
        }
        if self
            .last_sent
            .is_some_and(|at| now.saturating_duration_since(at) < LOOKUP_INTERVAL)
        {
            return true;
        }
        let Some(tx) = WORKER_TX.get() else {
            return true;
        };
        let phone = self.queued.remove(0);
        self.entries.insert(key(uin, &phone), Entry::Pending);
        self.started += 1;
        self.last_sent = Some(now);
        let _ = tx.send(WorkerCmd::LookupPhoneCard {
            uin: uin.to_string(),
            phone,
        });
        !self.queued.is_empty()
    }

    /// Store a lookup result. `None` (the search failed: network, rate limit, unknown number)
    /// may be retried after [`RETRY_AFTER`] while the card is on screen; a profile without an id
    /// is a confirmed miss.
    pub(super) fn finish(
        &mut self,
        uin: &str,
        phone: &str,
        profile: Option<PhoneLookupResult>,
        now: Instant,
    ) {
        let entry = match profile {
            Some(profile) if !profile.user_id.trim().is_empty() => Entry::Found(profile),
            Some(_) => Entry::NotFound,
            None => Entry::Failed(now),
        };
        self.entries.insert(key(uin, phone), entry);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_vietnamese_numbers_in_common_formats() {
        for (text, want) in [
            ("0912345678", "0912345678"),
            (" 091 234 5678 ", "0912345678"),
            ("091.234.5678", "0912345678"),
            ("0912-345-678", "0912345678"),
            ("+84912345678", "0912345678"),
            ("+84 912 345 678", "0912345678"),
            ("84912345678", "0912345678"),
            ("02412345678", "02412345678"),
        ] {
            assert_eq!(phone_only_text(text).as_deref(), Some(want), "{text}");
        }
    }

    #[test]
    fn rejects_other_text() {
        for text in [
            "",
            "gọi 0912345678",
            "0912345678 nhé",
            "0912345678\n0987654321",
            "091234567",
            "091234567890",
            "+1 415 555 0100",
            "1234567890",
            "0012345678",
            "0912  345678",
            "0912345678.",
            "+0912345678",
        ] {
            assert_eq!(phone_only_text(text), None, "{text:?}");
        }
    }

    fn message(text: &str) -> ChatMessage {
        ChatMessage {
            text: text.into(),
            ..Default::default()
        }
    }

    fn profile(uid: &str, name: &str) -> PhoneLookupResult {
        PhoneLookupResult {
            user_id: uid.into(),
            display_name: name.into(),
            zalo_name: String::new(),
            phone: String::new(),
            avatar: String::new(),
            status: String::new(),
            friendship: Default::default(),
        }
    }

    #[test]
    fn queues_each_visible_number_once_and_keeps_results_per_account() {
        let now = Instant::now();
        let mut cards = PhoneCards::default();
        let msg = message("0912 345 678");
        let card = cards.card_for("1", &msg).unwrap();
        assert_eq!(card.name, "0912345678");
        assert!(card.uid.is_empty());
        assert!(cards.queued.is_empty(), "rendering alone never queues");
        cards.request("1", &msg, now);
        cards.request("1", &msg, now);
        assert_eq!(cards.queued, vec!["0912345678".to_string()]);

        cards.queued.clear();
        cards.entries.insert(key("1", "0912345678"), Entry::Pending);
        cards.finish("1", "0912345678", Some(profile("42", "An")), now);
        let card = cards.card_for("1", &msg).unwrap();
        assert_eq!((card.uid.as_str(), card.name.as_str()), ("42", "An"));
        cards.request("1", &msg, now);
        assert!(cards.queued.is_empty());

        // Another account does not reuse the result.
        assert!(cards.card_for("2", &msg).unwrap().uid.is_empty());
    }

    #[test]
    fn failed_lookups_retry_later_and_confirmed_misses_never() {
        let now = Instant::now();
        let mut cards = PhoneCards::default();
        cards.finish("1", "0912345678", None, now);
        cards.request("1", &message("0912345678"), now);
        assert!(cards.queued.is_empty());
        cards.request("1", &message("0912345678"), now + RETRY_AFTER);
        assert_eq!(cards.queued.len(), 1);

        cards.queued.clear();
        cards.finish("1", "0987654321", Some(profile("", "")), now);
        cards.request("1", &message("0987654321"), now + RETRY_AFTER);
        assert!(cards.queued.is_empty());
        assert!(cards.card_for("1", &message("xin chào")).is_none());
    }

    #[test]
    fn media_messages_with_numeric_text_stay_media() {
        let mut photo = message("0912345678");
        photo.link_thumb = Some("https://example.invalid/t.jpg".into());
        assert!(PhoneCards::default().card_for("1", &photo).is_none());
        let mut sized = message("0912345678");
        sized.media_size = Some((10, 10));
        assert!(PhoneCards::default().card_for("1", &sized).is_none());
    }

    #[test]
    fn automatic_lookups_stop_at_the_cap() {
        let mut cards = PhoneCards {
            started: AUTO_LOOKUP_CAP,
            ..Default::default()
        };
        cards.request("1", &message("0912345678"), Instant::now());
        assert!(cards.queued.is_empty());
    }

    #[test]
    fn flush_paces_lookups() {
        let now = Instant::now();
        let mut cards = PhoneCards {
            last_sent: Some(now),
            ..Default::default()
        };
        cards.request("1", &message("0912345678"), now);
        assert!(cards.flush("1", now + Duration::from_millis(10)));
        assert_eq!(cards.queued.len(), 1, "still waiting for the interval");
    }
}
