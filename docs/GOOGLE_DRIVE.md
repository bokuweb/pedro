# Reading a PDF out of Google Drive

pedro can take a book from Drive instead of from the disk. Paste a link, and
the file arrives in the library as though it had been dragged in — same content
hash, same highlights if it was already there, same everything after that.

This is the one part of pedro that talks to a remote service, and the only part
that needs setting up before it works. Google will not let an application read
someone's Drive without knowing which application is asking, so you have to
create an OAuth client and tell pedro what it is.

## What it does

- A **link** is anything Drive gives you: `/file/d/{id}/view`, an older
  `?id=`, a `docs.google.com/document/d/{id}/edit`, or a bare file id.
- A **PDF** is downloaded as it is. A **Google Doc**, **Sheet** or **Slides**
  deck is exported as a PDF on the way out. Anything else is refused, with what
  it actually was.
- The **sign-in happens once**. A refresh token goes into the operating
  system's keychain — Keychain Services on macOS, the Credential Manager on
  Windows, the Secret Service on Linux — and every fetch after the first one is
  a single request with no browser in it.

## Setting up the OAuth client

In the [Google Cloud console](https://console.cloud.google.com/):

1. **Create a project**, or pick one you already have.
2. **Enable the Google Drive API** — APIs & Services → Library → Google Drive
   API → Enable.
3. **Configure the OAuth consent screen** — External, unless you are on Google
   Workspace and would rather it be Internal. Add
   `https://www.googleapis.com/auth/drive.readonly` as a scope, and add your
   own Google account under **Test users**.
4. **Create credentials** — Credentials → Create credentials → OAuth client ID
   → **Desktop app**. Copy the client id and client secret.

Then tell pedro:

```bash
export PEDRO_GOOGLE_CLIENT_ID='…apps.googleusercontent.com'
export PEDRO_GOOGLE_CLIENT_SECRET='…'
```

The secret is not really a secret — Google says so itself, because a desktop
client ships it inside every copy — but it is per-installation configuration
rather than something to commit, which is why it is read from the environment
rather than compiled in. What actually protects the exchange is PKCE, and pedro
always uses it.

## Using it

Press the link button beside the plus in the sidebar header, paste a Drive
link, and press ⏎. The first time, a browser opens and asks you to sign in;
after that it does not.

## The seven-day catch

While the consent screen is still in **Testing**, Google expires every refresh
token **seven days** after it is issued. pedro handles it — an expired token is
noticed, forgotten, and replaced by opening the browser again — but it does
mean a sign-in roughly once a week.

Setting the publishing status to **In production** stops that. For an
application only you sign in to, that is all it takes. Distributing pedro to
other people with this scope is a different matter: `drive.readonly` is a
*restricted* scope, and a public application that asks for it needs Google's
verification and an annual third-party security assessment.

The narrower `drive.file` scope needs neither, but it only reaches files chosen
through Google's own picker, which is a web component — it would take a browser
embedded in the application to show it. Pasting a link is what pedro has
instead, and a link can name any file in a Drive, so the scope has to be able
to as well.

## Where it lives

`crates/pedro-drive`, and nowhere else. `pedro-core` has no idea a book can come
from anywhere but a path, which is the point: fetching produces a file, and
adding a file to the library is what `Store::add_document` already did.

To sign out, remove the `pedro` / `google-drive` entry from your keychain, or
call `pedro_drive::forget()`.
