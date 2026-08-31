---
title: The Queue's Rules
description: What Next up holds, what Play next does, and the promises the queue keeps.
nav_order: 2
---

Next up is two lists drawn as one. First the songs you asked for, in the
order you asked for them. Then the songs the playing playlist, album, or
station would reach on its own. The rules below are the whole contract;
the queue tests in `src/app.rs` hold Fastpotify to them.

## The rules

1. **What you see is what plays.** The top row of Next up is the next
   song heard, the second row follows it, and so on down.

2. **Play next queues a song.** It goes in after the songs you queued
   before it and ahead of the playing context's own. Asking again queues
   it again, two rows for two asks; the two clicks of a double-click are
   one ask.

3. **A song that starts leaves Next up.** However it starts: the song
   before it ending, the Next button, a click, another device. The same
   song is never both Now playing and the top of Next up.

4. **Next pops.** The head of Next up becomes Now playing the moment the
   button is pressed, not when Spotify confirms it.

5. **A double-clicked row jumps the queue.** It plays at once; the rows
   above it are consumed with it, as if Next had been pressed down to
   it; the rows after it and the playing context stay untouched.

6. **Starting something new keeps your songs.** Playing a playlist or
   album replaces the context's rows under your queued songs, and the
   queued songs still play first.

7. **Clear queue removes your songs, not the context's.** The button
   appears where the promise can be kept: when this computer's player is
   the device, the one queue any client is allowed to drop.

8. **The interface acts first.** Every rule above shows its result the
   moment you act; Spotify is told afterwards. On this computer the
   engine is told directly, with no Web API detour.

9. **A late answer never undoes what you did.** Spotify's queue answers
   lag by seconds. An answer overtaken by a newer request is dropped
   unread; one that contradicts what the interface just did, naming the
   wrong playing song or still carrying a cleared row, is held back and
   asked again. Only a story Spotify keeps telling wins, and a row you
   queued is put back until Spotify confirms it. Nothing you did may
   flicker away and come back.
