---
title: Make It Faster
description: Why pages sometimes take a while to fill, and the five-minute fix: a Spotify app of your own, with an allowance nobody else shares.
nav_order: 4
---

## Why the app waits sometimes

Everything Fastpotify shows you, from your playlists to search results,
comes from Spotify's Web API, and Spotify meters that API per
*application*, not per person. Out of the box Fastpotify uses a shared
public application, the same one spotify-player, ncspot, and Omarchy
Spotify use, so its allowance is divided among everyone running any of
them. When that runs out, Spotify answers "come back later" and the app
waits, shows a spinner in the top bar, and retries. You see it as pages
that take a while to fill.

Fastpotify cannot simply ship an application of its own instead. Since
2026, Spotify keeps a new application in "development mode", which serves
at most five people and needs its owner to have Premium, and it grants
wider access only to registered businesses with hundreds of thousands of
users. So the way to an allowance that is yours alone is an application
that is yours alone. It is free, and it takes about five minutes.

## Make the application

1. Open the [Spotify developer dashboard](https://developer.spotify.com/dashboard)
   and sign in with your Spotify account (it needs Premium).
2. Choose **Create app**. Name and describe it however you like; they are
   only shown to you.
3. Under **Redirect URIs**, add exactly:

   ```
   http://127.0.0.1:8989/login
   ```

   That is the address Fastpotify listens on for Spotify's answer when you
   sign in.
4. Under **Which API/SDKs are you planning to use?**, tick **Web API**.
5. Accept the terms and save. The app's page shows its **Client ID**, a
   string of 32 letters and digits. Copy it.

## Tell Fastpotify

1. In Fastpotify, open **Settings** and find **Your own Spotify app**.
2. Paste the Client ID into the field.
3. Sign out from the account menu at the top right, then sign in again.
   The browser opens Spotify's consent page for *your* application this
   time.

That is all. Playback on this computer is unaffected: it uses a separate
grant against Spotify's own streaming identity ([why](/how-it-connects/)).

## What changes

Nothing you can see, except that the app stops waiting. The Client ID is
not a secret, but it is tied to your account, so keep it to yourself as
you would a username. To go back to the shared application, clear the
field and sign in again.
