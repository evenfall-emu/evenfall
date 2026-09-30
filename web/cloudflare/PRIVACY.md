# Privacy notice for play.evenfall-emu.org

This notice describes what the official hosted Evenfall instance at
<https://play.evenfall-emu.org> does with your data. It is written from the
code in this repository: the player in `web/player/` and the Worker in `web/server/`.

Anyone can deploy their own copy of Evenfall. A self-hosted instance is run by its
operator, who is responsible for its configuration, analytics, providers and
privacy notice. This notice does not describe those instances.

## Summary

- You can play without an account. Games, saves and progress stay in your
  browser unless you sign in and keep cloud backups enabled, or share a
  state.
- Evenfall never uploads BIOS images, and never uploads ROM files except when
  an admin account, designated by the operator, includes one with its own
  [share link](#shared-states).
- Analytics (page views and share counts) are cookieless and contain no
  account, game or gameplay information.
- An optional account stores your email address (and, with Discord sign-in,
  your Discord identity) plus your cloud backups.
- Deleting your account removes the account immediately and cloud backups
  shortly afterwards, as described below.

## Stored only on your device

The player keeps its data in your browser's IndexedDB and local storage for the
play.evenfall-emu.org origin:

- imported ROMs, their SHA-1 hashes and titles, and when you last played;
- in-game (battery) saves, resume checkpoints, quick-save slots and playthrough
  names;
- settings such as volume and the cloud-backup opt-out;
- when signed in, pending cloud uploads and the account ID they belong to.

A service worker caches the application files for offline use. It never caches
API responses, credentials, ROMs or saves.

This data is not sent to the server except for the cloud backups described
below. Clearing site data in your browser deletes it permanently; export saves
you want to keep first. Deleting your account does not touch it.

## Analytics

Production pages served from play.evenfall-emu.org send page views and share
events to PostHog's EU cloud (`eu.i.posthog.com`). Evenfall configures PostHog as
follows:

- A page-view event is sent when you open Home, Player or Library, with
  the view name, the page URL without its query string or fragment, the
  hostname, your browser's user-agent string and PostHog's in-memory session
  and window identifiers. Referrers and all other event properties are removed
  before sending.
- A share event is sent when you create a [share link](#shared-states) and
  when a shared state starts in your browser. It carries only the event's
  name, the hostname, your browser's user-agent string and the session and
  window identifiers: no page URL, link, game or account.
- PostHog runs in cookieless mode with person profiles disabled: it sets no
  cookie and stores no persistent identifier in your browser. PostHog receives
  your IP address with each request, as any web server does, and uses it with
  the hostname and user agent to count visitors without a cookie.
- Autocapture, session recording and feature flags are disabled, and any
  other event is dropped before sending. ROMs, game titles, gameplay and
  account information are never sent.
- Pages opened from a share link send no page view, so a link never reaches
  PostHog.
- Local development and builds served from any other hostname send nothing.

Retention of analytics events follows the PostHog project's settings.

## Accounts (optional)

You can sign in with Discord or with an emailed sign-in link. The account data
is stored in a Cloudflare D1 database.

- **Email sign-in:** your email address. Sign-in links are single-use, expire
  after five minutes and are stored only as a hash. Emails are sent through
  Cloudflare Email Sending.
- **Discord sign-in:** Evenfall requests only the `identify` and `email` scopes and
  requires a verified email address. It stores your Discord user ID, display
  name, avatar URL and email address, and the Discord OAuth tokens encrypted
  with a server secret.
- **Sessions:** a session cookie (`HttpOnly`, `Secure`, `SameSite=Lax`) keeps you
  signed in for up to seven days, extended while you use the site. Each session
  record stores the IP address and user-agent string of the device that created
  it. Signing out deletes the session. Expired session records are not purged
  on a schedule; they are removed when next presented, at sign-out or when the
  account is deleted.
- **Abuse limits:** request counters are keyed by IP address and endpoint, by a
  SHA-256 hash of the email address (sign-in emails) or by account ID (cloud
  uploads). A counter is reset when its time window restarts.

Signing in does not grant access to any other account's data or to catalog
management.

## Cloud backups

Cloud backups are on by default when you are signed in. **Account > Disable
cloud backups** turns them off for that account on the current device.

When enabled, the player uploads, per playthrough:

- the latest resume checkpoint (a compressed emulator snapshot),
- the latest in-game save,
- the latest state in each quick-save slot,

together with the playthrough name, the game's title label, the ROM's SHA-1
hash, capture time and a content digest. Uploads go to a private Cloudflare R2
bucket created in the EU jurisdiction, under a key derived from your account ID.
There are no public URLs to these objects. Only the most recent copy of each
item is kept; there is no history. A registry of your playthroughs (account ID,
ROM hash, playthrough ID and name) is kept in D1 to enforce the per-account
limit of 100 playthroughs. The public game catalog shows, per game, only the
total number of these playthroughs across all accounts, as a measure of
popularity; no account, name or individual playthrough is identified.

Backups are kept until they are replaced or your account is deleted. Deleting
a playthrough on your device does not delete its cloud backup, and there is
currently no in-app control for deleting a single cloud backup.

## Shared states

When you are signed in, **Share state** in the player uploads the current save
state of the game you are playing: a compressed emulator snapshot, which
includes the in-game save. It is sent with the playthrough name, the game's
title label and the ROM's SHA-1 hash, and stored in the same EU-jurisdiction
R2 bucket, outside your account's folder, with your account ID in private
metadata. You get a link that is the only way to reach it.

For at least 24 hours, anyone with the link can download the snapshot
without signing in, together with the playthrough name, the game title (the catalog's name for
known releases, otherwise your label) and the ROM hash. They do not see your
account ID or email. A snapshot contains the game's memory, which can include
names or other text you entered in the game, so share links only with people
you intend to see it.

A storage rule deletes the snapshot once it is 24 hours old, typically
within a day after that, and the link stops working then. Links cannot
currently be revoked early except by deleting your account. Shares count toward your account's upload
limits.

Accounts the operator has made admin can also press **Include the game**,
which uploads the game's ROM with that one link, stored and deleted like the
snapshot. Anyone with the link can then download the ROM. No other account
is offered this or can do it, and no ROM is ever uploaded without that
button.

Opening a share link needs no account, and the shared state is downloaded
without your cookies. If it is for a catalog game and your device has, or you
choose, that game's base ROM, the player downloads the game's public patch
and builds the ROM in your browser, as the Library does. Otherwise, if the
link includes the game, the player downloads that ROM, also without your
cookies, and stores it in your browser like a ROM you imported. Opening a
link never uploads your ROM files. The state is saved in your browser as a new playthrough, like
any other local progress; if you are signed in with cloud backups enabled,
that playthrough is then backed up like your others.

## Deleting your account

**Account > Delete my account** (one confirmation) does the following:

1. Records a deletion marker containing only the opaque account ID and
   timestamps, which immediately blocks new cloud uploads for that account.
2. Deletes the account record, sessions, linked Discord account and tokens,
   pending sign-in links, the playthrough registry, account limits and the
   account- and email-keyed abuse counters.
3. Deletes every cloud object stored under the account.

Shared-state links stop working as soon as step 1 is recorded, and their
snapshots are deleted by the storage rule described above.

Step 3 is attempted immediately and repeated by a scheduled job: every five
minutes during the first hour, then daily. The repeats remove any upload that
was already in flight when you deleted the account. Because of this, cloud
deletion is eventually consistent rather than instantaneous. The deletion
marker is kept indefinitely so these repeats can continue; it contains no email
address, name or game data. A later sign-up with the same email creates a new,
unrelated account ID.

Data stored only on your device is not affected; clear it through your browser.

## Where data is processed

The site, its API, the database, cloud storage and sign-in email all run on
Cloudflare. The cloud-backup bucket uses Cloudflare's EU jurisdiction and the
database has a Western Europe location hint, but Workers requests, email
delivery and Cloudflare's own operational logs are not restricted to the EU.
Discord processes your sign-in if you choose it, and PostHog processes page
views as described above. Each provider also handles data under its own
privacy policy.

The Worker configuration in this repository does not enable Cloudflare Workers
Logs, and Evenfall's server code logs only generic failure messages, never email
addresses, sign-in tokens or save contents.

## Contact

Send privacy questions and requests about your data to
[privacy@evenfall-emu.org](mailto:privacy@evenfall-emu.org).

## Changes

This notice changes with the code; its history is in the repository.
