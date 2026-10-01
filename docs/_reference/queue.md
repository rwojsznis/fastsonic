# The Queue's Rules

The queue is the list of what plays next. It has two parts. On top,
under **Playing next**, are the songs you queued yourself. Below them,
under **Next up**, are the songs that come next in whatever playlist or
album is playing. Your songs always play first.

Above the playing song, the queue names the album, playlist, artist,
Liked Songs, or radio it came from; a radio is named after the song,
playlist, album, or artist it is based on. Select that name to open its
page.

The **Shuffle** button beside a page's **Play** button changes the shuffle
mode and starts nothing. Chosen while nothing plays, it applies to the next
**Play**; while another album or playlist plays, it shuffles that one, and the
queue shows the new order. **Shuffle play** in a menu still starts the
collection shuffled.

These are the rules the app follows. The queue lives in the player, so
every one of them is about this computer: there is no other device with a
queue of its own to disagree with. The tests in `src/engine/queue.rs` hold
up the ones about what plays next; the ones in `src/app.rs` and
`src/demo.rs` hold up what the panel draws and what a click or a drop asks
for.

1. **The list shows the play order.** The top row plays next, followed by the
   rows below it.

2. **Add to queue adds a song to your part of the queue.** It goes after
   the songs you queued earlier and before the playlist's songs. Queue
   the same song twice and it plays twice. A double-click only counts
   once. An album goes in as its songs, in its own order, once the server
   has listed them; anything you queue meanwhile waits behind it, and
   Clear forgets an album still on its way.

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
   on where the album was. A radio is only the songs its page showed, and
   those are not kept: resuming one plays the last song on its own, still
   named as the radio's. Changing the output device or the normalisation
   switch replaces the player; the queue comes across with it, a radio's
   songs included.

10. **Dragging into *Playing next* puts the song where you drop it.** A
    row of *Playing next* dragged to another place in it moves there; a
    song or selection dragged from a list, the player bar or the sidebar
    goes in at the line between rows, or at the end below the last one.
    Your other songs keep their order. *Next up* is never a drop target: it
    plays from the album or playlist, not from a list you wrote. While
    *Playing next* is empty, drop on the player bar's Queue button, which
    queues at the end like **Add to queue**. If a song starts while you
    hold a row, the move still takes the song you picked up to the place
    you dropped it. The queue a closed session left behind is not the
    player's yet, so before anything plays a drop there just queues.
