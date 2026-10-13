# How It Connects

Fastsonic has no hosted service or application account. It connects directly
from this computer to the one music server entered on the sign-in screen.

## Sign-in and authentication

For Navidrome, sign-in is one `POST /auth/login` request containing the server
username and password. The response contains two different credentials:

- A Subsonic salt and token pair used for `/rest/*.view` calls. The token is
  MD5 of the password and salt because that is the protocol's wire format.
  Fastsonic stores this pair and never stores the password. The pair is still
  password-equivalent for that server and must be kept private.
- A Navidrome JWT used only by the small native-API module. It supplies
  personalisation that Subsonic cannot, and normally expires after 24 hours.
  Navidrome administrators can change that period with `ND_SESSIONTIMEOUT`.

The salted token continues to provide library browsing and playback after the
JWT expires. Personalised Home sections then appear empty until the next
password sign-in. On a compatible non-Navidrome server, the same core features
can work while Navidrome-only sections remain empty.

An empty password field retries the stored Subsonic credential, which is
useful after a server was temporarily unreachable. Signing out removes both
stored credentials.

Every Subsonic request carries the username, salt, and token. Fastsonic strips
credential query parameters and authorization headers from logs, from the
network errors it logs and shows, and from the panic log. Artwork is
cached under an opaque `sonic:art:` key rather than a credential-bearing URL.
The desktop's media controls get the playing song's cover as a file from that
cache, never a URL: when a song starts, its full-size cover is downloaded once
if nothing on screen has fetched it yet, and Linux MPRIS hands the desktop a
`file://` path.

## Server requests

Library, search, playlists, stars, artwork, lyrics supplied by the server,
streams, and scrobbles use Subsonic/OpenSubsonic. Playlist edits use form POST
when necessary. Subsonic can only append to a playlist or remove rows from it,
so a move, or a song dropped between rows, reads the playlist again and
rewrites it whole in the new order under the same id and name. While an
edit is on its way, Fastsonic asks for none of the playlist's rows and ignores
any that arrive, so a read from before the edit cannot undo it on screen;
**Refresh** waits too, and the last edit's answer reloads the rows in place. Playback requests the original file, without transcoding, so
HTTP byte ranges remain available for seeking and the in-process decoder sees
the library's real format.

A radio page asks the server for similar songs: `getSimilarSongs` for a song
or an album, `getSimilarSongs2` for an artist, each beside `getSong`,
`getAlbum` or `getArtist` for the name and cover it is titled with. Subsonic
takes no playlist, so a playlist's radio reads the playlist with `getPlaylist`
and sends `getSimilarSongs` for up to five of its songs. **Refresh** repeats
the same requests. The server answers from its own agents, such as Last.fm,
which it may contact itself; Fastsonic talks only to the server, and without
an agent the answers are empty.

Opening a playlist, album or artist keeps the title, byline and cover that the
sidebar, Home, Search or another page already holds for it while `getPlaylist`,
`getAlbum` or `getArtist` answers; this adds no request. Controls that need the
answer stay disabled under a loading row, and if the request fails the known
header stays above **Retry**. A link to something no list has shown has only
the loading or error row.

Each cover is requested from `getCoverArt` at about a quarter more pixels
than it covers on the window's display, rounded up to one of 64, 128, 192,
300, 448, 640, 960 or 1280 pixels, so a panel being dragged wider does not
ask for a new size at every pixel, nor fill the server's resize cache and
the artwork cache with near copies. The 64-pixel size serves only covers of
32 pixels or less. At 100% scaling a sidebar row asks for 128, a card for
192 and a page header for 300; at 200% a card asks for 448 and a header for
640. The colour a page is tinted with and the softened stand-in for a page's
cover use the 64-pixel size, and the media controls the 640-pixel one. Each
size is downloaded once and cached under its own request, so artwork cached
in those three sizes by earlier versions is still read. Fastsonic then
scales each cover on this computer to the pixels it covers before drawing
it.

While a cover at a new size is on its way, after a panel is resized or the
window moves to another display, the nearest size of the same cover already
drawn stands in. While a page header's cover is on its way, the card cover
of the same item stands in if it is already drawn, and otherwise the
64-pixel thumbnail, enlarged and softened. The player bar, the sidebar's
large artwork and the lyrics keep the last song's cover for up to two
seconds while the next one's arrives. A ready cover stays until its
replacement is ready, and a page whose answer has no cover shows its
placeholder rather than an old image. Hovering a sidebar row works out the
page's tint from its thumbnail before the page opens. An artist image the
server offers as its own URL is fetched as is, and only scaled here.

Sorting the Library's albums or artists by anything but the server's own
order loads the rest of that section, one page after another through the same
calls that list it, while the loaded entries stay visible. A failed page stops
the loading. Album star dates come with those pages; sorting adds no request
of its own, and every order is applied in the client.

The audio engine runs outside the UI thread. It reads the HTTP stream through
a bounded on-disk block cache, decodes and resamples it, applies ReplayGain and
the equalizer, sends post-EQ/pre-volume samples to the visualisers, then sends
the volume-adjusted signal to the selected local output device. The queue is
owned by this engine and does not exist on the server. When no device is
selected, playback follows the system's default output, so connecting
headphones moves the music to them. A device that goes away, or a stream that
stops playing for two seconds without reporting an error (time the computer
spent asleep does not count), is opened again and the song carries on from
where it was heard, converted to the new device's rate; only a device that
will not open stops playback with an error. On Linux, when ALSA
plays through PulseAudio's plugin, including PipeWire's PulseAudio server, the
stream is named **Fastsonic** with **Music playback** as its description, so
mixers and audio processors can identify and route it; explicit
`PULSE_PROP_application.name` and `PULSE_PROP_stream.description` values take
precedence. PipeWire's own ALSA plugin names the stream after the program.

On macOS, playback declares a user-initiated activity while a song plays or
loads, so App Nap does not slow the engine down when another application
covers Fastsonic. Pausing, stopping, an output error or signing out releases
it. The activity permits idle system sleep and does not keep the display
awake.

## Other outbound traffic

- When the lyrics panel is open and the server has no lyrics, Fastsonic sends
  the track's artist, title, album, and duration to
  [LRCLIB](https://lrclib.net). Lyrics are cached locally for a month.
- If update checks are enabled, Fastsonic asks GitHub's API once a day whether
  a newer release exists. **Check for updates** asks the same question once,
  whether or not the daily check is on.
- Settings can download optional MilkDrop preset packs from their documented
  upstream sources.

There is no telemetry, analytics, inbound listener, receiver discovery,
peer-to-peer traffic, or server-side playback control. TLS certificates use
the operating system's trust decisions; there is currently no switch to allow
a self-signed certificate.
