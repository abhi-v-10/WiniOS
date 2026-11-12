# Optional update notifications

Configure a trusted public GitHub repository as described in [Mission 013](mission-013.md).
For the official WiniOS repository, use `abhi-v-10/WiniOS` as the
`github_repository` value. GitHub must make the repository and its releases
public for this credential-free checker to work; a 404 can mean that no stable
release exists, or that the repository is unavailable or private. The configured
state verifies the local source format, not its public availability.

In **Settings → About**, enable
**Automatically check for updates**. The control is unavailable until a valid
source is configured; reopen Settings after changing the source. Automatic
checks are off by default, including when a source exists.

When enabled, WiniOS checks on startup if at least 24 hours have passed since the
previous automatic attempt, then at most once per 24 hours while running. A failed
or offline attempt counts toward that interval. Manual checks remain available at
any time. Network activity runs on a background worker, and errors during automatic
checks are silent. On a newer stable release, About shows its version and release
notes; choose **View Update** to open the validated official release page. WiniOS
does not download or install an update. Windows notification settings or Focus may
suppress the native notification.

The local `update-notifications.json` beside `settings.json` stores the opt-in,
last automatic attempt and versions for which a notification was requested. It
contains no clipboard content, credentials or release notes. The file is created
only when the user changes the setting or an enabled check runs. A version is
recorded before requesting a notification to avoid repeat popups across restarts;
if the 256-version history fills, future notifications are suppressed rather
than repeated. Disable the setting to stop automatic requests. Removing the
trusted source also stops them; manual checks report the unconfigured state.

Checks use the existing HTTPS-validated GitHub transport, refuse redirects and
do not send the WiniOS token or Windows credentials. GitHub receives the ordinary
network connection information. Independently verify the release publisher and
installer before downloading anything. Changing this setting does not alter
Tailscale, clipboard settings, certificates or the installed application.
