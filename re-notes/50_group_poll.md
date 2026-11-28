# 50 - Group polls: detail and vote endpoints

Date: 2026-11-28. Source: Zalo Web static bundle, fetched without cookies (no login, no messages read).
No capture yet; voting live is left to the owner.

## Evidence
Chunk `https://zalo-chat-static.zadn.vn/v1/lazy/2.21dd5856eb8f1d31f964.js` (lazy chunk 2 of
`render.ac0d393903624d3db026.js`):

- Command table: `M = GROUP_POLL` (`"group_poll"` in the service-map key list), entries
  `12080 /api/poll/create`, `12081 /api/poll/detail`, `12083 /api/poll/option/add`, `12084 /api/poll/vote`,
  `12085 /api/poll/share`, `12089 /api/poll/end`.
- Host: `getGroupPollDomain() { return this.getDomainByType(GROUP_POLL) }`, i.e.
  `zpw_service_map_v3.group_poll[0]`.
- Requests (all with `_getCommonParams()` = `zpw_ver`/`zpw_type`, and `encodeAES` = the post-login
  `zpw_enk` AES, the same path as other chat APIs, **not** `ParamCipher v2`):
  ```js
  static getPollDetail(e, t) {           // GET, cmd 12081
    let n = { poll_id: t, imei: getZaloClientID() };
    url = getGroupPollDomain() + "/api/poll/detail?" + commonParams + "&params=" + enc(encodeAES(JSON(n)));
  }
  static vote(e, t, n) {                 // GET, cmd 12084
    let a = { poll_id: t, option_ids: n, imei: getZaloClientID() };
    url = getGroupPollDomain() + "/api/poll/vote?" + commonParams + "&params=" + enc(encodeAES(JSON(a)));
  }
  ```
- Response: `_preParsePollData`: HTTP 200, `data.error_code == 0`, then `decodeAES(data.data)` gives
  `{error_code, data: poll}`; `creator` and `voters` are pre-parsed as id strings.
- Poll object fields read by the UI: `poll_id`, `question`, `options[]` (`option_id`, `content`, `votes`,
  `voters[]`), `creator`, `created_time`, `expired_time`, `closed`, `status`, `joined`,
  `allow_multi_choices`, `allow_add_new_option`, `is_anonymous`, `is_hide_vote_preview`, `num_vote`
  (else the count of distinct voters).
  - Details are disabled when nobody voted, the poll is anonymous, or `is_hide_vote_preview` is set; with
    `is_hide_vote_preview && !joined` counts stay hidden until the user votes.
- Messages: `msgType "group.poll"` maps to `MSG_POLL`; `JSON.parse(message.params).pollId` identifies the
  poll and `params.question` is the preview text ("Bình chọn: <question>"). Several messages may refer to
  the same poll (create, votes); Zalo Web shows the card once and collapses older ones.
- Error `166` is used across board/poll APIs for a closed, locked or missing item (seen for todos:
  `166 -> STR_TODO_NOT_EXIST_TASK`); upstream ctdozlite PR #193 treats 166 as a closed/locked poll
  (reference only).

## What Zalo-next does
- `session.rs`: `Domains::group_poll` from `zpw_service_map_v3.group_poll`; candidates end with the `group`
  host and drop any untrusted host. `Session::get_group_poll` uses a client with no redirects and no retries;
  a vote never fails over to another host except on an explicit HTTP 404 (no replayed votes). The client
  checks the HTTP status first and caps the streamed response at 1 MiB.
- `chat.rs`: `get_poll_detail` / `vote_poll` build `{poll_id, imei}` / `{poll_id, option_ids, imei}`,
  encrypt with the session's `zpw_enk` params path, decrypt the reply, map 166 to `PollUnavailable`.
  Option ids that parse as integers are sent as numbers. The worker fetches detail after a successful vote
  unless the vote response contains the requested poll ID and a non-empty options list.
- `parse_chat_message`: `group.poll` shows "Bình chọn: <question>" and keeps `poll_id` (persisted in
  `raw_json`).
- Worker: `FetchPoll` / `VotePoll` run serially on spawned tasks. The worker checks the current session and
  its generation after entering the queue, cancels stale requests and drops stale results. Details sent
  to the GUI are limited to 100 options, 500 voter IDs per option and 1,000 characters per question or option.
- Cards fetch when visible and refresh when the newest message for the poll changes or the user clicks
  “Làm mới”. Voter names require `is_anonymous == false` and remain hidden when `is_hide_vote_preview`
  is true. Polls that hide previews show counts after the user joins.

## Not verified
Live detail/vote against a real group (owner).
