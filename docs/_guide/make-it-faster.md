---
title: Make It Faster
description: If pages take a while to fill, a Spotify app of your own fixes it in five minutes.
nav_order: 4
---

## Why it waits

Everything Fastpotify shows you comes from Spotify's Web API, and Spotify
lets each *app* make only so many requests. Out of the box Fastpotify uses
a public app it shares with other open-source players, so at busy times
its requests queue behind everyone else's. That is the spinner in the top
bar, and pages that take a while to fill.

An app of your own has that limit to itself. Fastpotify cannot ship one
for everyone (Spotify allows a new app only a handful of users), but
making yours is free and takes five minutes.

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

## Use it in Fastpotify

1. Open **Settings**, find **Use your own Spotify app**, and paste the
   Client ID.
2. Sign out from the account menu at the top right, then sign in again.

That is all. Playing music on this computer is unaffected. To go back to
the shared app, clear the field and sign in again.
