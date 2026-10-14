# Everyday Use

## Library and search

Home combines recently added albums, recently played albums, top and most
played songs, top artists, and a random shelf. Some personalisation depends on
Navidrome's native API or its Last.fm integration and can be empty without
affecting browsing or playback.

Search covers the songs, albums, artists, and playlists indexed by your
server. Right-click rows and cards to star music, add songs to a playlist, or
put a song in the queue. The playlist submenu filters as you type.
Recent searches keeps one entry for a query typed with pauses: while you go on
typing, the longer or shorter query replaces the one the pause recorded.
Pressing Enter, picking a recent search, emptying the field or typing over the
selected query starts a new entry.
Sorting or filtering a playlist, an album or Liked Songs changes what Play
plays: the songs on screen, in the order shown, including a song that appears
twice. Play is disabled when a filter matches nothing; it never falls back to
the whole list, though it still pauses and resumes the page's songs while
they are what is playing. Clearing the filter restores the original order.
Sorting by album keeps each album's songs together in disc and track order,
and two albums that share a name stay apart.

When the window narrows, Library, Queue and Lyrics give up width before the
top bar runs out of room, and return to the widths you chose once it widens.
With Queue or Lyrics open, the window cannot be made narrower than the room
they need beside the page.

Right-clicking a song in a playlist you can edit offers **Remove from this
playlist** even when the list is sorted or filtered, since removal does not
depend on position; with several songs picked, it removes them all. Drag a
song to the top or bottom edge of the list, or of the sidebar, to scroll it
while you hold the song. **Move
up**, **Move down**, and drag-reorder stay on the default order, where the
rows on screen match the order saved on the server. Right-clicking the
playing song in the player bar offers the same removal while it plays from a
playlist you can edit.

Drag a song row, or the playing song's cover or title in the player bar, onto
a playlist in the sidebar to add it there. Drop it between the rows of an open
playlist you can edit to put a copy at that spot; the line between rows marks
where it goes, the blank space below the last row appends, and an empty
playlist takes its first song the same way. The song stays where it came from
and playback carries on. Clear the playlist's filter and sort first, so the
positions on screen are the saved ones.

Pick several songs with Ctrl-click (Cmd-click on macOS) or Shift-click, then
drag any picked row: the whole selection travels together in the order shown,
however you picked it, and the chip under the pointer names the first song
and counts the rest. Drop it on a sidebar playlist to append, between the rows
of another playlist you can edit to insert, or on Liked Songs to star every
song. Dragging a row that is not picked carries that song alone, and moving
rows within a playlist still moves one song at a time: a selection dropped
back between the rows of its own playlist stays where it was.

## Keyboard input

In a playlist, album or Liked Songs, Up and Down move between songs in the
order shown and select the song they land on, Shift with Up or Down extends
or shrinks the selection, Enter plays the focused song, and Tab reaches the
controls inside a row. Clicking a song's row selects it and gives it the
keyboard. In a playlist you can edit, Delete (or Backspace on macOS) removes
the selected songs.

Ctrl+A (Cmd+A on macOS) selects every song the list shows, following its
filter. Ctrl+C copies the selected songs' links, one a line, and Ctrl+V in a
playlist you can edit adds the songs whose links are on the clipboard. Ctrl+X
copies them and removes them from a playlist you can edit, so Ctrl+V in
another moves them there. A focused text field or an open dialog keeps these
keys.

Right-click a text field for Cut, Copy, Paste and Select all; the password
field offers only Paste and Select all.

Ctrl, Cmd and Alt arrow keys move the caret while a text field has focus.
Playback and navigation shortcuts on those keys remain available from song
rows and other controls.

Choose **Refresh** in a playlist's **…** menu to read it from the server
again, for instance after another app changed it.

## Middle-click autoscroll

On Windows, and on Linux once turned on, middle-click anywhere in a
scrolling list, on a row, a link or a button as well as on its background,
then move the pointer away from the starting point. That list follows the
pointer, faster as the distance grows, and the pointer turns into an arrow
showing which way it is going; a shelf scrolls sideways. Speed and the dead
zone are those of Chromium's autoscroll: each direction has its own small dead
zone, so an ordinary middle-click does not move the view, and a pointer
slightly off the vertical does not drift the list sideways. Moving across
another pane keeps the original list in control.

Hold the middle button while you move and let go to stop. After a plain
click the list keeps following the pointer: click again, press Esc, turn the
wheel, or switch to another window to stop. Text fields keep middle-click
paste, and sliders, scroll bars and panel edges keep their own dragging.

This works automatically on Windows, with no setting to enable. On Linux,
turn on **Middle-click autoscroll** under **Settings > Appearance**. It is
off by default, because Linux desktops usually paste the primary selection on
middle click. macOS keeps its existing middle-click behaviour.

## Radio

**Go to song radio** in a song's menu opens a page of songs your server picks
to go with it, without starting playback. Playlist, album, and artist menus,
including the **…** menu on their pages, have **Go to playlist radio**, **Go
to album radio**, and **Go to artist radio**.

The server picks afresh each time it is asked, so the page keeps the songs it
shows: **Play** and a double-clicked row play those songs, in the order shown,
and the queue names the radio and links back to its page. Choose **Refresh**
in the page's **…** menu for a new mix; the songs stay on the page until it
arrives, and stay if it fails. **Save as playlist** creates a private playlist
named after the radio with the songs on the page.

Subsonic has no radio for a playlist, so a playlist's radio is the songs that
go with a few of its songs, picked at random from different artists, merged
without repeats, up to 50.

The server finds similar songs through an agent, such as Navidrome's Last.fm
integration once it has an API key. Without one, every radio page says there
are no similar songs; nothing is broken. A radio's songs are not saved when
Fastsonic closes, so resuming one plays the last song on its own.

## Sidebar order

Click a playlist, album or artist in the sidebar to open it; double-click it
to play it, as its cover's play button does. On what is already playing, that
button pauses instead, and on what is paused it resumes where it stopped
rather than starting over; the cards on Home, Search, the Library pages and an
artist's page, and the big Play at the top of a page, behave the same.

The button beside the Library heading switches between the list and a grid of
cover cards, which adds columns as the sidebar widens, and the choice is
remembered. A click on a card opens it and its corner button plays it, or
pauses and resumes it while it plays; a double click only opens it. Pins,
search, sorting and dragging work the same in both. With the artwork expanded,
the grid scrolls on under the cover, and a song or card dropped on the cover is
ignored rather than landing on the card hidden beneath it.

The menu under the Library filters chooses an order for each section, and
the choice is remembered. **Name** and **Recently played** are offered
everywhere. Playlists also sort by **Creator**, their owner, and albums by
**Artist** and by **Recently added**, the date you starred them; an album
without a date comes last. Playlists cannot be starred and the Library keeps
no star date for artists, so those sections do not offer it. Albums and
artists start in **Library order**, the order the server lists them in.

By default, playlists are sorted by when you last played them. Drag a
playlist to switch to **Custom order**. New playlists appear below the pinned
group. Choosing another order keeps the arrangement, so choosing **Custom
order** again restores it; so does **Sort by recently played** in a
playlist's context menu. Any order but **Library order** loads the rest of the
section in the background; a page that fails stops that, and choosing the
order again retries it.

**By You**, beside the order under Playlists, shows only the playlists your
account owns, with Liked Songs. Other users' playlists the server shares with
you are hidden, pinned ones too. Search, the chosen order and dragging still
work; a drag while some playlists are hidden keeps them in the custom order.
The choice is remembered, and it needs nothing from the server: every playlist
already names its owner. Turn it off to see the whole shelf again.

Liked Songs starts pinned at the top. Drag it between pins to choose its
place, or below them to unpin it and put it in **Custom order**; other pins
can sit above it. Its right-click menu also offers **Unpin** and **Pin to
top**, which adds it after your other pins. Unpinned, it follows **Name**,
**Creator** and **Recently played** like the playlists, and returning to
**Custom order** puts it back where you left it. The arrangement survives a
restart, and dropping a song on Liked Songs still stars it wherever the row
sits.

## Queue

Songs added with **Add to queue** appear after songs already queued and before
the rest of the current album or playlist. The queue names the playing album,
playlist, artist, radio, or Liked Songs and links back to it. Clear removes
only those manually queued songs. The player owns the queue, so every change is
visible on the next frame and needs no server round-trip. The complete contract is in
[The Queue's Rules](../_reference/queue.md).

Drag a row of **Playing next** to reorder it, or drop a song, a selection or
the playing song between its rows to queue it at that place; the line between
rows shows where it goes, and the list scrolls while you hold a song near its
edge. **Next up** plays from the album or playlist and takes no drops. Drop on
the player bar's Queue button to queue at the end, which is the place to drop
while **Playing next** is empty.

Right-click a row of **Playing next**, or open its **…** menu, and choose
**Remove from queue** to take out that one row; other copies of the song stay.
Rows of **Next up** have no such entry.

Play and Pause fade over 50 ms, and a volume change, mute included, glides
over 30 ms instead of clicking. Pressing the player bar's volume slider sets
the volume where you press, without waiting for the pointer to move; the seek
slider shows where you press and seeks when you let go. Playing another song,
Next, Previous, a queue jump and a seek start the equalizer and its limiter
afresh, so no scrap of the sound before them reaches a quiet intro; a song
that follows on by itself, gaplessly, and Pause and Play keep the music
continuous. The visualizers still follow the post-equalizer, pre-volume
signal, so transport and volume changes do not move the picture.

Settings → Appearance → **Player bar visualizer** offers a spectrum or waveform
behind the controls. It is off by default. Click empty space on the player bar
to cycle through spectrum, waveform, and off; turn off **Click player bar to
change visualizer** just below it if those clicks change it by accident.
**Frame rate** under that sets how often it and the mini player's analyser
draw: 30, 60 (the default), your screen's rate once MilkDrop has seen it, or
Uncapped, which follows the display. A lower rate uses less power. MilkDrop
has a frame rate of its own.

## Less motion

Fastsonic keeps still when the system asks it to: **Animation effects**
turned off in Windows, **Reduce motion** on macOS, or **Animations** turned
off in GNOME. Each animation then shows where it ends at once: hovers and
expanding sections change without easing, a jump to a place in a list lands
there without scrolling, the player bar takes a new song's colour without
fading, the lyrics jump to the sung line and light it without a fade, and
rows make room for a dragged song without sliding. Spinners still turn, and
the visualizers still follow the music.

Windows and macOS are asked again about once a second, so changing the
setting takes effect without a restart. On Linux the desktop portal reports
GNOME's switch as it changes; other desktops that do not publish it keep
Fastsonic's animations.

The visualizers, the player bar's and the Winamp window's alike, draw at half
their frame rate while Fastsonic's window is out of focus or the system saves
energy (Energy Saver or battery saver on Windows, Low Power Mode on macOS, the
power-saver profile on Linux), at most 30 frames a second while it saves
energy, and not at all while nobody can see the window: minimised, closed to
the tray, with the display off or the screen locked.

## macOS Dock menu

Right-click or Control-click Fastsonic's Dock icon for **Play** (or **Pause**
while music plays), **Next**, and **Previous**, above the standard Dock items.
They keep working while the window is closed to the menu bar.

## MacBook notch

On a Mac whose display has a notch, turn on **MacBook notch widget** under
Settings → Playback on this computer. While Fastsonic is in the background,
behind another app, minimized or closed to the menu bar, rest the pointer on
the notch and a card drops out of it with the cover, the song and artist,
the elapsed and total time over a seek bar, and the heart, Shuffle,
Previous, Play/Pause, Next and Repeat. Drag or click the bar to seek; the
song moves there when you let go. Click anywhere else on the card, or on the
notch itself, to bring the window back. Move the pointer away and the card
folds back into the notch.

Clicking the card does not bring Fastsonic to the front, so pausing from the
notch leaves the app you were using where it was. The card stays away while
Fastsonic's window is in front, and while no display has a notch, such as a
MacBook closed on an external monitor. It is off by default.

## Windows taskbar controls

Hovering Fastsonic's taskbar button offers **Previous**, **Play/Pause**, and
**Next** beneath its window preview. They act like the player bar's
buttons, update immediately, and are disabled when nothing is playing. The
icons follow the system appearance and display scaling.

Closing to the tray removes the window and its preview. Reopening the main
window or switching to the Winamp window creates its controls again. Media
keys and the system's now-playing controls keep working while the window is
closed.

While nobody can see the window (minimised, closed to the tray, the display
off or the screen locked), Fastsonic asks Windows to run it in efficiency
mode, on efficient cores at efficient clocks, and hands the choice back as
soon as the window shows again. The thread that feeds the sound card keeps
its priority, so playback is not affected. If you set efficiency mode
yourself in Task Manager, Fastsonic leaves it alone.

## Recent plays

The queue panel's second tab combines the server's recent songs with tracks
played through Fastsonic. Server rows may not include an exact play time;
local rows do.

A song enters local history after about 30 seconds, or halfway through a
shorter song. Paused time and seeking do not count. A song played twice in a
row, on Repeat one for instance, is two plays, each counted once it has been
heard long enough. Fastsonic also scrobbles
playback to your own server so its history and play counts stay current.

The local list is stored in `history.json` and is never uploaded anywhere
except your configured server's scrobble endpoint. Settings → Storage shows
its location and has a **Clear history** button.
