# The Queue's Rules

The queue is the list of what plays next. It has two parts. On top,
under **Playing next**, are the songs you queued yourself. Below them,
under **Next up**, are the songs that come next in whatever playlist or
album is playing. Your songs always play first.

These are the rules the app follows. The queue lives in the player, so
every one of them is about this computer: there is no other device with a
queue of its own to disagree with. The tests in `src/engine/queue.rs` hold
up the ones about what plays next; the ones in `src/app.rs` hold up what
the panel draws and what a click asks for.

1. **The list shows the play order.** The top row plays next, followed by the
   rows below it.

2. **Play next adds a song to your part of the queue.** It goes after
   the songs you queued earlier and before the playlist's songs. Queue
   the same song twice and it plays twice. A double-click only counts
   once.

3. **When a song starts, its row leaves the queue.** It doesn't matter
   how it started: the song before it ended, you pressed Next, or you
   clicked it. A song is never shown as playing and as next at the same
   time.

4. **Next removes the top row right away.** The row is gone before the
   player has opened the song, so the list never waits on the server.

5. **Playing a row from the queue skips to it.** The rows above it are
   skipped and removed, as if you had pressed Next down to it. The rows
   below it stay, and the playlist keeps going afterwards.

6. **Starting a new playlist keeps your songs.** The rows underneath
   change to the new playlist; your songs stay on top and still play
   first.

7. **Clear only removes your songs.** The trash button sits beside the
   *Playing next* heading and empties that section; the playlist's rows
   below stay. It shows while there is something of yours to remove.

8. **Changes appear immediately.** The player is in Fastsonic, so a change
   to the queue is made before the screen is drawn again. Nothing is
   guessed at and nothing has to be confirmed.

9. **Closing the app keeps the queue.** Fastsonic saves it locally. When you
   resume the last song, it restores your queued songs and the place the
   album or playlist had reached — including the album's own place under a
   song you had queued, so that resuming plays your song and then carries
   on where the album was. Changing the output device or the normalisation
   switch replaces the player; the queue comes across with it.
