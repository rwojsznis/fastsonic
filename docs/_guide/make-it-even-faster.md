---
title: Use Your Own Spotify App
description: Use a personal Spotify application to avoid the shared Web API rate limit.
nav_order: 4
---

## API rate limits

Fastpotify loads library and catalogue data through Spotify's Web API, which
is rate-limited per *app*. By default, Fastpotify shares a public app with
several other open-source players. When that app reaches its limit, requests
are delayed and the top bar shows a spinner.

A personal app has a separate limit. Fastpotify cannot provide a dedicated
app for each user because Spotify restricts how many users a new app can
have, but you can create one at no cost.

## Limitations

Spotify keeps a personal app in Development Mode, and since February 2026
that mode reads only the playlists you own or collaborate on. Anyone
else's public playlist, and Spotify's own editorial playlists, show their
name and cover but not their songs. Artist top tracks and browse data are
also unavailable. The shared app does not have these restrictions. To switch
back, clear the field and press **Switch now**.

## Make a Spotify app

1. Open the [Spotify developer dashboard](https://developer.spotify.com/dashboard)
   and sign in with your Spotify account. Spotify asks that it be a
   Premium account.
2. Click **Create app**. Any name and description will do; nobody else
   sees them.
3. Under **Redirect URIs**, add exactly:

   ```
   http://127.0.0.1:8989/login
   ```

4. Tick **Web API**, accept the terms, and save.
5. The app's page shows its **Client ID**. Copy it.

![Settings, with a personal Spotify app in use](/assets/images/make-it-even-faster.png)

## Use it in Fastpotify

1. Open **Settings**, find **Make it even faster**, and paste the
   Client ID.
2. Click **Switch now**. Your browser opens Spotify's sign-in for your
   app; approve it and you are back in Fastpotify, which now says
   **Your app is in use**.

Local playback uses separate credentials and is unaffected. To return to the
shared app, clear the field and click **Switch now** again.
