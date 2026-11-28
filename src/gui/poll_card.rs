use std::collections::{BTreeSet, HashMap, HashSet};

use eframe::egui::{self, Id, RichText, Ui};

use super::messages::{normalize_ts_ms, BubbleAction};
use super::{CachedPoll, PollState, ThemePalette};
use crate::chat::ChatMessage;
use crate::poll::{PollDetail, PollOption, VoteRules};

/// Pick owners from the whole thread before applying history, search, or star filters.
pub(super) fn card_owners(messages: &[ChatMessage]) -> HashSet<usize> {
    let mut newest_by_poll = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        let Some(poll_id) = message.poll_id.as_deref() else {
            continue;
        };
        let candidate = (normalize_ts_ms(message.ts), index);
        newest_by_poll
            .entry(poll_id)
            .and_modify(|newest| *newest = candidate.max(*newest))
            .or_insert(candidate);
    }
    newest_by_poll
        .into_values()
        .map(|(_, index)| index)
        .collect()
}

fn option_id(option: &PollOption) -> Option<&str> {
    option
        .option_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
}

fn initial_selection(poll: &PollDetail, user_uin: &str) -> BTreeSet<String> {
    poll.options
        .iter()
        .filter(|option| !user_uin.is_empty() && option.voters.iter().any(|id| id == user_uin))
        .filter_map(option_id)
        .map(str::to_owned)
        .collect()
}

fn vote_enabled(
    rules: VoteRules,
    selection: &BTreeSet<String>,
    current: &BTreeSet<String>,
) -> bool {
    rules.can_vote
        && !selection.is_empty()
        && selection.len() <= rules.max_choices
        && selection != current
}

fn total_voters(poll: &PollDetail) -> u64 {
    poll.num_vote.unwrap_or_else(|| {
        poll.options
            .iter()
            .flat_map(|option| &option.voters)
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>()
            .len() as u64
    })
}

fn vote_share(votes: u64, total: u64) -> (f32, String) {
    let fraction = if total == 0 {
        0.0
    } else {
        votes.min(total) as f32 / total as f32
    };
    (fraction, format!("{votes} lượt · {:.0}%", fraction * 100.0))
}

fn needs_fetch(cached: Option<&CachedPoll>, owner_message_id: &str) -> bool {
    cached.is_none_or(|cached| {
        !matches!(cached.state, PollState::Loading { .. })
            && cached.owner_message_id != owner_message_id
    })
}

pub(super) struct PollCard<'a> {
    pub poll_id: &'a str,
    pub cached: Option<&'a CachedPoll>,
    pub user_uin: &'a str,
    pub voter_name: &'a dyn Fn(&str) -> Option<&'a str>,
    pub selecting_messages: bool,
}

impl PollCard<'_> {
    pub(super) fn render(
        &self,
        ui: &mut Ui,
        message: &ChatMessage,
        pal: &ThemePalette,
    ) -> Option<BubbleAction> {
        let selection_key = Id::new(("poll_selection", self.user_uin, self.poll_id));
        let state = self.cached.map(|cached| &cached.state);
        let detail = match state {
            Some(PollState::Loaded(poll)) => Some(poll),
            _ => None,
        };
        let question = detail
            .and_then(|poll| poll.question.as_deref())
            .filter(|question| !question.trim().is_empty())
            .or_else(|| message.text.strip_prefix("Bình chọn: "));
        let body = ui.scope(|ui| {
            let refresh_clicked = ui
                .horizontal(|ui| {
                    ui.label(RichText::new("Bình chọn").strong().color(pal.primary_hover));
                    if matches!(
                        state,
                        Some(PollState::Loaded(_) | PollState::Loading { refreshing: true })
                    ) {
                        ui.add_enabled(
                            detail.is_some() && !self.selecting_messages,
                            egui::Button::new("Làm mới").small(),
                        )
                        .clicked()
                    } else {
                        false
                    }
                })
                .inner;
            if let Some(question) = question {
                ui.add(
                    egui::Label::new(RichText::new(question).strong().color(pal.text_primary))
                        .wrap(),
                );
            }
            if refresh_clicked {
                return Some(self.fetch_action());
            }
            match state {
                Some(PollState::Loaded(poll)) => self.render_loaded(ui, poll, pal, selection_key),
                Some(PollState::Error(message)) => {
                    ui.add(
                        egui::Label::new(RichText::new(message).color(pal.text_secondary)).wrap(),
                    );
                    ui.add_enabled(!self.selecting_messages, egui::Button::new("Thử lại"))
                        .clicked()
                        .then(|| self.fetch_action())
                }
                None | Some(PollState::Loading { .. }) => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0).color(pal.primary));
                        ui.add(
                            egui::Label::new(
                                RichText::new("Đang tải bình chọn…").color(pal.text_secondary),
                            )
                            .wrap(),
                        );
                    });
                    None
                }
            }
        });
        // History rows outside the scroll clip still take up space in the chat layout.
        let action =
            if needs_fetch(self.cached, &message.id) && ui.is_rect_visible(body.response.rect) {
                Some(self.fetch_action())
            } else {
                body.inner
            };
        if matches!(action, Some(BubbleAction::FetchPoll { .. })) {
            ui.data_mut(|data| data.remove::<BTreeSet<String>>(selection_key));
        }
        action
    }

    fn fetch_action(&self) -> BubbleAction {
        BubbleAction::FetchPoll {
            poll_id: self.poll_id.to_owned(),
        }
    }

    fn render_loaded(
        &self,
        ui: &mut Ui,
        poll: &PollDetail,
        pal: &ThemePalette,
        selection_key: Id,
    ) -> Option<BubbleAction> {
        let rules = VoteRules::for_poll(poll, chrono::Utc::now().timestamp_millis());
        let current = initial_selection(poll, self.user_uin);
        let mut selection = ui
            .data(|data| data.get_temp(selection_key))
            .unwrap_or_else(|| current.clone());
        let total = rules.show_counts.then(|| total_voters(poll));
        let allow_multiple = poll.allow_multi_choices.unwrap_or_default();
        let can_interact = rules.can_vote && !self.selecting_messages;
        let mut changed = false;

        for option in &poll.options {
            let id = option_id(option);
            let mut checked = id.is_some_and(|id| selection.contains(id));
            let text = option.content.as_deref().unwrap_or("Lựa chọn");
            let response = if allow_multiple {
                ui.add_enabled(
                    can_interact && id.is_some(),
                    egui::Checkbox::new(&mut checked, text),
                )
            } else {
                ui.add_enabled(
                    can_interact && id.is_some(),
                    egui::RadioButton::new(checked, text),
                )
            };
            if let Some(id) = id.filter(|_| response.clicked()) {
                if !allow_multiple {
                    selection.clear();
                    selection.insert(id.to_owned());
                } else if checked {
                    selection.insert(id.to_owned());
                } else {
                    selection.remove(id);
                }
                changed = true;
            }
            if let Some(total) = total {
                let votes = option.votes.unwrap_or(option.voters.len() as u64);
                let (fraction, label) = vote_share(votes, total);
                let result = ui.horizontal(|ui| {
                    let width = ui.available_width() * 0.45;
                    ui.add(
                        egui::ProgressBar::new(fraction)
                            .desired_width(width)
                            .desired_height(5.0)
                            .fill(if fraction > 0.0 {
                                pal.primary
                            } else {
                                ui.visuals().extreme_bg_color
                            }),
                    );
                    ui.label(RichText::new(label).size(12.0).color(pal.text_secondary));
                });
                if rules.show_voters && !option.voters.is_empty() {
                    result.response.on_hover_ui(|ui| {
                        for id in option.voters.iter().take(8) {
                            ui.label(
                                (self.voter_name)(id)
                                    .filter(|name| !name.trim().is_empty())
                                    .unwrap_or("Thành viên"),
                            );
                        }
                    });
                }
            }
        }

        ui.separator();
        if let Some(total) = total {
            ui.label(
                RichText::new(format!("{total} người đã bình chọn")).color(pal.text_secondary),
            );
        }
        if !rules.can_vote {
            ui.label(RichText::new("Bình chọn đã kết thúc").color(pal.text_muted));
        }
        let label = if current.is_empty() && !poll.joined.unwrap_or_default() {
            "Bình chọn"
        } else {
            "Đổi bình chọn"
        };
        if ui
            .add_enabled(
                can_interact && vote_enabled(rules, &selection, &current),
                egui::Button::new(label),
            )
            .clicked()
        {
            ui.data_mut(|data| data.remove::<BTreeSet<String>>(selection_key));
            return Some(BubbleAction::VotePoll {
                poll_id: self.poll_id.to_owned(),
                option_ids: selection.into_iter().collect(),
            });
        }
        if changed {
            ui.data_mut(|data| data.insert_temp(selection_key, selection));
            ui.ctx().request_repaint();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, poll_id: Option<&str>, ts: i64) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            poll_id: poll_id.map(str::to_owned),
            ts,
            ..Default::default()
        }
    }

    #[test]
    fn newest_message_owns_each_poll_card_across_the_thread() {
        let messages = [
            message("msg-1", Some("poll-1"), 1_700_000_000_000),
            message("msg-2", Some("poll-2"), 1_700_000_000_000),
            message("msg-3", Some("poll-1"), 1_700_000_001),
            message("msg-4", Some("poll-1"), 1_700_000_000_000),
            message("msg-5", Some("poll-2"), 1_700_000_000_000),
            message("msg-6", None, 1_700_000_002_000),
        ];
        assert_eq!(card_owners(&messages), HashSet::from([2, 4]));
        assert!(card_owners(&messages[5..]).is_empty());
    }

    fn option(id: Option<&str>, voters: &[&str]) -> PollOption {
        PollOption {
            option_id: id.map(str::to_owned),
            voters: voters.iter().map(|id| (*id).to_owned()).collect(),
            ..Default::default()
        }
    }

    fn selection(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn initial_selection_includes_only_identified_options_voted_for_by_the_user() {
        let poll = PollDetail {
            options: vec![
                option(Some("a"), &["user-1", "user-2"]),
                option(Some("b"), &["user-2"]),
                option(Some("c"), &["user-1"]),
                option(None, &["user-1", ""]),
                option(Some(""), &["user-1", ""]),
            ],
            ..Default::default()
        };
        assert_eq!(initial_selection(&poll, "user-1"), selection(&["a", "c"]));
        assert_eq!(initial_selection(&poll, "user-2"), selection(&["a", "b"]));
        assert!(initial_selection(&poll, "").is_empty());
    }

    #[test]
    fn vote_button_requires_a_changed_nonempty_selection_within_the_limit() {
        let mut poll = PollDetail {
            options: vec![option(Some("a"), &[]), option(Some("b"), &[])],
            ..Default::default()
        };
        let current = selection(&["a"]);
        let one = selection(&["b"]);
        let two = selection(&["a", "b"]);
        let single = VoteRules::for_poll(&poll, 0);
        assert!(vote_enabled(single, &one, &current));
        assert!(!vote_enabled(single, &two, &current));
        assert!(!vote_enabled(single, &current, &current));
        assert!(!vote_enabled(single, &selection(&[]), &current));

        poll.allow_multi_choices = Some(true);
        let multi = VoteRules::for_poll(&poll, 0);
        assert!(vote_enabled(multi, &two, &current));
        assert!(!vote_enabled(multi, &two, &selection(&["b", "a"])));
        assert!(!vote_enabled(multi, &selection(&["a", "b", "c"]), &current));

        poll.closed = Some(true);
        assert!(!vote_enabled(VoteRules::for_poll(&poll, 0), &one, &current));
        poll.closed = None;
        poll.expired_time = Some(1);
        assert!(!vote_enabled(VoteRules::for_poll(&poll, 1), &one, &current));
    }

    #[test]
    fn vote_count_label_rounds_and_clamps_the_percentage() {
        assert_eq!(vote_share(0, 0), (0.0, "0 lượt · 0%".into()));
        assert_eq!(vote_share(3, 0), (0.0, "3 lượt · 0%".into()));
        assert_eq!(vote_share(1, 3), (1.0 / 3.0, "1 lượt · 33%".into()));
        assert_eq!(vote_share(2, 3), (2.0 / 3.0, "2 lượt · 67%".into()));
        assert_eq!(vote_share(5, 3), (1.0, "5 lượt · 100%".into()));
    }

    #[test]
    fn voter_total_prefers_the_server_count_and_deduplicates_the_fallback() {
        let mut poll = PollDetail {
            options: vec![
                option(Some("a"), &["user-1", "user-2"]),
                option(Some("b"), &["user-2", "user-3"]),
            ],
            ..Default::default()
        };
        assert_eq!(total_voters(&poll), 3);
        poll.num_vote = Some(5);
        assert_eq!(total_voters(&poll), 5);
    }

    #[test]
    fn visible_poll_fetches_once_per_card_owner() {
        let ctx = egui::Context::default();
        let msg = message("msg-2", Some("poll-1"), 0);
        let previous = CachedPoll {
            owner_message_id: "msg-1".into(),
            state: PollState::Loaded(PollDetail::default()),
        };
        let loading = CachedPoll {
            owner_message_id: "msg-1".into(),
            state: PollState::Loading { refreshing: true },
        };
        let error = CachedPoll {
            owner_message_id: msg.id.clone(),
            state: PollState::Error("Không thể tải bình chọn.".into()),
        };
        let loaded = CachedPoll {
            owner_message_id: msg.id.clone(),
            state: PollState::Loaded(PollDetail::default()),
        };
        for (visible, cached, fetches) in [
            (false, None, false),
            (true, None, true),
            (true, Some(&previous), true),
            (false, Some(&previous), false),
            (true, Some(&loading), false),
            (true, Some(&error), false),
            (true, Some(&loaded), false),
        ] {
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if !visible {
                        ui.set_clip_rect(ui.clip_rect().translate(egui::vec2(0.0, -100_000.0)));
                    }
                    let card = PollCard {
                        poll_id: "poll-1",
                        cached,
                        user_uin: "user-1",
                        voter_name: &|_| None,
                        selecting_messages: false,
                    };
                    let action = card.render(ui, &msg, &ThemePalette::light());
                    if fetches {
                        assert!(matches!(action, Some(BubbleAction::FetchPoll { poll_id }) if poll_id == "poll-1"));
                    } else {
                        assert!(action.is_none());
                    }
                });
            });
        }
    }
}
