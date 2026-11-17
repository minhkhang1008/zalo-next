# Video call quality (backlog, not urgent)

Owner report 2026-11-17, after the `voicecall` `zpw` fix (`re-notes/48_voicecall_zpw_pair.md`):
incoming/outgoing video works at the signalling level, but

1. **Zalo-next does not show the phone's camera.** The phone's video RTP is received (see
   `re-notes/08_video_call.md`: PT 97 whole access units, PT 98 fragments `1c <flags> <Annex-B>`, header
   extension, PLI every ~500 ms) but is not decoded/rendered, or is dropped.
2. **The phone sees Zalo-next's camera blurry and delayed.** Likely causes to check first: encoder
   bitrate/resolution (`maxBitrate`, `bwProfiles`, `startupBitrate` in the invite `zrtc_config`), keyframe
   cadence and PLI handling, frame fragmentation size, missing NACK/REMB feedback, RR reports that report
   no reception (`re-notes/08_video_call.md` second capture), capture resolution/fps from the macOS camera.

Suggested plan: (a) log per-second inbound video packets/bytes/frames decoded and the first decode error;
(b) verify the Annex-B reassembly and the h264 decoder path against a recorded inbound stream; (c) compare
our outbound bitrate/fps/resolution with the profile the phone requests; (d) implement proper RR/NACK/PLI
responses. Acceptance: phone camera visible in the Zalo-next call window within 2 s, and outbound video at
>= 360p / 15 fps without growing latency.
