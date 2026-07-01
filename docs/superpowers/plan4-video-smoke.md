# Plan 4 — Screen capture + video e2e smoke

Prereq: complete the Plan 3 bring-up (server + coturn + agent + web, session
connects, input works) from `plan3-input-smoke.md`. macOS: grant the agent
**Screen Recording** permission (System Settings → Privacy & Security → Screen
Recording) and restart it — otherwise the remote video is blank (the agent logs
a warning at startup).

## Stage 1 — test pattern (proves encode + transport + browser decode)
1. Start the agent with `RD_VIDEO_SOURCE=testpattern` (prefix the run command).
2. Connect from the browser and open the session.
3. **Expected:** the `<video>` shows an animated gradient with a marching white
   square. This confirms H.264 negotiation, Annex-B packetization, the video
   m-line, and browser decode — independent of screen capture.
4. Confirm Plan 3 still works: focus the video, move the mouse / type → the
   controlled machine responds; "Sent events" log updates.

## Stage 2 — real screen
1. Restart the agent WITHOUT `RD_VIDEO_SOURCE` (defaults to `screen`).
2. Reconnect.
3. **Expected:** the `<video>` shows the被控端's live main display. Moving the
   controlled machine's windows is visible in near real time.

## Expected
Live remote screen in the browser plus working mouse/keyboard injection over the
same PeerConnection. Fixed 720p/30fps/3Mbps (no adaptation yet).
