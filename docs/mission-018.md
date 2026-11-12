# WiniOS iPhone setup

Install just two standalone Shortcuts: [WiniOS Send](https://www.icloud.com/shortcuts/dd53de7c69464ee2bc45e90401036391) and [WiniOS Receive](https://www.icloud.com/shortcuts/039fa0677d764e88a399e16c3e614f30).

1. Install Tailscale on Windows and iPhone. Sign in to the appropriate tailnet
   and connect both devices. Keep your laptop awake and WiniOS running.
2. Open WiniOS from the tray, choose **Open Settings → Setup & troubleshooting**,
   and follow Setup to **iPhone Setup**. Use **Test connection**. Copy your
   **WiniOS HTTPS URL** with **Copy HTTPS URL**: use the private HTTPS address
   without a trailing slash. It is the base address, not the clipboard API path.
   Use **Copy WiniOS Token** and confirm the warning to copy your private token.
3. Open each link above in Safari on iPhone and add the Shortcut. Inspect the
   actions and destinations before entering your token. The action graphs and
   availability of the two links could not be verified from this computer.
4. For each Shortcut's **WiniOS HTTPS URL** question, enter the base URL. For
   **WiniOS Token**, enter your private token. Keep it out of shared links,
   screenshots and messages. If either question is missing, stop and review the
   Shortcut before entering a token; do not modify headers by guesswork.
5. With sharing resumed, copy disposable text on Windows, run Receive on iPhone
   and paste. Copy disposable text on iPhone, run Send and paste on Windows.
   Repeat with a disposable screenshot/photo. Send requires clipboard writes
   enabled in WiniOS Settings → Clipboard; images require Settings → Images and
   Apply & restart. A connection test on Windows does not prove iPhone transfer.

If a request fails, ensure Tailscale is connected on both devices, WiniOS is
Connected and sharing is resumed. For a certificate warning, stop and verify the
HTTPS identity; never bypass verification. Existing configured Shortcuts are not
modified automatically. No helper Shortcuts or manual header edits are required
by the intended two-Shortcut flow.

The copied token is sensitive. WiniOS never embeds it in the public links; other
local apps may read it from the clipboard after explicit copy. Copy disposable
text on Windows when finished. Do not publish an imported, configured Shortcut.
