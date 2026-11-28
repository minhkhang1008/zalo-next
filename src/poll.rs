//! Group poll payloads and voting rules from `re-notes/50_group_poll.md`.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Number, Value};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollDetail {
    #[serde(default, deserialize_with = "optional_id")]
    pub poll_id: Option<String>,
    pub question: Option<String>,
    #[serde(default, deserialize_with = "default_list")]
    pub options: Vec<PollOption>,
    #[serde(default, deserialize_with = "optional_id")]
    pub creator: Option<String>,
    pub created_time: Option<i64>,
    pub expired_time: Option<i64>,
    pub closed: Option<bool>,
    /// Numeric statuses are preserved as strings. The wire value for a numeric lock is unknown.
    #[serde(default, deserialize_with = "optional_id")]
    pub status: Option<String>,
    pub joined: Option<bool>,
    pub allow_multi_choices: Option<bool>,
    pub allow_add_new_option: Option<bool>,
    pub is_anonymous: Option<bool>,
    pub is_hide_vote_preview: Option<bool>,
    pub num_vote: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollOption {
    #[serde(default, deserialize_with = "optional_id")]
    pub option_id: Option<String>,
    pub content: Option<String>,
    pub votes: Option<u64>,
    #[serde(default, deserialize_with = "id_list")]
    pub voters: Vec<String>,
}

impl PollDetail {
    pub(crate) fn bounded(mut self) -> Self {
        if self.options.len() > 100 {
            self.options.truncate(100);
            self.options.shrink_to_fit();
        }
        truncate_poll_text(&mut self.question);
        for option in &mut self.options {
            if option.voters.len() > 500 {
                option.voters.truncate(500);
                option.voters.shrink_to_fit();
            }
            truncate_poll_text(&mut option.content);
        }
        self
    }
}

fn truncate_poll_text(text: &mut Option<String>) {
    if let Some(text) = text {
        if let Some((index, _)) = text.char_indices().nth(1000) {
            text.truncate(index);
            text.shrink_to_fit();
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WireId {
    Text(String),
    Number(Number),
}

impl From<WireId> for String {
    fn from(id: WireId) -> Self {
        match id {
            WireId::Text(text) => text,
            WireId::Number(number) => number.to_string(),
        }
    }
}

fn optional_id<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<WireId>::deserialize(de)?.map(String::from))
}

fn default_list<'de, D: Deserializer<'de>, T: Deserialize<'de>>(de: D) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(de)?.unwrap_or_default())
}

fn id_list<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<String>, D::Error> {
    Ok(default_list::<_, WireId>(de)?
        .into_iter()
        .map(String::from)
        .collect())
}

fn message_param<T>(
    content: &Value,
    key: &str,
    read: impl FnOnce(&Value) -> Option<T>,
) -> Option<T> {
    let params = content.get("params")?;
    if let Some(text) = params.as_str() {
        read(serde_json::from_str::<Value>(text).ok()?.get(key)?)
    } else {
        read(params.get(key)?)
    }
}

/// Read the poll ID from the wire content of a `group.poll` message.
pub fn poll_id_from_message(content: &Value) -> Option<String> {
    message_param(content, "pollId", |value| {
        WireId::deserialize(value)
            .ok()
            .map(String::from)
            .filter(|id| !id.trim().is_empty())
    })
}

/// Read the preview question from the wire content of a `group.poll` message.
pub fn question_from_message(content: &Value) -> Option<String> {
    message_param(content, "question", |value| {
        value
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteRules {
    pub can_vote: bool,
    pub max_choices: usize,
    pub show_counts: bool,
    pub show_voters: bool,
}

impl VoteRules {
    /// Calculate voting and visibility rules using a Unix time in milliseconds.
    pub fn for_poll(poll: &PollDetail, now_ms: i64) -> Self {
        let expired = poll
            .expired_time
            .is_some_and(|expiry| expiry > 0 && expiry <= now_ms);
        let locked = poll
            .status
            .as_deref()
            .is_some_and(|status| status.eq_ignore_ascii_case("locked"));

        Self {
            can_vote: !poll.closed.unwrap_or_default() && !expired && !locked,
            max_choices: if poll.allow_multi_choices.unwrap_or_default() {
                poll.options.len()
            } else {
                1
            },
            show_counts: !poll.is_hide_vote_preview.unwrap_or_default()
                || poll.joined.unwrap_or_default(),
            show_voters: poll.is_anonymous == Some(false)
                && poll.is_hide_vote_preview != Some(true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn poll_message_reads_object_and_string_params() {
        for (id, expected) in [(json!(42), "42"), (json!("poll-1"), "poll-1")] {
            let params = json!({"pollId": id, "question": "Chọn ngày nào?"});
            for content in [
                json!({"params": params}),
                json!({"params": params.to_string()}),
            ] {
                assert_eq!(poll_id_from_message(&content).as_deref(), Some(expected));
                assert_eq!(
                    question_from_message(&content).as_deref(),
                    Some("Chọn ngày nào?")
                );
            }
        }
        for content in [
            json!({}),
            json!({"params": "{"}),
            json!({"params": {"pollId": null}}),
        ] {
            assert_eq!(poll_id_from_message(&content), None);
            assert_eq!(question_from_message(&content), None);
        }
    }

    #[test]
    fn poll_vote_rules_honor_closed_locked_and_expired_states() {
        let mut poll = PollDetail {
            expired_time: Some(1000),
            ..Default::default()
        };
        assert!(VoteRules::for_poll(&poll, 999).can_vote);
        assert!(!VoteRules::for_poll(&poll, 1000).can_vote);
        poll.expired_time = Some(0);
        assert!(VoteRules::for_poll(&poll, 1000).can_vote);
        poll.status = Some("LOCKED".into());
        assert!(!VoteRules::for_poll(&poll, 1000).can_vote);
        poll.status = None;
        poll.closed = Some(true);
        assert!(!VoteRules::for_poll(&poll, 1000).can_vote);
    }

    #[test]
    fn poll_vote_rules_honor_choice_and_visibility_flags() {
        let mut poll = PollDetail::default();
        assert_eq!(
            VoteRules::for_poll(&poll, 0),
            VoteRules {
                can_vote: true,
                max_choices: 1,
                show_counts: true,
                show_voters: false,
            }
        );
        poll.allow_multi_choices = Some(true);
        poll.options = vec![PollOption::default(); 3];
        poll.is_anonymous = Some(false);
        poll.is_hide_vote_preview = Some(true);
        assert_eq!(
            VoteRules::for_poll(&poll, 0),
            VoteRules {
                can_vote: true,
                max_choices: 3,
                show_counts: false,
                show_voters: false,
            }
        );
        poll.joined = Some(true);
        assert!(VoteRules::for_poll(&poll, 0).show_counts);
        assert!(!VoteRules::for_poll(&poll, 0).show_voters);
    }

    #[test]
    fn poll_voter_names_require_explicitly_non_anonymous_votes() {
        for (flags, show_voters) in [
            (json!({}), false),
            (json!({"is_anonymous": null}), false),
            (json!({"is_anonymous": true}), false),
            (json!({"is_anonymous": false}), true),
        ] {
            let poll = serde_json::from_value(flags).unwrap();
            assert_eq!(VoteRules::for_poll(&poll, 0).show_voters, show_voters);
        }
    }

    #[test]
    fn poll_detail_bounds_options_voters_and_unicode_text() {
        let text = "ế".repeat(1000);
        let option = PollOption {
            content: Some(format!("{text}x")),
            voters: (0..501).map(|index| format!("user-{index}")).collect(),
            ..Default::default()
        };
        let poll = PollDetail {
            question: Some(format!("{text}x")),
            options: vec![option; 101],
            ..Default::default()
        }
        .bounded();

        assert_eq!(poll.question.as_deref(), Some(text.as_str()));
        assert_eq!(poll.options.len(), 100);
        for option in poll.options {
            assert_eq!(option.content.as_deref(), Some(text.as_str()));
            assert_eq!(option.voters.len(), 500);
            assert_eq!(option.voters[499], "user-499");
        }
    }

    #[test]
    fn poll_fields_default_when_absent_or_null() {
        for value in [json!({}), json!({"poll_id": null, "options": null})] {
            assert_eq!(
                serde_json::from_value::<PollDetail>(value).unwrap(),
                PollDetail::default()
            );
        }
        let option: PollOption = serde_json::from_value(json!({"voters": null})).unwrap();
        assert_eq!(option, PollOption::default());
    }
}
